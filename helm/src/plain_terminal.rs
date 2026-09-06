//! Local plain-chat ownership, byte handoff, and transient PTY screen.
#[cfg(all(test, unix))]
mod fixture;
use crate::{
    policy::Policy,
    terminal::{
        InteractiveTerminals, PlainDetachFilter, TerminalId, TerminalSnapshot, TerminalState,
        TerminalSummary,
    },
};
use anyhow::{Result, ensure};
#[cfg(not(unix))]
use std::io::Write;
use std::{
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const MAX_PENDING: usize = 64 * 1024;
static ATTACHED: AtomicBool = AtomicBool::new(false);
/// Plain output/decision adapters consult this before touching the outer terminal.
pub fn owns_terminal() -> bool {
    ATTACHED.load(Ordering::Acquire)
}
struct Ownership;
impl Ownership {
    fn acquire() -> Result<Self> {
        ensure!(
            !ATTACHED.swap(true, Ordering::AcqRel),
            "another plain attachment owns the terminal"
        );
        Ok(Self)
    }
}
impl Drop for Ownership {
    fn drop(&mut self) {
        ATTACHED.store(false, Ordering::Release);
    }
}

/// Unsubmitted post-detach input. Human bytes before the chord never enter here.
#[derive(Default)]
pub struct PendingInput {
    bytes: Vec<u8>,
}
impl PendingInput {
    pub fn append(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(
            self.bytes.len().saturating_add(bytes.len()) <= MAX_PENDING,
            "pending Helm input exceeds 64 KiB; no prompt was submitted"
        );
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    pub fn discard(&mut self) {
        self.bytes.clear();
    }
    pub fn take_line(&mut self) -> Result<Option<String>> {
        let Some(end) = self
            .bytes
            .iter()
            .position(|byte| matches!(byte, b'\n' | b'\r'))
        else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&self.bytes[..end])
            .map_err(|_| {
                anyhow::anyhow!("pending Helm input is not UTF-8; no prompt was submitted")
            })?
            .to_owned();
        let consumed = end
            + 1
            + usize::from(
                self.bytes.get(end) == Some(&b'\r') && self.bytes.get(end + 1) == Some(&b'\n'),
            );
        self.bytes.drain(..consumed);
        Ok(Some(text))
    }
    fn backspace(&mut self) {
        let start = previous_scalar_start(&self.bytes, self.bytes.len());
        self.bytes.truncate(start);
    }
}

// Inspect at most one UTF-8 scalar. Earlier malformed bytes remain editable.
fn previous_scalar_start(bytes: &[u8], end: usize) -> usize {
    if end == 0 {
        return 0;
    }
    let mut start = end - 1;
    for _ in 0..3 {
        if start == 0 || (bytes[start] & 0xc0) != 0x80 {
            break;
        }
        start -= 1;
    }
    if std::str::from_utf8(&bytes[start..end])
        .ok()
        .is_some_and(|text| text.chars().count() == 1)
    {
        start
    } else {
        end - 1
    }
}

/// A failed TTY attachment command cannot classify its queued tail as a prompt.
/// Clear it before displaying the refusal; piped input retains ordinary semantics.
pub fn discard_failed_attempt(pending: &mut PendingInput) -> Result<()> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }
    pending.discard();
    let mut input = crate::terminal_input::Terminal::enter_preserving_input().map_err(|_| {
        anyhow::anyhow!("queued attachment input could not be isolated; chat must stop")
    })?;
    input.discard().map_err(|_| {
        anyhow::anyhow!("queued attachment input could not be discarded; chat must stop")
    })?;
    input
        .restore()
        .map_err(|_| anyhow::anyhow!("terminal input restoration failed; chat must stop"))?;
    Ok(())
}

