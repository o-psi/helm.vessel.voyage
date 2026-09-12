//! Embedded supervisor storage. Canonical conversations remain in Voyage journals.
//! Files required by old/live runtimes are derived registration projections.
use super::registry;
use anyhow::{Context, Result, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::process::{
    CatalogueMetadata, CatalogueSummary, ProcessInfo, ProcessRegistration,
};

const FILE: &str = "catalogue.sqlite3";
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn private_file(path: &Path) -> Result<()> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "unsafe supervisor database file"
    );
    Ok(())
}
fn open(root: &Path) -> Result<Connection> {
    registry::private_directory(root)?;
    for name in [FILE, "catalogue.sqlite3-journal"] {
        private_file(&root.join(name))?;
    }
    for name in ["catalogue.sqlite3-wal", "catalogue.sqlite3-shm"] {
        ensure!(
            !root.join(name).try_exists()?,
            "unexpected supervisor WAL state"
        );
    }
    let mut db = Connection::open_with_flags(
        root.join(FILE),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL; PRAGMA journal_size_limit=1048576;")?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_version')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let initialized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_version')",
            [],
            |r| r.get(0),
        )?;
        if !initialized {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            ensure!(count == 0, "unrecognized supervisor database");
            tx.execute_batch(include_str!("database.sql"))?;
        }
        tx.commit()?;
        fs::File::open(root)?.sync_all()?;
    }
    let page_size: u64 = db.pragma_query_value(None, "page_size", |r| r.get(0))?;
    ensure!(page_size > 0, "invalid supervisor database page size");
    db.pragma_update(None, "max_page_count", (512 * 1024 * 1024u64) / page_size)?;
    let version: i64 = db.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| {
        r.get(0)
    })?;
    ensure!(version == 1, "unsupported supervisor database version");
    Ok(db)
}
fn save_tx(tx: &Transaction<'_>, registration: &ProcessRegistration) -> Result<()> {
    let previous: Option<String> = tx
        .query_row(
            "SELECT session_id FROM incarnations WHERE incarnation=?1",
            [registration.incarnation.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        previous
            .as_ref()
            .is_none_or(|id| id == &registration.session_id.to_string()),
        "incarnation belongs to another voyage"
    );
    let current: Option<String> = tx
        .query_row(
            "SELECT incarnation FROM voyages WHERE session_id=?1",
            [registration.session_id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        current
            .as_ref()
            .is_none_or(|current| current == &registration.incarnation.to_string()
                || (previous.is_none()
                    && registration.restart_from.map(|id| id.to_string()).as_ref()
                        == Some(current))),
        "stale incarnation publication"
    );
    let bytes = serde_json::to_string(registration)?;
    ensure!(bytes.len() < 16384, "registration exceeds limit");
    let id = registration.session_id.to_string();
    let inc = registration.incarnation.to_string();
    let state = serde_json::to_value(&registration.state)?
        .as_str()
        .context("invalid process state")?
        .to_owned();
    tx.execute("INSERT INTO voyages VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session_id) DO UPDATE SET incarnation=excluded.incarnation,workspace=excluded.workspace,state=excluded.state,name=excluded.name,registration=excluded.registration,updated_at_ms=excluded.updated_at_ms",params![id,inc,registration.workspace.as_os_str().as_encoded_bytes(),state,registration.name,bytes,now()])?;
    tx.execute("INSERT INTO incarnations VALUES(?1,?2,?3,?4,?5) ON CONFLICT(incarnation) DO UPDATE SET registration=excluded.registration",params![inc,id,registration.command_id.to_string(),bytes,now()])?;
    tx.execute("INSERT INTO catalogue(session_id,incarnation) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET incarnation=excluded.incarnation,fingerprint=CASE WHEN incarnation!=excluded.incarnation THEN NULL ELSE fingerprint END,error_code=CASE WHEN incarnation!=excluded.incarnation THEN 'owner_changed' ELSE error_code END,next_attempt_ms=0",params![id,inc])?;
    Ok(())
}
fn record_tx(
    tx: &Transaction<'_>,
    namespace: &str,
    id: Uuid,
    bytes: &[u8],
    reserve: bool,
) -> Result<bool> {
    ensure!(
        bytes.len() <= 16384,
        "lifecycle command exceeds receipt limit"
    );
    let saved: Option<Vec<u8>> = tx
        .query_row(
            "SELECT request FROM lifecycle_commands WHERE namespace=?1 AND command_id=?2",
            params![namespace, id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(saved) = saved {
        ensure!(saved == bytes, "lifecycle command ID payload conflict");
        return Ok(true);
    }
    if reserve {
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM lifecycle_commands WHERE namespace=?1",
            [namespace],
            |r| r.get(0),
        )?;
        ensure!(count < 65536, "lifecycle receipt capacity exhausted");
        tx.execute(
            "INSERT INTO lifecycle_commands VALUES(?1,?2,?3)",
            params![namespace, id.to_string(), bytes],
        )?;
    }
    Ok(false)
}
async fn blocking<T: Send + 'static>(
    root: &Path,
    f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
) -> Result<T> {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || f(&mut open(&root)?)).await?
}
pub async fn save(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    let r = registration.clone();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_tx(&tx, &r)?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub async fn command(
    root: &Path,
    namespace: &str,
    id: Uuid,
    bytes: Vec<u8>,
    reserve: bool,
) -> Result<bool> {
    let namespace = namespace.to_owned();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = record_tx(&tx, &namespace, id, &bytes, reserve)?;
        tx.commit()?;
        Ok(result)
    })
    .await
}
pub async fn admit(root: &Path, registration: &ProcessRegistration, bytes: Vec<u8>) -> Result<()> {
    let r = registration.clone();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            !record_tx(&tx, "start-resolution/intent", r.command_id, &bytes, false)?,
            "start command fenced as not admitted"
        );
        ensure!(
            !record_tx(&tx, "commands", r.command_id, &bytes, true)?,
            "creation already reserved"
        );
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM voyages WHERE session_id=?1)",
            [r.session_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(!exists, "voyage already reserved");
        save_tx(&tx, &r)?;
        tx.execute(
            "INSERT INTO catalogue_events(session_id,kind,recorded_at_ms) VALUES(?1,'created',?2)",
            params![r.session_id.to_string(), now()],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
pub async fn creation_receipt(root: &Path, id: Uuid) -> Result<Option<ProcessInfo>> {
    blocking(root, move |db| {
        let s: Option<String> = db
            .query_row(
                "SELECT result FROM creation_receipts WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    })
    .await
}
pub async fn settle_creation(root: &Path, id: Uuid, info: &ProcessInfo) -> Result<()> {
    let info = info.clone();
    blocking(root, move |db| {
        db.execute(
            "INSERT OR IGNORE INTO creation_receipts VALUES(?1,?2,?3)",
            params![
                id.to_string(),
                info.session_id.to_string(),
                serde_json::to_string(&info)?
            ],
        )?;
        Ok(())
    })
    .await
}

/// One supervisor lock surrounds import and registration publication. Legacy
/// input is preserved; once imported, SQLite alone supplies registration state.
pub async fn initialize(root: &Path) -> Result<HashMap<Uuid, ProcessRegistration>> {
    let path = root.to_owned();
    blocking(root,move|db|{
  let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
  let imported:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE source='complete-v1')",[],|r|r.get(0))?;
  if !imported {
   for entry in fs::read_dir(path.join("sessions"))? {
    let dir=entry?.path();let source=dir.join("registration.json");
    if !source.try_exists()? {continue}
    registry::private_directory(&dir)?;
    let r=registry::load(&source)?;
    ensure!(dir==registry::directory(&path,r.session_id) && r.protocol==voyage_protocol::process::PROCESS_PROTOCOL,"invalid legacy registration identity");
    save_tx(&tx,&r)?;
    let digest=Sha256::digest(fs::read(&source)?);
    tx.execute("INSERT INTO legacy_imports VALUES(?1,?2)",params![format!("sessions/{}/registration.json",r.session_id),digest.as_slice()])?;
   }
   for namespace in ["commands","start-resolution/intent","start-resolution/not-admitted"] {
    let dir=if namespace=="commands" {path.join(namespace)}else{path.join(namespace).join("commands")};
    if !dir.try_exists()? {continue}
    registry::private_directory(&dir)?;
    for entry in fs::read_dir(dir)? {
     let source=entry?.path();if source.extension().and_then(|v|v.to_str())!=Some("json"){continue}
     let id=Uuid::parse_str(source.file_stem().and_then(|v|v.to_str()).context("invalid receipt name")?)?;
     let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&source)?;
     let m=file.metadata()?;ensure!(m.is_file() && m.nlink()==1 && m.uid()==unsafe{libc::geteuid()} && m.mode()&0o077==0 && m.len()<=16384,"unsafe legacy receipt");
     use std::io::Read;let mut bytes=Vec::new();file.take(16385).read_to_end(&mut bytes)?;
     record_tx(&tx,namespace,id,&bytes,true)?;
    }
   }
   tx.execute("INSERT INTO legacy_imports VALUES('complete-v1',?1)",[&[0u8;32][..]])?;
  }
  tx.execute("UPDATE catalogue SET process_info=NULL,fingerprint=NULL,error_code='refresh_pending',next_attempt_ms=0",[])?;
  let rows=tx.prepare("SELECT registration FROM voyages ORDER BY session_id")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  ensure!(rows.len()<=4096,"supervisor registration retention limit reached");
  let registrations=rows.into_iter().map(|s|{let r:ProcessRegistration=serde_json::from_str(&s)?;Ok((r.session_id,r))}).collect::<Result<HashMap<_,_>>>()?;
  tx.commit()?;
  Ok(registrations)
 }).await
}

pub async fn catalogue(root: &Path) -> Result<Vec<ProcessInfo>> {
    blocking(root,move|db|{
  let mut stmt=db.prepare("SELECT v.registration,c.summary,c.observed_at_ms,c.error_code,c.process_info FROM voyages v LEFT JOIN catalogue c USING(session_id) ORDER BY v.session_id")?;
  let rows=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?)))?;
  let mut infos=Vec::new();
  for row in rows {let (reg,summary,observed,error,info)=row?;let r:ProcessRegistration=serde_json::from_str(&reg)?;
   if matches!(r.initialize,Some(voyage_protocol::process::RuntimeInitialization::Participant{..})) {continue}
   let mut i=info.and_then(|s|serde_json::from_str::<ProcessInfo>(&s).ok()).filter(|i|i.incarnation==r.incarnation).unwrap_or_else(||ProcessInfo::from(&r));
   if i.state==voyage_protocol::process::ProcessState::Live && observed.is_none_or(|at|now().saturating_sub(at)>5000){i.state=voyage_protocol::process::ProcessState::Unavailable;}
   let summary:Option<CatalogueSummary>=summary.map(|s|serde_json::from_str(&s)).transpose()?;
   if let Some(s)=&summary {i.name=s.name.clone().or(i.name);}
   i.catalogue=Some(Box::new(CatalogueMetadata{summary,observed_at_ms:observed.and_then(|v|v.try_into().ok()),stale:error.is_some() || observed.is_none(),error_code:error}));
   infos.push(i);
  }Ok(infos)
 }).await
}

