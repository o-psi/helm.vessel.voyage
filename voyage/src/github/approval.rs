//! Bounded attended CLI paging; a TTY is a frontend, not an OS identity boundary.
use crate::tools::{ApprovalOutcome, ApprovalRequest, InteractionMode};
use std::io::{IsTerminal, Write};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut output = Vec::new();
    for line in text.lines() {
        let mut current = String::new();
        let mut used = 0;
        for grapheme in line.graphemes(true) {
            let size = UnicodeWidthStr::width(grapheme);
            if used + size > width && !current.is_empty() {
                output.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push_str(grapheme);
            used += size;
        }
        output.push(current);
    }
    output
}

fn usable(size: (u16, u16)) -> bool {
    size.0 >= 60 && size.1 >= 8
}

fn render(
    writer: &mut impl Write,
    wrapped: &[String],
    offset: usize,
    rows: usize,
) -> std::io::Result<bool> {
    let end = (offset + rows).min(wrapped.len());
    crossterm::queue!(
        writer,
        crossterm::cursor::MoveTo(0, 0),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
    )?;
    write!(
        writer,
        "Exact GitHub preview [{}/{}]\r\n",
        offset + 1,
        wrapped.len()
    )?;
    for line in &wrapped[offset..end] {
        write!(writer, "{line}\r\n")?;
    }
    let final_page = end == wrapped.len();
    let hint = if final_page {
        "[y] confirm  [b] back  [n/Esc] deny"
    } else {
        "[Space] next  [b] back  [n/Esc] deny"
    };
    write!(writer, "{hint}\r\n")?;
    writer.flush()?;
    Ok(final_page)
}

struct Screen;
impl Screen {
    fn enter() -> std::io::Result<Self> {
        let screen = Self;
        crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableBracketedPaste
        )?;
        Ok(screen)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::event::DisableBracketedPaste,
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}

pub async fn approve_terminal(request: &ApprovalRequest) -> ApprovalOutcome {
    if request.mode != InteractionMode::Attended
        || !std::io::stdin().is_terminal()
        || !std::io::stderr().is_terminal()
        || request.reason.len() > 256 * 1024
    {
        return ApprovalOutcome::Unavailable;
    }
    let Ok(mut input) = crate::terminal_input::Terminal::enter() else {
        return ApprovalOutcome::Unavailable;
    };
    let Ok(_screen) = Screen::enter() else {
        return ApprovalOutcome::Unavailable;
    };
    let mut offset = 0usize;
    let mut last_size = (0, 0);
    let mut wrapped = Vec::new();
    let mut redraw = true;
    let mut presented_end = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(900);
    let text = format!(
        "Action: {}\nTarget: {}\nRequest: {}\n\n{}",
        request.action, request.target, request.id, request.reason
    );
    let outcome = loop {
        let Ok(size) = crossterm::terminal::size() else {
            break ApprovalOutcome::Unavailable;
        };
        if !usable(size) {
            break ApprovalOutcome::Unavailable;
        }
        if size != last_size {
            last_size = size;
            offset = 0;
            wrapped = lines(&text, usize::from(size.0) - 1);
            redraw = true;
            presented_end = false;
            if input.discard().is_err() {
                break ApprovalOutcome::Unavailable;
            }
        }
        let rows = usize::from(size.1) - 3;
        let end = (offset + rows).min(wrapped.len());
        if redraw {
            match render(&mut std::io::stderr().lock(), &wrapped, offset, rows) {
                Ok(final_page) => presented_end = final_page,
                Err(_) => break ApprovalOutcome::Unavailable,
            }
            redraw = false;
        }
        match input.read() {
            Ok(Some((code, repeat))) => match code {
                // Bracketed paste begins with ESC and is cancelled, not interpreted
                // as approval. Plain unmarked input has no trustworthy origin.
                3 | 27 => break ApprovalOutcome::Cancelled,
                110 | 78 => break ApprovalOutcome::Denied,
                121 | 89 if presented_end && repeat == 1 => break ApprovalOutcome::Approved,
                32 | 13 | 10 if end < wrapped.len() => {
                    offset = end;
                    redraw = true;
                }
                98 | 66 => {
                    offset = offset.saturating_sub(rows);
                    redraw = true;
                }
                _ => (),
            },
            Err(_) => break ApprovalOutcome::Unavailable,
            Ok(None) => (),
        }
        if tokio::time::Instant::now() >= deadline {
            break ApprovalOutcome::Expired;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    if input.discard().is_err() || input.restore().is_err() {
        return ApprovalOutcome::Unavailable;
    }
    outcome
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn unicode_preview_wrapping_keeps_graphemes_and_all_exact_body_text() {
        let body = "α🦀e\u{301}界\nexact second line";
        let wrapped = lines(body, 4);
        assert_eq!(wrapped.concat(), body.replace('\n', ""));
        assert!(
            wrapped
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 4)
        );
        assert!(wrapped.iter().any(|line| line.contains("e\u{301}")));
        assert_eq!(lines("wide界", 0).concat(), "wide界");
    }

    #[test]
    fn incomplete_preview_never_displays_confirmation_until_final_page() {
        let body = vec!["one".into(), "two".into(), "three".into()];
        let mut first = Vec::new();
        assert!(!render(&mut first, &body, 0, 2).unwrap());
        let first = String::from_utf8(first).unwrap();
        assert!(first.contains("one\r\ntwo\r\n"));
        assert!(first.contains("[Space] next"));
        assert!(!first.contains("[y] confirm"));
        let mut last = Vec::new();
        assert!(render(&mut last, &body, 2, 2).unwrap());
        let last = String::from_utf8(last).unwrap();
        assert!(last.contains("three\r\n"));
        assert!(last.contains("[y] confirm"));
        assert!(!last.contains("[Space] next"));
    }

    #[test]
    fn unusable_screen_and_failed_preview_do_not_report_final_presentation() {
        for size in [(0, 0), (59, 8), (60, 7)] {
            assert!(!usable(size));
        }
        assert!(usable((60, 8)));
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fixture display unavailable"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(render(&mut FailedWriter, &["exact body".into()], 0, 1).is_err());
    }
}