pub fn select(items: &[TerminalSummary], reference: &str) -> Result<TerminalId> {
    let by_id = uuid::Uuid::parse_str(reference).ok().map(TerminalId);
    let matches = items
        .iter()
        .filter(|item| Some(item.id) == by_id || item.title == reference)
        .collect::<Vec<_>>();
    ensure!(
        matches.len() == 1,
        "terminal selection is missing or ambiguous; use an exact listed ID"
    );
    ensure!(
        matches[0].state == TerminalState::Running,
        "terminal is no longer running; saved metadata cannot reattach a process"
    );
    Ok(matches[0].id)
}

#[derive(Default)]
struct InputEncoding {
    #[cfg(not(unix))]
    high: Option<u16>,
}
impl InputEncoding {
    fn push(&mut self, unit: u16, repeat: u16, bytes: &mut Vec<u8>) -> Result<()> {
        for _ in 0..repeat.min(64) {
            #[cfg(unix)]
            bytes.push(u8::try_from(unit).map_err(|_| anyhow::anyhow!("invalid terminal input"))?);
            #[cfg(not(unix))]
            {
                if (0xd800..=0xdbff).contains(&unit) {
                    ensure!(self.high.replace(unit).is_none(), "invalid console Unicode");
                    continue;
                }
                let units = if let Some(high) = self.high.take() {
                    vec![high, unit]
                } else {
                    vec![unit]
                };
                let text = String::from_utf16(&units)
                    .map_err(|_| anyhow::anyhow!("invalid console Unicode"))?;
                bytes.extend_from_slice(text.as_bytes());
            }
        }
        Ok(())
    }
}
/// Resume editing a partial byte suffix without a competing buffered stdin reader.
/// Complete lines are returned first; later lines remain queued for later prompts.
pub async fn pending_prompt(
    pending: &mut PendingInput,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    if let Ok(Some(line)) = pending.take_line() {
        return Ok(Some(line));
    }
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "pending terminal input needs a local TTY"
    );
    let mut input = crate::terminal_input::Terminal::enter_preserving_input()?;
    let mut output = Output::new()?;
    let result = async {
        let mut displayed = String::new();
        let mut displayed_cursor = usize::MAX;
        let mut encoding = InputEncoding::default();
        let mut escape = Vec::new();
        let mut cursor = pending.bytes.len();
        let mut blocked = false;
        loop {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let columns = crossterm::terminal::size()?.0.max(2);
            let (display, back) = prompt_view(&pending.bytes, cursor, columns, blocked);
            if display != displayed || back != displayed_cursor {
                output.write(format!("\r\x1b[2K{display}").as_bytes())?;
                if back > 0 {
                    output.write(format!("\x1b[{back}D").as_bytes())?;
                }
                displayed = display;
                displayed_cursor = back;
            }
            match input.read()? {
                Some((3, _)) => return Ok(None),
                Some((4, _)) if pending.is_empty() => return Ok(None),
                Some((21, _)) => {
                    pending.bytes.clear();
                    cursor = 0;
                    escape.clear();
                    blocked = false;
                }
                Some((8 | 127, _)) => {
                    if cursor == pending.bytes.len() {
                        pending.backspace();
                        cursor = pending.bytes.len();
                    } else if cursor > 0 {
                        let end = cursor;
                        cursor = previous_scalar_start(&pending.bytes, end);
                        pending.bytes.drain(cursor..end);
                    }
                    blocked = false;
                }
                Some((unit, repeat)) => {
                    let mut bytes = Vec::new();
                    encoding.push(unit, repeat, &mut bytes)?;
                    for byte in bytes {
                        if byte == 27 && escape.is_empty() {
                            escape.push(byte);
                            continue;
                        }
                        if !escape.is_empty() {
                            escape.push(byte);
                            if escape.len() == 2 && byte == b'[' {
                                continue;
                            }
                            if escape.len() > 2 && (byte.is_ascii_digit() || byte == b';') {
                                if escape.len() > 16 {
                                    escape.clear();
                                }
                                continue;
                            }
                            match escape.as_slice() {
                                b"\x1b[D" => {
                                    if cursor > 0 {
                                        cursor -= 1;
                                        while cursor > 0 && (pending.bytes[cursor] & 0xc0) == 0x80 {
                                            cursor -= 1;
                                        }
                                    }
                                }
                                b"\x1b[C" => {
                                    if cursor < pending.bytes.len() {
                                        cursor += 1;
                                        while cursor < pending.bytes.len()
                                            && (pending.bytes[cursor] & 0xc0) == 0x80
                                        {
                                            cursor += 1;
                                        }
                                    }
                                }
                                b"\x1b[H" | b"\x1b[1~" => cursor = 0,
                                b"\x1b[F" | b"\x1b[4~" => cursor = pending.bytes.len(),
                                b"\x1b[3~" => {
                                    if cursor < pending.bytes.len() {
                                        let mut end = cursor + 1;
                                        while end < pending.bytes.len()
                                            && (pending.bytes[end] & 0xc0) == 0x80
                                        {
                                            end += 1;
                                        }
                                        pending.bytes.drain(cursor..end);
                                    }
                                }
                                _ => (),
                            }
                            escape.clear();
                            continue;
                        }
                        if blocked {
                            continue;
                        }
                        if matches!(byte, b'\n' | b'\r') {
                            if pending.append(&[byte]).is_err() {
                                blocked = true;
                                continue;
                            }
                            if let Ok(Some(line)) = pending.take_line() {
                                return Ok(Some(line));
                            }
                            cursor = pending.bytes.len();
                            continue;
                        }
                        if byte < 32 && byte != b'\t' {
                            continue;
                        }
                        if pending.bytes.len() == MAX_PENDING {
                            blocked = true;
                            continue;
                        }
                        pending.bytes.insert(cursor, byte);
                        cursor += 1;
                    }
                }
                None => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    }
    .await;
    let _ = output.write(b"\r\n");
    input.restore()?;
    drop(output);
    result
}

fn prompt_view(bytes: &[u8], cursor: usize, columns: u16, blocked: bool) -> (String, usize) {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let width = usize::from(columns.saturating_sub(1));
    let prefix = if width < 8 {
        ">"
    } else if blocked && width >= 18 {
        "limit; Ctrl+U> "
    } else {
        "helm> "
    };
    let room = width.saturating_sub(prefix.len());
    let before = visible(&String::from_utf8_lossy(&bytes[..cursor.min(bytes.len())]));
    let after = visible(&String::from_utf8_lossy(&bytes[cursor.min(bytes.len())..]));
    let mut used = 0;
    let mut left = Vec::new();
    for ch in before.chars().rev() {
        let size = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + size > room {
            break;
        }
        used += size;
        left.push(ch);
    }
    let mut text = prefix.to_owned();
    text.extend(left.into_iter().rev());
    let mut suffix = String::new();
    for ch in after.chars() {
        let size = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + size > room {
            break;
        }
        used += size;
        suffix.push(ch);
    }
    let back = UnicodeWidthStr::width(suffix.as_str());
    text.push_str(&suffix);
    (text, back)
}

fn visible(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Only bytes following a locally consumed detach chord may become Helm input.
pub struct Detached {
    pub pending: Vec<u8>,
    pub exited: bool,
    pub delivery_failed: bool,
}
struct Screen {
    output: Output,
}
impl Screen {
    fn enter() -> Result<Self> {
        let mut screen = Self {
            output: Output::new()?,
        };
        screen.output.write(b"\x1b[?1049h\x1b[?25l")?;
        Ok(screen)
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.output.write(b"\x1b[0m\x1b[?25h\x1b[?1049l");
    }
}

#[cfg(unix)]
struct Output {
    saved: i32,
}
#[cfg(unix)]
impl Output {
    fn new() -> Result<Self> {
        let saved = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL) };
        ensure!(saved >= 0, "outer terminal output unavailable");
        ensure!(
            unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, saved | libc::O_NONBLOCK) }
                == 0,
            "outer terminal output cannot be bounded"
        );
        Ok(Self { saved })
    }
    fn write(&mut self, mut bytes: &[u8]) -> Result<()> {
        let end = std::time::Instant::now() + Duration::from_millis(250);
        while !bytes.is_empty() {
            let count =
                unsafe { libc::write(libc::STDOUT_FILENO, bytes.as_ptr().cast(), bytes.len()) };
            if count > 0 {
                bytes = &bytes[count as usize..];
                continue;
            }
            let error = io::Error::last_os_error();
            ensure!(
                error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::Interrupted,
                "outer terminal output failed"
            );
            ensure!(
                std::time::Instant::now() < end,
                "outer terminal output stalled; attachment stopped"
            );
            let mut descriptor = libc::pollfd {
                fd: libc::STDOUT_FILENO,
                events: libc::POLLOUT,
                revents: 0,
            };
            unsafe { libc::poll(&mut descriptor, 1, 10) };
        }
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for Output {
    fn drop(&mut self) {
        unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, self.saved) };
    }
}
#[cfg(not(unix))]
struct Output;
#[cfg(not(unix))]
impl Output {
    fn new() -> Result<Self> {
        Ok(Self)
    }
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        io::stdout().write_all(bytes)?;
        io::stdout().flush()?;
        Ok(())
    }
}

