//! One attended input owner, released before execution can ask approval/questions.
use super::{Parameter, safe_text};
use crate::terminal_input::Terminal;
use anyhow::{Result, bail, ensure};
use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum InputFailure {
    #[error("workflow input cancelled")]
    Cancelled,
    #[error("workflow input timed out")]
    TimedOut,
    #[error("workflow terminal restoration failed")]
    Restoration,
}

pub(super) struct Field {
    pub name: String,
    pub parameter: Parameter,
}
#[derive(Default)]
struct State {
    terminal: Option<Terminal>,
    cancelled: bool,
}
#[derive(Clone, Default)]
struct Control(Arc<Mutex<State>>);
impl Control {
    fn start(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("workflow input unavailable"))?;
        ensure!(!state.cancelled, "workflow input cancelled");
        state.terminal = Some(
            Terminal::enter()
                .map_err(|_| anyhow::anyhow!("workflow terminal input unavailable"))?,
        );
        Ok(())
    }
    fn finish(&self, cancel: bool) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("workflow input unavailable"))?;
        state.cancelled |= cancel;
        if let Some(terminal) = state.terminal.as_mut() {
            let discarded = terminal.discard();
            let restored = terminal.restore();
            if restored.is_ok() {
                state.terminal = None;
            }
            discarded
                .and(restored)
                .map_err(|_| anyhow::anyhow!("workflow terminal restoration failed"))?;
        }
        Ok(())
    }
    fn discard(&self) -> Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("workflow input unavailable"))?;
        state
            .terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("workflow input cancelled"))?
            .discard()
            .map_err(|_| anyhow::anyhow!("workflow input unavailable"))
    }
    fn read(&self) -> Result<Option<(u16, u16)>> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("workflow input unavailable"))?;
        ensure!(!state.cancelled, "workflow input cancelled");
        state
            .terminal
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("workflow input cancelled"))?
            .read()
            .map_err(|_| anyhow::anyhow!("workflow terminal input unavailable"))
    }
}
struct Restore(Control);
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = self.0.finish(false);
    }
}

struct CancelOnDrop(Control);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let _ = self.0.finish(true);
    }
}

