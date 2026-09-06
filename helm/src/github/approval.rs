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

fn usable(size: (u16,u16)) -> bool { size.0 >= 60 && size.1 >= 8 }

fn render(writer: &mut impl Write, wrapped: &[String], offset: usize, rows: usize) -> std::io::Result<bool> {
    let end = (offset + rows).min(wrapped.len());
    crossterm::queue!(writer,crossterm::cursor::MoveTo(0,0),crossterm::terminal::Clear(crossterm::terminal::ClearType::All))?;
    write!(writer,"Exact GitHub preview [{}/{}]\r\n",offset+1,wrapped.len())?;
    for line in &wrapped[offset..end] { write!(writer,"{line}\r\n")?; }
    let final_page = end == wrapped.len();
    let hint = if final_page { "[y] confirm  [b] back  [n/Esc] deny" } else { "[Space] next  [b] back  [n/Esc] deny" };
    write!(writer,"{hint}\r\n")?;
    writer.flush()?;
    Ok(final_page)
}

struct Screen;
impl Screen {
    fn enter() -> std::io::Result<Self> {
        let screen = Self;
        crossterm::execute!(std::io::stderr(),crossterm::terminal::EnterAlternateScreen,crossterm::event::EnableBracketedPaste)?;
        Ok(screen)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stderr(),crossterm::event::DisableBracketedPaste,crossterm::terminal::LeaveAlternateScreen);
    }
}

pub async fn approve_terminal(request: &ApprovalRequest) -> ApprovalOutcome {
    if request.mode != InteractionMode::Attended || !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() || request.reason.len() > 256 * 1024 {
        return ApprovalOutcome::Unavailable;
    }
    let Ok(mut input) = crate::terminal_input::Terminal::enter() else { return ApprovalOutcome::Unavailable };
    let Ok(_screen) = Screen::enter() else { return ApprovalOutcome::Unavailable };
    let mut offset = 0usize;
    let mut last_size = (0,0);
    let mut wrapped = Vec::new();
    let mut redraw = true;
    let mut presented_end = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(900);
    let text = format!("Action: {}\nTarget: {}\nRequest: {}\n\n{}",request.action,request.target,request.id,request.reason);
    let outcome = loop {
        let Ok(size) = crossterm::terminal::size() else { break ApprovalOutcome::Unavailable };
        if !usable(size) { break ApprovalOutcome::Unavailable; }
        if size != last_size {
            last_size = size;
            offset = 0;
            wrapped = lines(&text,usize::from(size.0)-1);
            redraw = true;
            presented_end = false;
            if input.discard().is_err() { break ApprovalOutcome::Unavailable; }
        }
        let rows = usize::from(size.1)-3;
        let end = (offset + rows).min(wrapped.len());
        if redraw {
            match render(&mut std::io::stderr().lock(),&wrapped,offset,rows) {
                Ok(final_page) => presented_end = final_page,
                Err(_) => break ApprovalOutcome::Unavailable,
            }
            redraw = false;
        }
        match input.read() {
            Ok(Some((code,repeat))) => match code {
                // Bracketed paste begins with ESC and is denied, not interpreted
                // as approval. Plain unmarked input has no trustworthy origin.
                3 | 27 | 110 | 78 => break ApprovalOutcome::Denied,
                121 | 89 if presented_end && repeat == 1 => break ApprovalOutcome::Approved,
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
    if input.discard().is_err() || input.restore().is_err() { return ApprovalOutcome::Unavailable; }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_and_render_failures_never_enable_confirmation() {
        assert!(!usable((1,1)));
        assert!(!usable((59,24)));
        assert!(!usable((80,7)));
        assert!(usable((60,8)));
        struct Broken { flush: bool }
        impl Write for Broken {
            fn write(&mut self,bytes:&[u8]) -> std::io::Result<usize> {
                if self.flush { Ok(bytes.len()) } else { Err(std::io::Error::other("injected write failure")) }
            }
            fn flush(&mut self) -> std::io::Result<()> { Err(std::io::Error::other("injected flush failure")) }
        }
        for flush in [false,true] { assert!(render(&mut Broken { flush },&["exact body".into()],0,5).is_err()); }
    }
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