fn frame(snapshot: &TerminalSnapshot, columns: u16, rows: u16) -> Vec<u8> {
    use crate::terminal::TerminalColor;
    use std::fmt::Write as _;
    let mut text = String::from("\x1b[?25l\x1b[0m\x1b[H\x1b[2K");
    let title = format!(
        "Ctrl+T/Ctrl+] detach · private · {}",
        visible(&snapshot.title)
    );
    let mut occupied = 0;
    for ch in title.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if occupied + width >= usize::from(columns) {
            break;
        }
        occupied += width;
        text.push(ch);
    }
    for (row, cells) in snapshot
        .cells
        .iter()
        .take(usize::from(rows.saturating_sub(1)))
        .enumerate()
    {
        let _ = write!(text, "\x1b[{};1H\x1b[0m\x1b[2K", row + 2);
        let mut skip = 0;
        for cell in cells.iter().take(usize::from(columns)) {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            text.push_str("\x1b[0m");
            for (color, foreground) in [(cell.foreground, true), (cell.background, false)] {
                let base = if foreground { 38 } else { 48 };
                match color {
                    TerminalColor::Default => (),
                    TerminalColor::Indexed(n) => {
                        let _ = write!(text, "\x1b[{base};5;{n}m");
                    }
                    TerminalColor::Rgb(r, g, b) => {
                        let _ = write!(text, "\x1b[{base};2;{r};{g};{b}m");
                    }
                }
            }
            for (enabled, code) in [
                (cell.bold, 1),
                (cell.dim, 2),
                (cell.italic, 3),
                (cell.underlined, 4),
                (cell.reversed, 7),
            ] {
                if enabled {
                    let _ = write!(text, "\x1b[{code}m");
                }
            }
            let value = if cell.text.is_empty() {
                " ".into()
            } else {
                visible(&cell.text)
            };
            skip = unicode_width::UnicodeWidthStr::width(value.as_str()).saturating_sub(1);
            text.push_str(&value);
        }
    }
    if let Some((column, row)) = snapshot
        .cursor
        .filter(|(column, row)| *column < columns && row.saturating_add(1) < rows)
    {
        let _ = write!(text, "\x1b[{};{}H\x1b[?25h", row + 2, column + 1);
    }
    text.into_bytes()
}