pub struct InputMonitor {
    cancellation: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl InputMonitor {
    pub fn cancellation(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation.clone()
    }
}
impl Drop for InputMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub(super) struct Collected {
    pub values: Vec<(String, Zeroizing<String>)>,
    pub monitor: InputMonitor,
}

pub(super) async fn collect(fields: Vec<Field>, timeout: Duration) -> Result<Collected> {
    ensure!(
        io::stdin().is_terminal() && io::stderr().is_terminal(),
        "--prompt-missing requires terminal stdin and stderr; use --input and --secret-env for noninteractive workflows"
    );
    let control = Control::default();
    // Caller cancellation must forbid late entry even before the worker starts.
    let _caller = CancelOnDrop(control.clone());
    let worker = control.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    // Register signals before raw-mode entry. A detached thread cannot hold Tokio
    // shutdown hostage if a terminal output write is blocked during cancellation.
    let mut interrupt = Interrupt::new()?;
    std::thread::Builder::new()
        .name("helm-workflow-input".into())
        .spawn(move || {
            let _restore = Restore(worker.clone());
            let result = read_fields(&worker, fields, timeout);
            let result = worker.finish(false).and(result);
            let _ = send.send(result);
        })
        .map_err(|_| anyhow::anyhow!("workflow input unavailable"))?;
    tokio::select! { biased;
        _ = interrupt.wait() => {
            control.finish(true).map_err(|_| InputFailure::Restoration)?;
            bail!(InputFailure::Cancelled);
        }
        _ = tokio::time::sleep(timeout) => {
            control.finish(true).map_err(|_| InputFailure::Restoration)?;
            bail!(InputFailure::TimedOut);
        }
        result = receive => {
            let values = result.map_err(|_| anyhow::anyhow!("workflow input reader stopped"))??;
            // Tokio signal handlers persist after registration. Retain the receiver
            // through execution so TERM/HUP cannot become silently ignored.
            let cancellation = tokio_util::sync::CancellationToken::new();
            let signal = cancellation.clone();
            let task = tokio::spawn(async move { interrupt.wait().await; signal.cancel(); });
            Ok(Collected { values, monitor: InputMonitor { cancellation, task } })
        },
    }
}

fn write(text: &str) -> Result<()> {
    io::stderr()
        .write_all(text.as_bytes())
        .and_then(|_| io::stderr().flush())
        .map_err(|_| anyhow::anyhow!("workflow prompt output unavailable"))
}
fn read_fields(
    control: &Control,
    fields: Vec<Field>,
    timeout: Duration,
) -> Result<Vec<(String, Zeroizing<String>)>> {
    control.start()?;
    let deadline = Instant::now() + timeout;
    let mut values = Vec::new();
    for field in fields {
        let visibility = if field.parameter.secret {
            "secret, hidden"
        } else {
            "public, model-visible; recorded when run is saved"
        };
        let description = safe_text(&field.parameter.description);
        write(&format!(
            "\r\n{} ({visibility}, {:?})\r\n{}\r\n",
            field.name, field.parameter.kind, description
        ))?;
        if !field.parameter.choices.is_empty() {
            write(&format!(
                "Choices: {}\r\n",
                serde_json::to_string(&field.parameter.choices)?
            ))?;
        }
        let mut accepted = None;
        for attempt in 0..3 {
            write(&format!("{}: ", field.name))?;
            let value = read_value(control, field.parameter.secret, deadline)?;
            control.discard()?;
            write("\r\n")?;
            let valid = if field.parameter.secret
                && matches!(field.parameter.kind, super::ParameterType::String)
            {
                field
                    .parameter
                    .max_length
                    .is_none_or(|maximum| value.len() <= maximum)
            } else {
                field.parameter.decode(&value).is_ok()
            };
            if !value.contains('\0') && valid {
                accepted = Some(value);
                break;
            }
            if attempt < 2 {
                write("Input does not match the declared type, bounds or choices; try again.\r\n")?;
            }
        }
        let value = accepted.ok_or_else(|| {
            anyhow::anyhow!("workflow input failed validation after three attempts")
        })?;
        values.push((field.name, value));
    }
    Ok(values)
}
fn read_value(control: &Control, hidden: bool, deadline: Instant) -> Result<Zeroizing<String>> {
    let mut value = Zeroizing::new(String::new());
    let mut decoder = Decoder::default();
    loop {
        ensure!(Instant::now() < deadline, InputFailure::TimedOut);
        let Some((unit, repeats)) = control.read()? else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        for _ in 0..repeats {
            match unit {
                3 | 4 | 27 => bail!(InputFailure::Cancelled),
                10 | 13 => {
                    ensure!(decoder.is_empty(), "workflow input is not valid Unicode");
                    return Ok(value);
                }
                8 | 127 => {
                    if !decoder.is_empty() {
                        decoder = Decoder::default();
                    } else if let Some(character) = value.pop()
                        && !hidden
                    {
                        use unicode_width::UnicodeWidthChar;
                        write(&"\u{8} \u{8}".repeat(character.width().unwrap_or(0)))?;
                    }
                }
                _ => {
                    if let Some(character) = decoder.push(unit, cfg!(windows))? {
                        ensure!(
                            !character.is_control(),
                            "workflow input contains a control character"
                        );
                        ensure!(
                            value.len() + character.len_utf8() <= 8192,
                            "workflow input exceeds limit"
                        );
                        value.push(character);
                        if !hidden {
                            write(character.encode_utf8(&mut [0; 4]))?;
                        }
                    }
                }
            }
        }
    }
}
#[derive(Default)]
struct Decoder {
    bytes: Zeroizing<Vec<u8>>,
    high: Option<u16>,
}
impl Decoder {
    fn is_empty(&self) -> bool {
        self.bytes.is_empty() && self.high.is_none()
    }
    fn push(&mut self, unit: u16, utf16: bool) -> Result<Option<char>> {
        if utf16 {
            if let Some(high) = self.high.take() {
                ensure!(
                    (0xdc00..=0xdfff).contains(&unit),
                    "workflow input is not valid Unicode"
                );
                let value = 0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00;
                return Ok(char::from_u32(value));
            }
            if (0xd800..=0xdbff).contains(&unit) {
                self.high = Some(unit);
                return Ok(None);
            }
            return char::from_u32(u32::from(unit))
                .map(Some)
                .ok_or_else(|| anyhow::anyhow!("workflow input is not valid Unicode"));
        }
        self.bytes.push(
            u8::try_from(unit)
                .map_err(|_| anyhow::anyhow!("workflow input is not valid Unicode"))?,
        );
        match std::str::from_utf8(&self.bytes) {
            Ok(text) => {
                let character = text.chars().next();
                self.bytes.clear();
                Ok(character)
            }
            Err(error) if error.error_len().is_none() && self.bytes.len() < 4 => Ok(None),
            Err(_) => bail!("workflow input is not valid Unicode"),
        }
    }
}
struct Interrupt {
    #[cfg(windows)]
    int: tokio::signal::windows::CtrlC,
    #[cfg(windows)]
    brk: tokio::signal::windows::CtrlBreak,
    #[cfg(windows)]
    close: tokio::signal::windows::CtrlClose,
    #[cfg(windows)]
    shutdown: tokio::signal::windows::CtrlShutdown,
    #[cfg(unix)]
    int: tokio::signal::unix::Signal,
    #[cfg(unix)]
    term: tokio::signal::unix::Signal,
    #[cfg(unix)]
    hup: tokio::signal::unix::Signal,
}
impl Interrupt {
    fn new() -> Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            Ok(Self {
                int: signal(SignalKind::interrupt())?,
                term: signal(SignalKind::terminate())?,
                hup: signal(SignalKind::hangup())?,
            })
        }
        #[cfg(windows)]
        {
            use tokio::signal::windows;
            Ok(Self {
                int: windows::ctrl_c()?,
                brk: windows::ctrl_break()?,
                close: windows::ctrl_close()?,
                shutdown: windows::ctrl_shutdown()?,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {})
        }
    }
    async fn wait(&mut self) {
        #[cfg(unix)]
        {
            tokio::select! { _ = self.int.recv() => (), _ = self.term.recv() => (), _ = self.hup.recv() => () }
        }
        #[cfg(windows)]
        {
            tokio::select! { _ = self.int.recv() => (), _ = self.brk.recv() => (),
            _ = self.close.recv() => (), _ = self.shutdown.recv() => () }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = self;
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caller_drop_forbids_late_worker_entry() {
        let control = Control::default();
        let caller = CancelOnDrop(control.clone());
        drop(caller);
        assert!(
            control
                .start()
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        let state = control.0.lock().unwrap();
        assert!(state.cancelled && state.terminal.is_none());
    }

    #[test]
    fn unicode_decoder_handles_split_utf8_and_utf16_without_loss() {
        for text in ["雪λ", "a🦀é"] {
            for utf16 in [false, true] {
                let units: Vec<u16> = if utf16 {
                    text.encode_utf16().collect()
                } else {
                    text.bytes().map(u16::from).collect()
                };
                let mut decoder = Decoder::default();
                let result: String = units
                    .into_iter()
                    .filter_map(|unit| decoder.push(unit, utf16).unwrap())
                    .collect();
                assert_eq!(result, text);
                assert!(decoder.is_empty());
            }
        }
    }

    #[test]
    fn unicode_decoder_rejects_malformed_sequences_and_retains_partial_state() {
        for (utf16, units) in [
            (false, vec![0xff]),
            (false, vec![0xe9, 0x20]),
            (true, vec![0xdc00]),
            (true, vec![0xd800, 0x41]),
        ] {
            let mut decoder = Decoder::default();
            assert!(
                units
                    .into_iter()
                    .any(|unit| decoder.push(unit, utf16).is_err())
            );
        }
        for (utf16, unit) in [(false, 0xe9), (true, 0xd800)] {
            let mut decoder = Decoder::default();
            assert!(decoder.push(unit, utf16).unwrap().is_none());
            assert!(!decoder.is_empty());
        }
    }
}
