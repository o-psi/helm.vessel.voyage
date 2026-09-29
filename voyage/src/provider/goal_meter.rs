//! One process-local meter shared by a Goal turn and all its local children.
//! Counts come from provider observations, never token estimates or model claims.
use super::*;
use futures_util::StreamExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct Totals {
    input: u64,
    output: u64,
    in_flight: u32,
    uncertain: bool,
}

/// Runtime-only state. It cannot be constructed from serialized configuration.
#[derive(Debug)]
pub struct GoalMeter {
    totals: Mutex<Totals>,
    token_allowance: u64,
    time_allowance: Duration,
    started: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Measurement {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    pub complete: bool,
}

impl GoalMeter {
    pub(crate) fn new(token_allowance: u64, time_allowance: Duration) -> Arc<Self> {
        Arc::new(Self {
            totals: Mutex::new(Totals::default()),
            token_allowance,
            time_allowance,
            started: Instant::now(),
        })
    }

    pub(crate) fn measurement(&self) -> Measurement {
        let elapsed_ms = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        match self.totals.lock() {
            Ok(t) => Measurement {
                input_tokens: t.input,
                output_tokens: t.output,
                elapsed_ms,
                complete: !t.uncertain && t.in_flight == 0,
            },
            Err(_) => Measurement {
                input_tokens: 0,
                output_tokens: 0,
                elapsed_ms,
                complete: false,
            },
        }
    }

    fn begin(self: &Arc<Self>, request: &mut ModelRequest) -> Result<Attempt, ProviderError> {
        let mut t = self.totals.lock().map_err(|_| refused())?;
        let used = t.input.checked_add(t.output).ok_or_else(refused)?;
        if t.uncertain
            || used >= self.token_allowance
            || self.started.elapsed() >= self.time_allowance
        {
            return Err(refused());
        }
        // This caps requested output. Input size and provider billing are known
        // only after dispatch; this is an observed-usage stop, not a billing cap.
        let remaining = (self.token_allowance - used).min(u64::from(u32::MAX)) as u32;
        request.max_tokens = Some(request.max_tokens.unwrap_or(remaining).min(remaining));
        t.in_flight = t.in_flight.checked_add(1).ok_or_else(refused)?;
        Ok(Attempt {
            meter: self.clone(),
            input: None,
            output: None,
            finished: false,
        })
    }
}

fn refused() -> ProviderError {
    ProviderError::Code {
        source: Box::new(ProviderError::Request(
            "Goal usage or time budget does not permit another provider request".into(),
        )),
        code: "goal_budget_stopped",
    }
}

struct Attempt {
    meter: Arc<GoalMeter>,
    input: Option<u64>,
    output: Option<u64>,
    finished: bool,
}

impl Attempt {
    fn fail(&mut self) {
        if !self.finished
            && let Ok(mut t) = self.meter.totals.lock()
        {
            t.in_flight = t.in_flight.saturating_sub(1);
            t.uncertain = true;
            self.finished = true;
        }
    }
    fn observe(&mut self, report: ReportedUsage) -> Result<(), ProviderError> {
        let mut t = self.meter.totals.lock().map_err(|_| refused())?;
        fn accumulate(total: &mut u64, previous: &mut Option<u64>, next: Option<u64>) -> bool {
            let Some(next) = next else {
                return true;
            };
            if next < previous.unwrap_or(0) {
                return false;
            }
            let Some(sum) = total.checked_add(next - previous.unwrap_or(0)) else {
                return false;
            };
            *total = sum;
            *previous = Some(next);
            true
        }
        let input_valid = accumulate(&mut t.input, &mut self.input, report.input_tokens);
        let output_valid = accumulate(&mut t.output, &mut self.output, report.output_tokens);
        if !input_valid || !output_valid {
            t.uncertain = true;
            return Err(ProviderError::InvalidResponse(
                "Goal provider usage counters are inconsistent".into(),
            ));
        }
        Ok(())
    }

    fn finish(&mut self, response: &ModelResponse) -> Result<(), ProviderError> {
        let consistent = self.input == Some(response.usage.input_tokens)
            && self.output == Some(response.usage.output_tokens);
        if !consistent {
            // Preserve every known lower bound even when the adapter omits its
            // explicit presence metadata or disagrees with its final response.
            let input = self.input.unwrap_or(0).max(response.usage.input_tokens);
            let output = self.output.unwrap_or(0).max(response.usage.output_tokens);
            self.observe(ReportedUsage {
                input_tokens: Some(input),
                output_tokens: Some(output),
            })?;
        }
        let mut t = self.meter.totals.lock().map_err(|_| refused())?;
        t.uncertain |= !consistent;
        t.in_flight = t.in_flight.checked_sub(1).ok_or_else(refused)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.fail();
    }
}

pub(super) struct MeteredProvider {
    pub inner: Box<dyn Provider>,
    pub meter: Arc<GoalMeter>,
}

#[async_trait]
impl Provider for MeteredProvider {
    fn context_window(&self, model: &str) -> Option<usize> {
        self.inner.context_window(model)
    }
    fn supports_steering(&self) -> bool {
        self.inner.supports_steering()
    }
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.models().await
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let mut stream = self.stream(request).await?;
        while let Some(event) = stream.next().await {
            if let ProviderStreamEvent::Completed(response) = event? {
                return Ok(*response);
            }
        }
        Err(ProviderError::StreamInterrupted)
    }
    async fn stream(&self, mut request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        // Keep the guard outside the lazy stream body: dropping the returned
        // stream without polling still marks the dispatched request uncertain.
        let mut attempt = self.meter.begin(&mut request)?;
        let mut source = self.inner.stream(request).await?;
        Ok(Box::pin(async_stream::try_stream! {
            while let Some(event) = source.next().await {
                if event.is_err() { attempt.fail(); }
                let event = event?;
                match &event {
                    ProviderStreamEvent::UsageReported(usage) => attempt.observe(*usage)?,
                    ProviderStreamEvent::Completed(response) => {
                        attempt.finish(response)?;
                        yield event;
                        return;
                    }
                    _ => {}
                }
                yield event;
            }
            attempt.fail();
            Err(ProviderError::StreamInterrupted)?;
        }))
    }
}

#[cfg(test)]
mod tests;
