//! Strict bounded SSE decoding for durable Vessel invalidations.
use anyhow::Result;
use futures_util::{StreamExt, stream::BoxStream};
use std::time::Duration;
use voyage_protocol::process::{MAX_PROCESS_FRAME, PROCESS_PROTOCOL, VesselEvent};

pub(super) fn decode(response: reqwest::Response) -> BoxStream<'static, Result<VesselEvent>> {
    Box::pin(async_stream::try_stream! {
        if !response.status().is_success() {
            Err(anyhow::anyhow!(
                "Vessel event stream rejected (HTTP {})",
                response.status().as_u16()
            ))?;
        }
        check(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(';').next() == Some("text/event-stream")),
            "Vessel returned a non-SSE event response",
        )?;
        let mut body = response.bytes_stream();
        let mut pending = Vec::new();
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(15), body.next())
                .await
                .map_err(|_| anyhow::anyhow!("Vessel event stream stalled"))?;
            let chunk = match chunk {
                Some(chunk) => chunk,
                None => break,
            };
            let chunk = chunk.map_err(|_| anyhow::anyhow!("Vessel event stream interrupted"))?;
            check(
                pending.len().saturating_add(chunk.len()) <= MAX_PROCESS_FRAME,
                "Vessel SSE event exceeds frame limit",
            )?;
            pending.extend_from_slice(&chunk);
            while let Some((end, delimiter)) = frame_end(&pending) {
                let frame = pending.drain(..end + delimiter).collect::<Vec<_>>();
                let text = std::str::from_utf8(&frame[..end])
                    .map_err(|_| anyhow::anyhow!("Vessel SSE is not UTF-8"))?;
                let mut kind = None;
                let mut data = String::new();
                for line in text.lines() {
                    if let Some(value) = line.strip_prefix("event:") {
                        kind = Some(value.trim());
                    } else if let Some(value) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(value.strip_prefix(' ').unwrap_or(value));
                    }
                }
                if data.is_empty() {
                    continue;
                }
                check(kind == Some("update"), "unsupported Vessel SSE event")?;
                let event: VesselEvent = serde_json::from_str(&data)
                    .map_err(|_| anyhow::anyhow!("invalid Vessel SSE event"))?;
                check(event.protocol == PROCESS_PROTOCOL, "unsupported Vessel event protocol")?;
                yield event;
            }
        }
        check(pending.is_empty(), "Vessel event stream ended mid-frame")?;
    })
}

fn check(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(anyhow::anyhow!(message))
    }
}

fn frame_end(bytes: &[u8]) -> Option<(usize, usize)> {
    let lf = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    let crlf = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(found), None) | (None, Some(found)) => Some(found),
        (None, None) => None,
    }
}
