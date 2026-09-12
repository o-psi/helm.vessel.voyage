//! Read-only, bounded catalogue projection owned by the journal's schema library.
//! Callers authenticate and authorize before disclosure. This never acquires an
//! execution owner, reconstructs configuration, runs migrations, or starts work.
use anyhow::{Result, ensure};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::OpenOptions,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::process::CatalogueSummary;

pub fn read(directory: &Path, session: Uuid) -> Result<CatalogueSummary> {
    let metadata = std::fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "private catalogue journal required"
    );
    for name in [
        "journal.sqlite3",
        "journal.sqlite3-journal",
        "journal.sqlite3-wal",
        "journal.sqlite3-shm",
    ] {
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(directory.join(name))
        {
            Ok(file) => {
                let m = file.metadata()?;
                ensure!(
                    m.is_file()
                        && m.nlink() == 1
                        && m.uid() == unsafe { libc::geteuid() }
                        && m.mode() & 0o077 == 0,
                    "unsafe catalogue journal file"
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && name != "journal.sqlite3" => {}
            Err(e) => return Err(e.into()),
        }
    }
    let mut db = Connection::open_with_flags(
        directory.join("journal.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(50))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let tx = db.transaction()?;
    let version: i64 = tx.query_row(
        "SELECT version FROM attachment_schema WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        (2..=12).contains(&version),
        "unsupported catalogue journal version"
    );
    // SQL projects only display fields; conversation/tool/provider payloads never
    // leave the journal. The read transaction gives one consistent revision.
    let encoded:String=tx.query_row("SELECT json_object(
        'session_id',id,'revision',revision,'observation_cursor',coalesce((SELECT max(cursor) FROM process_observations WHERE session_id=s.id),0),
        'name',json_extract(state,'$.name'),'model',json_extract(state,'$.model'),
        'created_at',json_extract(state,'$.created_at'),
        'last_turn_end',(SELECT max(json_extract(value,'$.finished_at')) FROM json_each(s.state,'$.run_summaries')),
        'total_messages',json_array_length(state,'$.messages'),
        'run_id',(SELECT id FROM runs WHERE session_id=s.id ORDER BY rowid DESC LIMIT 1),
        'run_state',(SELECT json_extract(record,'$.state') FROM runs WHERE session_id=s.id ORDER BY rowid DESC LIMIT 1),
        'archived',json(CASE WHEN coalesce((SELECT archived FROM process_lifecycle WHERE session_id=s.id),0) THEN 'true' ELSE 'false' END),
        'deleted',json(CASE WHEN coalesce((SELECT deleted FROM process_lifecycle WHERE session_id=s.id),0) THEN 'true' ELSE 'false' END),
        'pending_cleanup_run',(SELECT run_id FROM local_cleanup_obligations WHERE session_id=s.id AND confirmation IS NULL LIMIT 1)
    ) FROM sessions s WHERE id=?1",[session.to_string()],|r|r.get(0))?;
    ensure!(encoded.len() <= 8192, "catalogue projection exceeds bounds");
    let summary: CatalogueSummary = serde_json::from_str(&encoded)?;
    ensure!(
        summary.session_id == session
            && summary.name.as_ref().is_none_or(|s| s.len() <= 512)
            && summary.model.len() <= 512,
        "invalid catalogue projection"
    );
    Ok(summary)
}
