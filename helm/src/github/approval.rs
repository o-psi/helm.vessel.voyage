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
            if used + size > width && !current.is_empty() { output.push(std::mem::take(&mut current)); used = 0; }
            current.push_str(grapheme);
            used += size;
        }
        output.push(current);
    }
    output
}

pub async fn approve_terminal(request: &ApprovalRequest) -> ApprovalOutcome {
    if request.mode != InteractionMode::Attended || !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() || request.reason.len() > 256 * 1024 {
        return ApprovalOutcome::Unavailable;
    }
    let Ok(mut input) = crate::terminal_input::Terminal::enter() else { return ApprovalOutcome::Unavailable };
    let mut offset = 0usize;
    let mut last_size = (0,0);
    let mut redraw = true;
    let mut presented_end = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(900);
    let text = format!("Action: {}\nTarget: {}\nRequest: {}\n\n{}",request.action,request.target,request.id,request.reason);
    let outcome = loop {
        let size = crossterm::terminal::size().unwrap_or((80,24));
        if size != last_size { last_size = size; redraw = true; }
        let rows = usize::from(size.1).saturating_sub(5).max(1);
        let wrapped = lines(&text,usize::from(size.0).saturating_sub(1).max(1));
        offset = offset.min(wrapped.len().saturating_sub(1));
        let end = (offset + rows).min(wrapped.len());
        if redraw {
            let mut stderr = std::io::stderr().lock();
            if write!(stderr,"\r\n--- Exact GitHub preview · lines {}–{} of {} ---\r\n",offset+1,end,wrapped.len()).is_err() { break ApprovalOutcome::Unavailable; }
            for line in &wrapped[offset..end] { let _ = write!(stderr,"{line}\r\n"); }
            presented_end = end == wrapped.len();
            let hint = if presented_end { "[y] confirm · [b] back · [n/Esc] deny" } else { "[Space] next · [b] back · [n/Esc] deny (confirm on final page)" };
            let _ = write!(stderr,"{hint}\r\n");
            let _ = stderr.flush();
            redraw = false;
        }
        match input.read() {
            Ok(Some((code,_))) => match code {
                3 | 27 | 110 | 78 => break ApprovalOutcome::Denied,
                121 | 89 if presented_end => break ApprovalOutcome::Approved,
                32 | 13 | 10 if end < wrapped.len() => { offset = end; redraw = true; },
                98 | 66 => { offset = offset.saturating_sub(rows); redraw = true; },
                _ => (),
            },
            Err(_) => break ApprovalOutcome::Unavailable,
            Ok(None) => (),
        }
        if tokio::time::Instant::now() >= deadline { break ApprovalOutcome::Unavailable; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    let _ = input.discard();
    if input.restore().is_err() { return ApprovalOutcome::Unavailable; }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrow_unicode_preview_preserves_every_grapheme() {
        let input = "ab🧭e\u{301}日本語\\u202e";
        for width in [1,2,3,10,80] {
            let output = lines(input,width);
            assert_eq!(output.concat(),input);
            assert!(output.iter().all(|line| UnicodeWidthStr::width(line.as_str()) <= width.max(2)));
        }
    }
}
