//! Helm's process clients hold presentation state and never construct an executor.
pub mod cli;
pub mod connections;
pub mod duplex;
pub mod local;
pub mod transport;
mod ui;

pub fn safe(value: &str) -> String {
    value
        .chars()
        .filter(|ch| {
            (!ch.is_control() || matches!(ch, '\n' | '\t'))
                && !matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>()
        .replace('\t', "    ")
}

pub mod plain;

mod access;

mod commands;
mod inbox;

pub mod terminal;

pub mod frontend;

mod admin;

pub mod export;

mod artifacts;

/// Explicit human-authorized local browser resources, never an agent runtime.
pub mod browser;