pub async fn refresh(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    refresh_mode(root, registration, false).await
}
pub async fn refresh_now(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    refresh_mode(root, registration, true).await
}
async fn refresh_mode(root: &Path, registration: &ProcessRegistration, force: bool) -> Result<()> {
    let r = registration.clone();
    let path = root.to_owned();
    blocking(root,move|db|{
  let dir=registry::directory(&path,r.session_id);
  let fingerprint=fingerprint(&dir);
  let old:(Option<String>,i64)=db.query_row("SELECT fingerprint,next_attempt_ms FROM catalogue WHERE session_id=?1",[r.session_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
  if !force && (old.1>now() || (fingerprint.is_some() && fingerprint==old.0)){return Ok(())}
  let summary=voyage_runtime::catalogue::read(&dir.join("journal"),r.session_id);
  let mut info=ProcessInfo::from(&r);
  info.state=if super::recovery::suspended(&dir,&r){voyage_protocol::process::ProcessState::Suspended}
   else if r.state==voyage_protocol::process::ProcessState::Relinquished{r.state.clone()}
   else if super::recovery::clean_stop(&dir,&r){voyage_protocol::process::ProcessState::Stopped}
   else {voyage_protocol::process::ProcessState::Unavailable};
  if info.state==voyage_protocol::process::ProcessState::Stopped {info.archive=super::recovery::archived(&dir,&r);info.deletion=super::recovery::deletion(&dir,&r);}
  let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
  let current:String=tx.query_row("SELECT incarnation FROM voyages WHERE session_id=?1",[r.session_id.to_string()],|row|row.get(0))?;
  if current!=r.incarnation.to_string(){return Ok(())}
  match summary {
   Ok(summary)=>{
    let previous:Option<String>=tx.query_row("SELECT summary FROM catalogue WHERE session_id=?1",[r.session_id.to_string()],|row|row.get(0))?;
    if let Some(previous)=previous {
        let previous:CatalogueSummary=serde_json::from_str(&previous)?;
        if (summary.revision,summary.observation_cursor)<(previous.revision,previous.observation_cursor) {return Ok(())}
    }
    tx.execute("UPDATE catalogue SET summary=?2,observed_at_ms=?3,fingerprint=?4,process_info=?5,error_code=NULL,failures=0,next_attempt_ms=0 WHERE session_id=?1",params![r.session_id.to_string(),serde_json::to_string(&summary)?,now(),fingerprint,serde_json::to_string(&info)?])?;},
   Err(_)=>{tx.execute("UPDATE catalogue SET process_info=?2,error_code='journal_unavailable',failures=min(failures+1,6),next_attempt_ms=?3+min(60000,1000*(1<<min(failures,6))) WHERE session_id=?1",params![r.session_id.to_string(),serde_json::to_string(&info)?,now()])?;}
  }
  tx.execute("INSERT INTO catalogue_events(session_id,kind,recorded_at_ms) VALUES(?1,'metadata',?2)",params![r.session_id.to_string(),now()])?;
  tx.commit()?;Ok(())
 }).await
}
fn fingerprint(dir: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for name in [
        "journal/journal.sqlite3",
        "journal/journal.sqlite3-journal",
        "journal/journal.sqlite3-wal",
        "stopped.json",
        "runtime.sock",
    ] {
        match fs::symlink_metadata(dir.join(name)) {
            Ok(m) => parts.push(format!(
                "{}:{}:{}:{}:{}",
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime_nsec()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => parts.push("absent".into()),
            Err(_) => return None,
        }
    }
    Some(parts.join("/"))
}

pub async fn observe_process(
    root: &Path,
    registration: &ProcessRegistration,
    info: &ProcessInfo,
) -> Result<()> {
    let r = registration.clone();
    let i = info.clone();
    blocking(root,move|db|{db.execute("UPDATE catalogue SET process_info=?3,observed_at_ms=?4 WHERE session_id=?1 AND incarnation=?2",params![r.session_id.to_string(),r.incarnation.to_string(),serde_json::to_string(&i)?,now()])?;Ok(())}).await
}

/// Serialize lifecycle operations while loading their starting state from SQLite.
/// A cancelled caller cannot strand a committed incarnation in an old memory map.
pub struct Registrations {
    root: std::path::PathBuf,
    serial: tokio::sync::Mutex<()>,
}
pub struct RegistrationGuard<'a> {
    records: HashMap<Uuid, ProcessRegistration>,
    _serial: tokio::sync::MutexGuard<'a, ()>,
}
impl std::ops::Deref for RegistrationGuard<'_> {
    type Target = HashMap<Uuid, ProcessRegistration>;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
impl std::ops::DerefMut for RegistrationGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.records
    }
}
impl Registrations {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self {
            root,
            serial: tokio::sync::Mutex::new(()),
        }
    }
    pub async fn lock(&self) -> Result<RegistrationGuard<'_>> {
        let serial = self.serial.lock().await;
        let records = blocking(&self.root, |db| {
            let mut query = db.prepare("SELECT registration FROM voyages")?;
            let rows = query.query_map([], |r| r.get::<_, String>(0))?;
            let mut records = HashMap::new();
            for row in rows {
                let registration: ProcessRegistration = serde_json::from_str(&row?)?;
                records.insert(registration.session_id, registration);
            }
            Ok(records)
        })
        .await?;
        Ok(RegistrationGuard {
            records,
            _serial: serial,
        })
    }
}

pub async fn registration(root: &Path, session: Uuid) -> Result<ProcessRegistration> {
    blocking(root, move |db| {
        let encoded: String = db
            .query_row(
                "SELECT registration FROM voyages WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .context("unknown session")?;
        Ok(serde_json::from_str(&encoded)?)
    })
    .await
}

#[cfg(test)]
mod tests;