// Before an explicit detach, every unread byte is still private PTY input.
struct AttachmentInput {
    inner: crate::terminal_input::Terminal,
    preserve: bool,
    finished: bool,
}
impl AttachmentInput {
    fn new() -> Result<Self> {
        Ok(Self {
            inner: crate::terminal_input::Terminal::enter_preserving_input()?,
            preserve: false,
            finished: false,
        })
    }
    fn finish(&mut self) -> Result<()> {
        if !self.finished {
            if !self.preserve {
                let _ = self.inner.discard();
            }
            self.inner.restore()?;
            self.finished = true;
        }
        Ok(())
    }
}
impl Drop for AttachmentInput {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

async fn terminal_operation<T>(
    future: impl std::future::Future<Output = std::result::Result<T, crate::terminal::TerminalError>>,
    cancel: &CancellationToken,
) -> Result<T> {
    tokio::select! {biased;
        _=cancel.cancelled()=>anyhow::bail!("plain attachment interrupted"),
        result=tokio::time::timeout(Duration::from_secs(1),future)=>Ok(result.map_err(|_|anyhow::anyhow!("terminal operation timed out; detached"))??),
    }
}

pub async fn attach(
    manager: Arc<dyn InteractiveTerminals>,
    id: TerminalId,
    policy: Arc<Policy>,
    cancel: CancellationToken,
) -> Result<Detached> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "plain terminal attachment requires local TTY input and output"
    );
    policy.check_current()?;
    ensure!(
        policy.access_mode() != crate::config::AccessMode::ReadOnly,
        "direct terminal input is denied in read-only mode"
    );
    let _ownership = Ownership::acquire()?;
    let mut input = AttachmentInput::new()?;
    let mut screen = Screen::enter()?;
    let mut events = manager.subscribe();
    let mut snapshot = terminal_operation(manager.attach(id), &cancel).await?;
    let mut dimensions = (0, 0);
    let mut revision = None;
    let mut filter = PlainDetachFilter::default();
    let mut encoding = InputEncoding::default();
    let result=async {
        loop {
            if cancel.is_cancelled(){anyhow::bail!("plain attachment interrupted");}
            policy.check_current()?;
            let (columns,rows)=crossterm::terminal::size()?;
            ensure!(columns>=4&&rows>=3,"terminal viewport is too small; detached");
            let size=(columns.min(240),rows.min(120));
            if dimensions!=size {terminal_operation(manager.resize(id,size.0,size.1-1),&cancel).await?;dimensions=size;revision=None;snapshot=terminal_operation(manager.snapshot(id),&cancel).await?;}
            if revision!=Some(snapshot.revision){screen.output.write(&frame(&snapshot,size.0,size.1))?;revision=Some(snapshot.revision);}
            if snapshot.state!=TerminalState::Running{return Ok(Detached{pending:vec![],exited:true,delivery_failed:false});}
            let mut bytes=Vec::new();
            for _ in 0..4096 {
                match input.inner.read()? {Some((byte,repeat))=>{
                    encoding.push(byte,repeat,&mut bytes)?;
                },None=>break}
                if bytes.len()>=4096{break;}
            }
            if !bytes.is_empty(){
                let separated=filter.push(&bytes);
                let delivered=if separated.terminal_input.is_empty(){Ok(())}else{terminal_operation(manager.write(id,separated.terminal_input.to_vec()),&cancel).await};
                if separated.detached {
                    input.preserve=true;
                    return Ok(Detached{pending:separated.helm_input.to_vec(),exited:false,delivery_failed:delivered.is_err()});
                }
                delivered?;
            }
            tokio::select!{
                biased;
                _=cancel.cancelled()=>anyhow::bail!("plain attachment interrupted"),
                event=events.recv()=>{match event{Ok(_)|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>snapshot=terminal_operation(manager.snapshot(id),&cancel).await?,Err(_)=>anyhow::bail!("terminal events closed")}},
                _=tokio::time::sleep(Duration::from_millis(20))=>snapshot=terminal_operation(manager.snapshot(id),&cancel).await?,
            }
        }
    }.await;
    // Preserve bytes still in the OS queue as well as the explicit returned suffix.
    input.finish()?;
    drop(screen);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_detach_suffix_preserves_unicode_multiple_lines_and_each_split() {
        let bytes = "private秘密\u{14}next🧭\nsecond\n".as_bytes();
        for split in 0..=bytes.len() {
            let mut filter = PlainDetachFilter::default();
            let mut terminal = vec![];
            let mut pending = PendingInput::default();
            for chunk in [&bytes[..split], &bytes[split..]] {
                let separated = filter.push(chunk);
                terminal.extend_from_slice(separated.terminal_input);
                pending.append(separated.helm_input).unwrap();
            }
            assert_eq!(terminal, "private秘密".as_bytes());
            assert_eq!(pending.take_line().unwrap().as_deref(), Some("next🧭"));
            assert_eq!(pending.take_line().unwrap().as_deref(), Some("second"));
            assert!(pending.is_empty());
        }
    }
    #[test]
    fn pending_input_overflow_and_invalid_utf8_preserve_unsubmitted_bytes() {
        let mut pending = PendingInput::default();
        pending.append(&vec![b'x'; MAX_PENDING]).unwrap();
        assert!(pending.append(b"y").is_err());
        assert_eq!(pending.bytes.len(), MAX_PENDING);
        pending.bytes = vec![255, b'\n'];
        assert!(pending.take_line().is_err());
        assert_eq!(pending.bytes, vec![255, b'\n']);
        pending.bytes = "prefix🧭".as_bytes().to_vec();
        pending.backspace();
        assert_eq!(pending.bytes, b"prefix");
    }
    #[test]
    fn backspace_removes_only_the_final_scalar_or_invalid_byte() {
        for (bytes, expected) in [
            (
                [vec![0xff], b"keep".to_vec()].concat(),
                [vec![0xff], b"kee".to_vec()].concat(),
            ),
            ("前🧭".as_bytes().to_vec(), "前".as_bytes().to_vec()),
            (vec![b'x', 0xff], vec![b'x']),
            (vec![b'x', 0xf0, 0x9f, 0xa7], vec![b'x', 0xf0, 0x9f]),
            (
                vec![b'x', 0x80, 0x80, 0x80, 0x80, 0x80],
                vec![b'x', 0x80, 0x80, 0x80, 0x80],
            ),
        ] {
            let mut pending = PendingInput { bytes };
            pending.backspace();
            assert_eq!(pending.bytes, expected);
        }
        let mut bytes = vec![b'x'; MAX_PENDING / 2];
        bytes.push(0xff);
        bytes.extend(vec![b'y'; MAX_PENDING / 2 - 1]);
        let mut pending = PendingInput {
            bytes: bytes.clone(),
        };
        pending.backspace();
        bytes.pop();
        assert_eq!(pending.bytes, bytes);
    }
    #[test]
    fn prompt_view_bounds_columns_and_keeps_control_text_inert() {
        for columns in 2..80 {
            let bytes = "前🧭next\x1b]52;c;secret\x07suffix".as_bytes();
            for cursor in [0, 3, 7, bytes.len()] {
                let (view, back) = prompt_view(bytes, cursor, columns, false);
                assert!(
                    unicode_width::UnicodeWidthStr::width(view.as_str()) < usize::from(columns)
                );
                assert!(!view.contains('\x1b') && !view.contains('\x07'));
                assert!(back <= unicode_width::UnicodeWidthStr::width(view.as_str()));
            }
        }
    }
    #[test]
    fn selection_requires_current_unique_running_identity() {
        let a = TerminalSummary {
            id: TerminalId(uuid::Uuid::new_v4()),
            title: "same".into(),
            state: TerminalState::Running,
        };
        let b = TerminalSummary {
            id: TerminalId(uuid::Uuid::new_v4()),
            ..a.clone()
        };
        assert!(select(&[a.clone(), b], "same").is_err());
        assert_eq!(select(&[a.clone()], &a.id.to_string()).unwrap(), a.id);
        assert!(select(&[], &a.id.to_string()).is_err());
        let stale = TerminalSummary {
            state: TerminalState::Disconnected,
            ..a
        };
        assert!(select(&[stale], "same").is_err());
    }
}
