//! Local plain-chat ownership, byte handoff, and transient PTY screen.
#[cfg(all(test, unix))]
mod fixture;
use crate::{policy::Policy,terminal::{InteractiveTerminals,PlainDetachFilter,TerminalId,TerminalSnapshot,TerminalState,TerminalSummary}};
use anyhow::{Result,ensure};
use std::{io::{self,IsTerminal,Write},sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};
use tokio_util::sync::CancellationToken;

const MAX_PENDING:usize=64*1024;
static ATTACHED:AtomicBool=AtomicBool::new(false);
/// Plain output/decision adapters consult this before touching the outer terminal.
pub fn owns_terminal()->bool {ATTACHED.load(Ordering::Acquire)}
struct Ownership;
impl Ownership {
    fn acquire()->Result<Self>{ensure!(!ATTACHED.swap(true,Ordering::AcqRel),"another plain attachment owns the terminal");Ok(Self)}
}
impl Drop for Ownership {fn drop(&mut self){ATTACHED.store(false,Ordering::Release);}}

/// Unsubmitted post-detach input. Human bytes before the chord never enter here.
#[derive(Default)]
pub struct PendingInput {bytes:Vec<u8>}
impl PendingInput {
    pub fn append(&mut self,bytes:&[u8])->Result<()> {
        ensure!(self.bytes.len().saturating_add(bytes.len())<=MAX_PENDING,"pending Helm input exceeds 64 KiB; no prompt was submitted");
        self.bytes.extend_from_slice(bytes);Ok(())
    }
    pub fn is_empty(&self)->bool {self.bytes.is_empty()}
    pub fn take_line(&mut self)->Result<Option<String>> {
        let Some(end)=self.bytes.iter().position(|byte|matches!(byte,b'\n'|b'\r')) else{return Ok(None)};
        let text=std::str::from_utf8(&self.bytes[..end]).map_err(|_|anyhow::anyhow!("pending Helm input is not UTF-8; no prompt was submitted"))?.to_owned();
        let consumed=end+1+usize::from(self.bytes.get(end)==Some(&b'\r')&&self.bytes.get(end+1)==Some(&b'\n'));
        self.bytes.drain(..consumed);Ok(Some(text))
    }
    fn backspace(&mut self){
        self.bytes.pop();
        while !self.bytes.is_empty()&&std::str::from_utf8(&self.bytes).is_err(){self.bytes.pop();}
    }
}

pub fn select(items:&[TerminalSummary],reference:&str)->Result<TerminalId>{
    let by_id=uuid::Uuid::parse_str(reference).ok().map(TerminalId);
    let matches=items.iter().filter(|item|Some(item.id)==by_id||item.title==reference).collect::<Vec<_>>();
    ensure!(matches.len()==1,"terminal selection is missing or ambiguous; use an exact listed ID");
    ensure!(matches[0].state==TerminalState::Running,"terminal is no longer running; saved metadata cannot reattach a process");
    Ok(matches[0].id)
}


