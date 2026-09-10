//! Public HTTP transport only: no runtime IPC, subprocesses or terminal traffic.
use super::*;
use std::io::Read;
use voyage_protocol::vessel::{
    AccessCredential, COMMAND_PATH, LocalAccessCredential, MAX_VESSEL_BODY, VESSEL_API_VERSION,
    VesselRequest, VesselResponse,
};

#[cfg(unix)]
pub(super) fn private_directory(path: &Path) -> Result<(), ToolError> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)
        .map_err(|_| failed("private Vessel directory unavailable"))?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(failed("Vessel directory must be owned and private"));
    }
    Ok(())
}
#[cfg(not(unix))]
pub(super) fn private_directory(_: &Path) -> Result<(), ToolError> {
    Err(failed(
        "private Vessel storage is unsupported on this platform",
    ))
}

#[cfg(unix)]
pub(super) fn private_read(path: &Path, limit: u64) -> Result<Vec<u8>, ToolError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| failed("private Vessel file unavailable"))?;
    let m = file
        .metadata()
        .map_err(|_| failed("private Vessel file metadata unavailable"))?;
    if !m.is_file()
        || m.nlink() != 1
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.len() > limit
    {
        return Err(failed("invalid private Vessel file"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failed("private Vessel file read failed"))?;
    if bytes.len() as u64 > limit {
        return Err(failed("private Vessel file too large"));
    }
    Ok(bytes)
}
#[cfg(not(unix))]
pub(super) fn private_read(_: &Path, _: u64) -> Result<Vec<u8>, ToolError> {
    Err(failed(
        "private Vessel credentials unsupported on this platform",
    ))
}

pub(super) struct Transport {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    token: String,
    grant: Option<Uuid>,
    journal: Option<PathBuf>,
}
impl Transport {
    pub(super) fn open(local: &Path, access: Option<&Path>) -> Result<Self, ToolError> {
        let (address, token, grant) = if let Some(path) = access {
            let c: AccessCredential = serde_json::from_slice(&private_read(path, 16384)?)
                .map_err(|_| failed("invalid Vessel grant credential"))?;
            (c.endpoint, c.token, Some(c.grant_id))
        } else {
            private_directory(local)?;
            let c: LocalAccessCredential =
                serde_json::from_slice(&private_read(&local.join("process-http.json"), 4096)?)
                    .map_err(|_| failed("invalid local Vessel discovery record"))?;
            if c.token.len() != 64 || !c.token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(failed("invalid local Vessel credential"));
            }
            (c.endpoint, c.token, None)
        };
        let mut endpoint =
            reqwest::Url::parse(&address).map_err(|_| failed("invalid Vessel endpoint"))?;
        let loopback = endpoint.host_str().is_some_and(|h| {
            h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|a| a.is_loopback())
        });
        if !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || !(endpoint.scheme() == "http" && loopback
                || grant.is_some() && endpoint.scheme() == "https")
        {
            return Err(failed(
                "Vessel requires literal-loopback HTTP locally or HTTPS for remote grants; URL credentials are forbidden",
            ));
        }
        endpoint.set_path(COMMAND_PATH);
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(35))
            .build()
            .map_err(|_| failed("Vessel HTTP client initialization failed"))?;
        Ok(Self {
            client,
            endpoint,
            token,
            grant,
            journal: None,
        })
    }
    pub(super) fn redact_complete(&self, value: Value) -> Value {
        fn scrub(value: Value, token: &str) -> Value {
            match value {
                Value::String(s) => Value::String(s.replace(token, "[REDACTED]")),
                Value::Array(a) => Value::Array(a.into_iter().map(|v| scrub(v, token)).collect()),
                Value::Object(o) => {
                    Value::Object(o.into_iter().map(|(k, v)| (k, scrub(v, token))).collect())
                }
                v => v,
            }
        }
        if self.token.is_empty() {
            value
        } else {
            scrub(value, &self.token)
        }
    }
    pub(super) fn journal(&mut self, root: PathBuf) {
        self.journal = Some(root);
    }
    pub(super) async fn exchange(&self, command: VesselCommand) -> Result<Value, ToolError> {
        let text_chunk = matches!(&command, VesselCommand::Voyage(r)
            if matches!(r.command, VoyageCommand::MessageChunk { .. } | VoyageCommand::RunOutput { .. }));
        let mutation_id = match &command {
            VesselCommand::Start { command_id, .. }
            | VesselCommand::StartConfigured { command_id, .. }
            | VesselCommand::Restart { command_id, .. } => Some(*command_id),
            VesselCommand::Voyage(r) if !matches!(r.command, VoyageCommand::Receipt { .. }) => {
                r.command.mutation_id()
            }
            _ => None,
        };
        if let Some(id) = mutation_id {
            let root = self
                .journal
                .as_ref()
                .ok_or_else(|| failed("mutation has no durable journal"))?;
            journal::wire(
                root,
                id,
                &serde_json::to_value(&command).map_err(|_| failed("command encoding failed"))?,
            )?;
        }
        let request = VesselRequest {
            protocol: VESSEL_API_VERSION,
            command,
        };
        let encoded =
            serde_json::to_vec(&request).map_err(|_| failed("Vessel request encoding failed"))?;
        if encoded.len() > MAX_VESSEL_BODY {
            return Err(invalid("Vessel request exceeds frame limit"));
        }
        let mut builder = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(encoded);
        if let Some(grant) = self.grant {
            builder = builder.header("x-voyage-grant", grant.to_string());
        }
        let mut response = builder.send().await.map_err(|_| {
            failed("Vessel connection failed; outcome unknown, never replay mutations")
        })?;
        if !response.status().is_success() {
            return Err(failed(&format!(
                "Vessel HTTP {}; outcome unknown, consult receipt",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failed("Vessel response interrupted; outcome unknown"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_VESSEL_BODY {
                return Err(failed(
                    "Vessel response exceeds frame limit; outcome unknown",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let reply: VesselResponse = serde_json::from_slice(&bytes)
            .map_err(|_| failed("invalid Vessel response; outcome unknown"))?;
        if reply.protocol != VESSEL_API_VERSION {
            return Err(failed("unsupported Vessel protocol; outcome unknown"));
        }
        // Server diagnostics may contain private paths or credentials; never return them.
        if let Some(error) = reply.error {
            let error = error.to_ascii_lowercase();
            let code = if error.contains("revision") {
                "stale_revision"
            } else if error.contains("denied")
                || error.contains("right")
                || error.contains("grant")
                || error.contains("permission")
            {
                "permission_denied"
            } else if error.contains("cleanup") || error.contains("reconcil") {
                "cleanup_pending"
            } else if error.contains("busy") || error.contains("active run") {
                "busy"
            } else if error.contains("not found") || error.contains("unknown session") {
                "not_found"
            } else if error.contains("unsupported") || error.contains("unavailable") {
                "unsupported_or_unavailable"
            } else if error.contains("expired") {
                "expired"
            } else {
                "refused"
            };
            return Ok(
                json!({"status": if reply.outcome_unknown {"outcome_unknown"} else {"refused"}, "code":code, "detail":"Inspect current state/capabilities or query receipt before deciding a new action; never replay uncertain effects"}),
            );
        }
        // These internal chunks are reassembled, decoded and scrubbed in read_text
        // before any model-facing result. Scrubbing here breaks byte continuations
        // and cannot catch a credential straddling two chunks.
        if text_chunk {
            return Ok(reply.result);
        }
        Ok(self.redact_complete(reply.result))
    }
}
