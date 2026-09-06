//! Helm's process clients hold presentation state and never construct an executor.
pub mod cli;
pub mod local;
mod ssh;
pub mod transport;
mod ui;

pub fn safe(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
