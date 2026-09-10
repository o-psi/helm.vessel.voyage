//! Bounded, local presentation data; never provider or voyage instructions.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
use unicode_width::UnicodeWidthStr;

const DEFAULTS: &[u8] = include_bytes!("../../../../../assets/working-statuses.json");
const MAX_BYTES: usize = 64 * 1024;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) struct Labels {
    pub frames: Vec<[String; 10]>,
    pub width: u16,
}

impl Labels {
    pub fn from_env() -> Result<Self> {
        match std::env::var_os("HELM_WORKING_STATUSES") {
            None => Self::parse(DEFAULTS).context("invalid bundled working statuses"),
            Some(path) => Self::read(Path::new(&path))
                .context("HELM_WORKING_STATUSES must name a valid working-status JSON file"),
        }
    }

    fn read(path: &Path) -> Result<Self> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        // A mistakenly supplied FIFO must not block terminal startup on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        ensure!(
            std::fs::metadata(path)?.is_file(),
            "expected a regular file"
        );
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "expected a regular file");
        ensure!(metadata.len() <= MAX_BYTES as u64, "file exceeds 64 KiB");
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        Self::parse(&bytes)
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BYTES, "file exceeds 64 KiB");
        let labels: Vec<String> =
            serde_json::from_slice(bytes).context("expected a JSON array of strings")?;
        ensure!(
            !labels.is_empty() && labels.len() <= 128,
            "expected 1–128 labels"
        );
        let mut width = 0;
        for (i, label) in labels.iter().enumerate() {
            let columns = label.width();
            ensure!(
                !label.trim().is_empty() && label == label.trim()
                    && label.len() <= 128 && (1..=32).contains(&columns)
                    && !label.chars().any(|c| c.is_control()
                        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')),
                "label {} must be trimmed readable text, 1–32 columns and at most 128 UTF-8 bytes, without control or bidi characters", i + 1
            );
            width = width.max(columns);
        }
        let frames = labels
            .into_iter()
            .map(|label| {
                let padding = " ".repeat(width - label.width());
                std::array::from_fn(|i| format!("{} {label}{padding}", SPINNER[i]))
            })
            .collect();
        Ok(Self {
            frames,
            width: width as u16,
        })
    }
}