#[derive(Default)]
struct InputEncoding { #[cfg(not(unix))] high: Option<u16> }
impl InputEncoding {
    fn push(&mut self, unit:u16, repeat:u16, bytes:&mut Vec<u8>)->Result<()> {
        for _ in 0..repeat.min(64) {
            #[cfg(unix)] bytes.push(u8::try_from(unit).map_err(|_|anyhow::anyhow!("invalid terminal input"))?);
            #[cfg(not(unix))] {
                if (0xd800..=0xdbff).contains(&unit) { ensure!(self.high.replace(unit).is_none(),"invalid console Unicode"); continue; }
                let units = if let Some(high)=self.high.take(){vec![high,unit]}else{vec![unit]};
                let text=String::from_utf16(&units).map_err(|_|anyhow::anyhow!("invalid console Unicode"))?;
                bytes.extend_from_slice(text.as_bytes());
            }
        }
        Ok(())
    }
}
/// Resume editing a partial byte suffix without a competing buffered stdin reader.
/// Complete lines are returned first; later lines remain queued for later prompts.
pub async fn pending_prompt(pending:&mut PendingInput,cancel:CancellationToken)->Result<Option<String>> {
    if let Some(line)=pending.take_line()? {return Ok(Some(line));}
    ensure!(io::stdin().is_terminal()&&io::stdout().is_terminal(),"pending terminal input needs a local TTY");
    let mut input=crate::terminal_input::Terminal::enter_preserving_input()?;
    let result=async {
        let mut displayed=String::new();
        let mut encoding=InputEncoding::default();
        loop {
            if cancel.is_cancelled(){return Ok(None);}
            let visible=visible(&String::from_utf8_lossy(&pending.bytes));
            if visible!=displayed {print!("\r\x1b[2Khelm> {visible}");io::stdout().flush()?;displayed=visible;}
            match input.read()? {
                Some((3,_))=>return Ok(None),
                Some((4,_)) if pending.is_empty()=>return Ok(None),
                Some((21,_))=>pending.bytes.clear(),
                Some((8|127,_))=>pending.backspace(),
                Some((byte,repeat))=>{
                    let mut bytes=Vec::new();encoding.push(byte,repeat,&mut bytes)?;pending.append(&bytes)?;
                    if let Some(line)=pending.take_line()?{return Ok(Some(line));}
                },
                None=>tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    }.await;
    input.restore()?;println!();result
}

fn visible(text:&str)->String {
    text.chars().flat_map(|c| if c.is_control()||matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {c.escape_default().collect::<Vec<_>>()} else {vec![c]}).collect()
}

/// Only bytes following a locally consumed detach chord may become Helm input.
pub struct Detached {pub pending:Vec<u8>,pub exited:bool}
struct Screen {output:Output}
impl Screen {
    fn enter()->Result<Self>{let mut screen=Self{output:Output::new()?};screen.output.write(b"\x1b[?1049h\x1b[?25l")?;Ok(screen)}
}
impl Drop for Screen {fn drop(&mut self){let _=self.output.write(b"\x1b[0m\x1b[?25h\x1b[?1049l");}}

#[cfg(unix)]
struct Output {saved:i32}
#[cfg(unix)]
impl Output {
    fn new()->Result<Self>{
        let saved=unsafe{libc::fcntl(libc::STDOUT_FILENO,libc::F_GETFL)};
        ensure!(saved>=0,"outer terminal output unavailable");
        ensure!(unsafe{libc::fcntl(libc::STDOUT_FILENO,libc::F_SETFL,saved|libc::O_NONBLOCK)}==0,"outer terminal output cannot be bounded");
        Ok(Self{saved})
    }
    fn write(&mut self,mut bytes:&[u8])->Result<()> {
        let end=std::time::Instant::now()+Duration::from_millis(250);
        while !bytes.is_empty(){
            let count=unsafe{libc::write(libc::STDOUT_FILENO,bytes.as_ptr().cast(),bytes.len())};
            if count>0 {bytes=&bytes[count as usize..];continue;}
            let error=io::Error::last_os_error();
            ensure!(error.kind()==io::ErrorKind::WouldBlock||error.kind()==io::ErrorKind::Interrupted,"outer terminal output failed");
            ensure!(std::time::Instant::now()<end,"outer terminal output stalled; attachment stopped");
            let mut descriptor=libc::pollfd{fd:libc::STDOUT_FILENO,events:libc::POLLOUT,revents:0};
            unsafe{libc::poll(&mut descriptor,1,10)};
        }
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for Output {fn drop(&mut self){unsafe{libc::fcntl(libc::STDOUT_FILENO,libc::F_SETFL,self.saved)};}}
#[cfg(not(unix))]
struct Output;
#[cfg(not(unix))]
impl Output {fn new()->Result<Self>{Ok(Self)} fn write(&mut self,bytes:&[u8])->Result<()>{io::stdout().write_all(bytes)?;io::stdout().flush()?;Ok(())}}

fn frame(snapshot:&TerminalSnapshot,columns:u16,rows:u16)->Vec<u8>{
    use std::fmt::Write as _;
    use crate::terminal::TerminalColor;
    let mut text=String::from("\x1b[?25l\x1b[0m\x1b[H\x1b[2K");
    let title=format!("Ctrl+T/Ctrl+] detach · private · {}",visible(&snapshot.title));
    let mut occupied=0;
    for ch in title.chars() {
        let width=unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if occupied+width>=usize::from(columns) { break; }
        occupied+=width;text.push(ch);
    }
    for (row,cells) in snapshot.cells.iter().take(usize::from(rows.saturating_sub(1))).enumerate(){
        let _=write!(text,"\x1b[{};1H\x1b[0m\x1b[2K",row+2);
        let mut skip=0;
        for cell in cells.iter().take(usize::from(columns)) {
            if skip>0 {skip-=1;continue;}
            text.push_str("\x1b[0m");
            for (color,foreground) in [(cell.foreground,true),(cell.background,false)] {
                let base=if foreground {38}else{48};
                match color {TerminalColor::Default=>(),TerminalColor::Indexed(n)=>{let _=write!(text,"\x1b[{base};5;{n}m");},TerminalColor::Rgb(r,g,b)=>{let _=write!(text,"\x1b[{base};2;{r};{g};{b}m");}}
            }
            for (enabled,code) in [(cell.bold,1),(cell.dim,2),(cell.italic,3),(cell.underlined,4),(cell.reversed,7)] {if enabled{let _=write!(text,"\x1b[{code}m");}}
            let value=if cell.text.is_empty(){" ".into()}else{visible(&cell.text)};
            skip=unicode_width::UnicodeWidthStr::width(value.as_str()).saturating_sub(1);
            text.push_str(&value);
        }
    }
    if let Some((column,row))=snapshot.cursor.filter(|(column,row)|*column<columns&&row.saturating_add(1)<rows){let _=write!(text,"\x1b[{};{}H\x1b[?25h",row+2,column+1);}
    text.into_bytes()
}

pub async fn attach(manager:Arc<dyn InteractiveTerminals>,id:TerminalId,policy:Arc<Policy>,cancel:CancellationToken)->Result<Detached>{
    ensure!(io::stdin().is_terminal()&&io::stdout().is_terminal(),"plain terminal attachment requires local TTY input and output");
    policy.check_current()?;
    ensure!(policy.access_mode()!=crate::config::AccessMode::ReadOnly,"direct terminal input is denied in read-only mode");
    let _ownership=Ownership::acquire()?;
    let mut input=crate::terminal_input::Terminal::enter_preserving_input()?;
    let mut screen=Screen::enter()?;
    let mut events=manager.subscribe();
    let mut snapshot=manager.attach(id).await?;
    let mut dimensions=(0,0);let mut revision=None;let mut filter=PlainDetachFilter::default();let mut encoding=InputEncoding::default();
    let result=async {
        loop {
            if cancel.is_cancelled(){anyhow::bail!("plain attachment interrupted");}
            policy.check_current()?;
            let (columns,rows)=crossterm::terminal::size()?;
            ensure!(columns>=4&&rows>=3,"terminal viewport is too small; detached");
            let size=(columns.min(240),rows.min(120));
            if dimensions!=size {manager.resize(id,size.0,size.1-1).await?;dimensions=size;revision=None;snapshot=manager.snapshot(id).await?;}
            if revision!=Some(snapshot.revision){screen.output.write(&frame(&snapshot,size.0,size.1))?;revision=Some(snapshot.revision);}
            if snapshot.state!=TerminalState::Running{return Ok(Detached{pending:vec![],exited:true});}
            let mut bytes=Vec::new();
            for _ in 0..4096 {
                match input.read()? {Some((byte,repeat))=>{
                    encoding.push(byte,repeat,&mut bytes)?;
                },None=>break}
                if bytes.len()>=4096{break;}
            }
            if !bytes.is_empty(){
                let separated=filter.push(&bytes);
                if !separated.terminal_input.is_empty(){manager.write(id,separated.terminal_input.to_vec()).await?;}
                if separated.detached{return Ok(Detached{pending:separated.helm_input.to_vec(),exited:false});}
            }
            tokio::select!{
                biased;
                _=cancel.cancelled()=>anyhow::bail!("plain attachment interrupted"),
                event=events.recv()=>{match event{Ok(_)|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>snapshot=manager.snapshot(id).await?,Err(_)=>anyhow::bail!("terminal events closed")}},
                _=tokio::time::sleep(Duration::from_millis(20))=>snapshot=manager.snapshot(id).await?,
            }
        }
    }.await;
    // Preserve bytes still in the OS queue as well as the explicit returned suffix.
    input.restore()?;drop(screen);result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_detach_suffix_preserves_unicode_multiple_lines_and_each_split(){
        let bytes="private秘密\u{14}next🧭\nsecond\n".as_bytes();
        for split in 0..=bytes.len(){
            let mut filter=PlainDetachFilter::default();let mut terminal=vec![];let mut pending=PendingInput::default();
            for chunk in [&bytes[..split],&bytes[split..]] {let separated=filter.push(chunk);terminal.extend_from_slice(separated.terminal_input);pending.append(separated.helm_input).unwrap();}
            assert_eq!(terminal,"private秘密".as_bytes());
            assert_eq!(pending.take_line().unwrap().as_deref(),Some("next🧭"));
            assert_eq!(pending.take_line().unwrap().as_deref(),Some("second"));assert!(pending.is_empty());
        }
    }
    #[test]
    fn pending_input_overflow_and_invalid_utf8_preserve_unsubmitted_bytes(){
        let mut pending=PendingInput::default();pending.append(&vec![b'x';MAX_PENDING]).unwrap();
        assert!(pending.append(b"y").is_err());assert_eq!(pending.bytes.len(),MAX_PENDING);
        pending.bytes=vec![255,b'\n'];assert!(pending.take_line().is_err());assert_eq!(pending.bytes,vec![255,b'\n']);
        pending.bytes="prefix🧭".as_bytes().to_vec();pending.backspace();assert_eq!(pending.bytes,b"prefix");
    }
    #[test]
    fn selection_requires_current_unique_running_identity(){
        let a=TerminalSummary{id:TerminalId(uuid::Uuid::new_v4()),title:"same".into(),state:TerminalState::Running};
        let b=TerminalSummary{id:TerminalId(uuid::Uuid::new_v4()),..a.clone()};
        assert!(select(&[a.clone(),b],"same").is_err());assert_eq!(select(&[a.clone()],&a.id.to_string()).unwrap(),a.id);
        assert!(select(&[],&a.id.to_string()).is_err());
        let stale=TerminalSummary{state:TerminalState::Disconnected,..a};assert!(select(&[stale],"same").is_err());
    }
}

