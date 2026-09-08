//! An immutable intent is fsynced before any effect. Existing intents are never replayed.
use super::*;
use std::io::Write;

pub(super) enum Admission {
    Fresh,
    Existing(Value),
}

#[cfg(unix)]
fn directory(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(failed("cannot create private Vessel journal directory")),
    }
    transport::private_directory(path)
}
#[cfg(not(unix))]
fn directory(_: &Path) -> Result<(), ToolError> {
    Err(failed(
        "durable Vessel journal unsupported on this platform",
    ))
}

fn owner_resources(local: &Path, owner: Uuid) -> Result<PathBuf, ToolError> {
    transport::private_directory(local)?;
    let sessions = local.join("sessions");
    transport::private_directory(&sessions)?;
    let session = sessions.join(owner.to_string());
    transport::private_directory(&session)?;
    Ok(session.join("resources"))
}

pub(super) fn root(local: &Path, owner: Uuid) -> Result<PathBuf, ToolError> {
    let resources = owner_resources(local, owner)?;
    directory(&resources)?;
    sync(
        resources
            .parent()
            .ok_or_else(|| failed("invalid resource path"))?,
    )?;
    let root = resources.join("vessel-coordination");
    directory(&root)?;
    sync(&resources)?;
    Ok(root)
}
fn sync(path: &Path) -> Result<(), ToolError> {
    std::fs::File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| failed("Vessel journal directory sync failed"))
}
#[cfg(unix)]
fn exclusive(path: &Path, value: &Value) -> Result<(), ToolError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| {
            failed("Vessel journal reservation failed; consult receipt, do not retry effects")
        })?;
    let bytes = serde_json::to_vec(value).map_err(|_| failed("Vessel journal encoding failed"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| failed("Vessel journal persistence failed; no effects replayed"))?;
    sync(
        path.parent()
            .ok_or_else(|| failed("invalid journal path"))?,
    )
}
#[cfg(not(unix))]
fn exclusive(_: &Path, _: &Value) -> Result<(), ToolError> {
    Err(failed("Vessel journal unsupported"))
}

pub(super) fn admit(root: &Path, id: Uuid, intent: &Value) -> Result<Admission, ToolError> {
    let path = root.join(format!("{id}.intent.json"));
    if path
        .try_exists()
        .map_err(|_| failed("cannot inspect Vessel journal"))?
    {
        let previous: Value =
            serde_json::from_slice(&transport::private_read(&path, 4 * 1024 * 1024)?)
                .map_err(|_| failed("incomplete Vessel intent; outcome unknown, never replay"))?;
        if &previous != intent {
            return Err(invalid(
                "command_id already reserved for a different request",
            ));
        }
        let retained = receipt(root, id)?;
        return Ok(Admission::Existing(
            retained.get("result").cloned().unwrap_or(retained),
        ));
    }
    exclusive(&path, intent)?;
    Ok(Admission::Fresh)
}
pub(super) fn finish(root: &Path, id: Uuid, result: &Value) -> Result<(), ToolError> {
    exclusive(&root.join(format!("{id}.result.json")), result)
}
pub(super) fn existing_root(local: &Path, owner: Uuid) -> Result<PathBuf, ToolError> {
    let resources = owner_resources(local, owner)?;
    let root = resources.join("vessel-coordination");
    for path in [&resources, &root] {
        if path
            .try_exists()
            .map_err(|_| failed("cannot inspect journal directory"))?
        {
            transport::private_directory(path)?;
        }
    }
    Ok(root)
}
fn metadata(root: &Path, id: Uuid) -> Result<Value, ToolError> {
    let path = root.join(format!("{id}.intent.json"));
    let intent: Value = serde_json::from_slice(&transport::private_read(&path, 4 * 1024 * 1024)?)
        .map_err(|_| failed("incomplete intent; unknown outcome"))?;
    let request = &intent["request"];
    Ok(
        json!({"command_id":id,"target":intent["target"],"action":request["action"],"session_id":request["session_id"],"start_command_id": if request["action"] == "create" {request["session_id"].clone()} else {Value::Null}}),
    )
}
pub(super) fn receipt(root: &Path, id: Uuid) -> Result<Value, ToolError> {
    let intent = root.join(format!("{id}.intent.json"));
    if !intent
        .try_exists()
        .map_err(|_| failed("cannot inspect Vessel intent"))?
    {
        return Ok(json!({"status":"not_found","command_id":id}));
    }
    let metadata = metadata(root, id)?;
    let result = root.join(format!("{id}.result.json"));
    if result
        .try_exists()
        .map_err(|_| failed("cannot inspect Vessel receipt"))?
    {
        let result: Value =
            serde_json::from_slice(&transport::private_read(&result, 4 * 1024 * 1024)?).map_err(
                |_| failed("incomplete Vessel result; outcome unknown, consult server receipt"),
            )?;
        return Ok(json!({"intent":metadata,"result":result,"replayed":false}));
    }
    Ok(
        json!({"status":"outcome_unknown", "intent":metadata, "replayed":false, "detail":"Durable intent exists without a confirmed result. Inspect the session or query its server receipt; never resubmit uncertain effects."}),
    )
}
pub(super) fn operations(root: &Path, offset: usize, limit: u32) -> Result<Value, ToolError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"operations":[],"next_offset":null}));
        }
        Err(_) => return Err(failed("cannot enumerate Vessel intents")),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| failed("cannot read Vessel journal entry"))?;
        let name = entry.file_name();
        if let Some(id) = name
            .to_str()
            .and_then(|s| s.strip_suffix(".intent.json"))
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            ids.push(id);
        }
    }
    ids.sort();
    let end = offset.saturating_add(limit as usize).min(ids.len());
    let operations = ids
        .get(offset..end)
        .unwrap_or(&[])
        .iter()
        .map(|id| metadata(root, *id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"operations":operations,"total":ids.len(),"next_offset":(end < ids.len()).then_some(end)}),
    )
}

pub(super) fn wire(root: &Path, id: Uuid, command: &Value) -> Result<(), ToolError> {
    let op = command
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| failed("wire command has no operation"))?;
    if !op.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return Err(failed("invalid wire operation"));
    }
    exclusive(&root.join(format!("{id}.{op}.command.json")), command)
}
