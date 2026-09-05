//! Dedicated single-owner enrollment client. Never opens or deletes session data.
//! Keep this object alive while connected: its advisory lock serializes processes.
//! Uncertain mutations remain pending and MUST be resumed, not replaced. Server
//! recovery currently lasts five minutes; after expiry operator repair is needed.
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use voyage_protocol::enrollment::{
    Challenge, MAX_PROOF_BYTES, ProofOperation, SignedChallenge, SigningKey,
};

type Result<T> = std::result::Result<T, ClientError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error("invalid enrollment origin")]
    Origin,
    #[error("private enrollment storage unavailable")]
    Storage,
    #[error("enrollment storage is already in use")]
    Busy,
    #[error("enrollment state conflict; resume pending transaction or contact operator")]
    Conflict,
    #[error("enrollment network outcome uncertain; resume original transaction")]
    Network,
    #[error("enrollment request denied; local state retained")]
    Denied,
    #[error("invalid enrollment response")]
    Response,
    #[error("enrollment cryptography failed")]
    Crypto,
    #[error("private enrollment storage unsupported on this platform")]
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Unenrolled,
    Enrolling,
    Active,
    Rotating,
    Revoking,
    Revoked,
    Detached,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
    pub revoked: bool,
}
// No Debug on any key-bearing or request type. Invitation secrets are never saved.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u16,
    origin: String,
    machine_id: Uuid,
    owner_id: Option<Uuid>,
    epoch: u64,
    status: Status,
    private_key: Vec<u8>,
    pending: Option<Pending>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    operation: ProofOperation,
    new_private_key: Option<Vec<u8>>,
}
#[derive(Serialize)]
struct ChallengeRequest<'a> {
    operation: &'a ProofOperation,
    invitation_key: Option<&'a str>,
}
#[derive(Serialize)]
struct CompleteRequest<'a> {
    proof: &'a SignedChallenge,
    invitation_key: Option<&'a str>,
}

pub fn validate_origin(origin: &str, allow_loopback_http: bool) -> Result<String> {
    if origin.len() > 2048 || origin.chars().any(char::is_control) || origin.trim() != origin {
        return Err(ClientError::Origin);
    }
    let url = Url::parse(origin).map_err(|_| ClientError::Origin)?;
    // Require canonical literal IP text: URL parsing otherwise accepts 127.1,
    // integer/hex IPv4 and other surprising spellings as loopback addresses.
    let authority = origin
        .split_once("://")
        .map(|(_, s)| s.split('/').next().unwrap_or(""))
        .unwrap_or("");
    let literal = if authority.starts_with('[') {
        authority
            .split_once(']')
            .and_then(|(s, _)| s[1..].parse::<std::net::Ipv6Addr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    } else {
        authority
            .split(':')
            .next()
            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback_http && literal))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || origin.contains('\\')
        || authority.contains('@')
    {
        return Err(ClientError::Origin);
    }
    Ok(url.origin().ascii_serialization())
}

pub struct EnrollmentClient {
    directory: PathBuf,
    _lock: File,
    http: Client,
    state: State,
    poisoned: bool,
}
impl EnrollmentClient {
    /// Explicit dedicated directory (not an existing Helm/session data root).
    /// Unix requires owned 0700 directory and owned 0600 regular files; other
    /// platforms fail closed until equivalent native ACL checks are implemented.
    pub fn open(directory: &Path, origin: &str, allow_loopback_http: bool) -> Result<Self> {
        let origin = validate_origin(origin, allow_loopback_http)?;
        let lock = lock_directory(directory)?;
        let path = directory.join("client.json");
        let state = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let mut bytes = Vec::new();
                private_file(&path, false)?
                    .take(65537)
                    .read_to_end(&mut bytes)
                    .map_err(|_| ClientError::Storage)?;
                if bytes.len() > 65536 {
                    return Err(ClientError::Storage);
                }
                let state: State =
                    serde_json::from_slice(&bytes).map_err(|_| ClientError::Storage)?;
                if state.version != 2 || state.origin != origin {
                    return Err(ClientError::Conflict);
                }
                validate_state(&state)?;
                state
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State {
                version: 2,
                origin,
                machine_id: Uuid::new_v4(),
                owner_id: None,
                epoch: 0,
                status: Status::Unenrolled,
                private_key: SigningKey::generate()
                    .map_err(|_| ClientError::Crypto)?
                    .as_pkcs8()
                    .to_vec(),
                pending: None,
            },
            Err(_) => return Err(ClientError::Storage),
        };
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| ClientError::Network)?;
        let mut client = Self {
            directory: directory.into(),
            _lock: lock,
            http,
            state,
            poisoned: false,
        };
        client.persist()?;
        Ok(client)
    }
    pub fn status(&self) -> Status {
        self.state.status
    }
    pub fn origin(&self) -> &str {
        &self.state.origin
    }
    pub fn machine_id(&self) -> Uuid {
        self.state.machine_id
    }
    pub fn epoch(&self) -> u64 {
        self.state.epoch
    }
    pub fn owner_id(&self) -> Option<Uuid> {
        self.state.owner_id
    }
    fn ready(&self) -> Result<()> {
        if self.poisoned {
            Err(ClientError::Storage)
        } else {
            Ok(())
        }
    }
    fn active(&self) -> Result<()> {
        self.ready()?;
        if self.state.status != Status::Active || self.state.pending.is_some() {
            Err(ClientError::Conflict)
        } else {
            Ok(())
        }
    }
    fn persist(&mut self) -> Result<()> {
        self.ready()?;
        let result = (|| {
            let bytes = serde_json::to_vec(&self.state).map_err(|_| ClientError::Storage)?;
            let mut temp = tempfile::NamedTempFile::new_in(&self.directory)
                .map_err(|_| ClientError::Storage)?;
            temp.write_all(&bytes)
                .and_then(|_| temp.as_file().sync_all())
                .map_err(|_| ClientError::Storage)?;
            temp.persist(self.directory.join("client.json"))
                .map_err(|_| ClientError::Storage)?;
            File::open(&self.directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| ClientError::Storage)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    /// Persists the machine key and original transaction BEFORE any redemption.
    /// An existing enrollment is never implicitly replaced, including detached ones.
    pub async fn enroll(&mut self, invitation_id: Uuid, invitation_key: &str) -> Result<Receipt> {
        self.ready()?;
        if self.state.status != Status::Unenrolled
            || invitation_id.is_nil()
            || invitation_key.len() != 43
        {
            return Err(ClientError::Conflict);
        }
        let public_key = self.key()?.public_key();
        self.state.pending = Some(Pending {
            operation: ProofOperation::Enroll {
                transaction_id: Uuid::new_v4(),
                invitation_id,
                machine_id: self.state.machine_id,
                public_key,
            },
            new_private_key: None,
        });
        self.state.status = Status::Enrolling;
        self.persist()?;
        self.resume(Some(invitation_key)).await
    }
    pub async fn rotate(&mut self) -> Result<Receipt> {
        self.active()?;
        if self.state.epoch >= i64::MAX as u64 - 1 {
            return Err(ClientError::Conflict);
        }
        let next = SigningKey::generate().map_err(|_| ClientError::Crypto)?;
        self.state.pending = Some(Pending {
            operation: ProofOperation::Rotate {
                machine_id: self.state.machine_id,
                epoch: self.state.epoch,
                transaction_id: Uuid::new_v4(),
                new_public_key: next.public_key(),
            },
            new_private_key: Some(next.as_pkcs8().to_vec()),
        });
        self.state.status = Status::Rotating;
        self.persist()?;
        self.resume(None).await
    }
    pub async fn revoke(&mut self) -> Result<Receipt> {
        self.active()?;
        self.state.pending = Some(Pending {
            operation: ProofOperation::Revoke {
                machine_id: self.state.machine_id,
                epoch: self.state.epoch,
                transaction_id: Uuid::new_v4(),
            },
            new_private_key: None,
        });
        self.state.status = Status::Revoking;
        self.persist()?;
        self.resume(None).await
    }
    /// Local detach is NOT server revocation. Retains identity/tombstone and never
    /// deletes user sessions. Resolve pending server mutations before detaching.
    pub fn detach(&mut self) -> Result<()> {
        self.ready()?;
        if self.state.pending.is_some() {
            return Err(ClientError::Conflict);
        }
        self.state.status = Status::Detached;
        self.persist()
    }
    /// Fresh challenge for the ORIGINAL operation; never retry a consumed proof.
    /// On every error both old and proposed keys remain available on disk.
    pub async fn resume(&mut self, invitation_key: Option<&str>) -> Result<Receipt> {
        self.ready()?;
        let pending = self.state.pending.as_ref().ok_or(ClientError::Conflict)?;
        if !matches!(pending.operation, ProofOperation::Enroll { .. }) && invitation_key.is_some() {
            return Err(ClientError::Conflict);
        }
        let proof = self
            .proof(
                &pending.operation,
                invitation_key,
                pending.new_private_key.as_deref(),
            )
            .await?;
        let receipt: Receipt = self
            .post(
                "complete",
                &CompleteRequest {
                    proof: &proof,
                    invitation_key,
                },
            )
            .await?;
        self.accept_receipt(&receipt)?;
        Ok(receipt)
    }
    /// Return the proof directly to the WebSocket handshake. DO NOT POST this
    /// proof to /complete: that consumes it and makes the socket handshake replay.
    pub async fn connect_proof(&self) -> Result<SignedChallenge> {
        self.active()?;
        self.proof(
            &ProofOperation::Connect {
                machine_id: self.state.machine_id,
                epoch: self.state.epoch,
            },
            None,
            None,
        )
        .await
    }
    fn key(&self) -> Result<SigningKey> {
        SigningKey::from_pkcs8(&self.state.private_key).map_err(|_| ClientError::Crypto)
    }
    async fn proof(
        &self,
        operation: &ProofOperation,
        invitation_key: Option<&str>,
        next: Option<&[u8]>,
    ) -> Result<SignedChallenge> {
        let challenge: Challenge = self
            .post(
                "challenge",
                &ChallengeRequest {
                    operation,
                    invitation_key,
                },
            )
            .await?;
        self.sign_challenge(challenge, operation, next)
    }
    fn sign_challenge(
        &self,
        challenge: Challenge,
        operation: &ProofOperation,
        next: Option<&[u8]>,
    ) -> Result<SignedChallenge> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .ok_or(ClientError::Response)?;
        challenge
            .validate(&self.state.origin, now)
            .map_err(|_| ClientError::Response)?;
        if &challenge.operation != operation {
            return Err(ClientError::Response);
        }
        let signature = self
            .key()?
            .sign(&challenge)
            .map_err(|_| ClientError::Crypto)?;
        let new_signature = next
            .map(|bytes| {
                SigningKey::from_pkcs8(bytes)
                    .and_then(|key| key.sign(&challenge))
                    .map_err(|_| ClientError::Crypto)
            })
            .transpose()?;
        Ok(SignedChallenge {
            challenge,
            signature,
            new_signature,
        })
    }
    fn accept_receipt(&mut self, receipt: &Receipt) -> Result<()> {
        let pending = self.state.pending.as_ref().ok_or(ClientError::Conflict)?;
        let (epoch, revoked) = match pending.operation {
            ProofOperation::Enroll { .. } => (1, false),
            ProofOperation::Rotate { epoch, .. } => (epoch + 1, false),
            ProofOperation::Revoke { epoch, .. } => (epoch + 1, true),
            _ => return Err(ClientError::Conflict),
        };
        if receipt.machine_id != self.state.machine_id
            || receipt.owner_id.is_nil()
            || self.state.owner_id.is_some_and(|id| id != receipt.owner_id)
            || receipt.epoch != epoch
            || receipt.revoked != revoked
        {
            return Err(ClientError::Response);
        }
        let pending = self.state.pending.take().ok_or(ClientError::Conflict)?;
        if let Some(key) = pending.new_private_key {
            self.state.private_key = key;
        }
        self.state.epoch = epoch;
        self.state.owner_id = Some(receipt.owner_id);
        self.state.status = if revoked {
            Status::Revoked
        } else {
            Status::Active
        };
        self.persist()
    }
    async fn post<T: DeserializeOwned>(&self, endpoint: &str, body: &impl Serialize) -> Result<T> {
        let bytes = serde_json::to_vec(body).map_err(|_| ClientError::Response)?;
        if bytes.len() > MAX_PROOF_BYTES {
            return Err(ClientError::Response);
        }
        let mut response = self
            .http
            .post(format!("{}/v2/enrollment/{}", self.state.origin, endpoint))
            .header("x-voyage-request", "2")
            .header("content-type", "application/json")
            .header("origin", &self.state.origin)
            .body(bytes)
            .send()
            .await
            .map_err(|_| ClientError::Network)?;
        if !response.status().is_success() {
            return Err(if response.status().is_server_error() {
                ClientError::Network
            } else {
                ClientError::Denied
            });
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_PROOF_BYTES as u64)
        {
            return Err(ClientError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| ClientError::Network)? {
            if bytes.len() + chunk.len() > MAX_PROOF_BYTES {
                return Err(ClientError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| ClientError::Response)
    }
}

fn validate_state(s: &State) -> Result<()> {
    let invalid = || ClientError::Storage;
    let key = SigningKey::from_pkcs8(&s.private_key).map_err(|_| invalid())?;
    if s.machine_id.is_nil()
        || s.owner_id.is_some_and(|id| id.is_nil())
        || s.epoch >= i64::MAX as u64
    {
        return Err(invalid());
    }
    let enrolled = s.epoch > 0 && s.owner_id.is_some();
    let virgin = s.epoch == 0 && s.owner_id.is_none();
    match (&s.status, &s.pending) {
        (Status::Unenrolled, None) if virgin => (),
        (Status::Active | Status::Revoked, None) if enrolled => (),
        (Status::Detached, None) if enrolled || virgin => (),
        (
            Status::Enrolling,
            Some(Pending {
                operation:
                    ProofOperation::Enroll {
                        transaction_id,
                        invitation_id,
                        machine_id,
                        public_key,
                    },
                new_private_key: None,
            }),
        ) if virgin
            && !transaction_id.is_nil()
            && !invitation_id.is_nil()
            && *machine_id == s.machine_id
            && *public_key == key.public_key() => {}
        (
            Status::Rotating,
            Some(Pending {
                operation:
                    ProofOperation::Rotate {
                        transaction_id,
                        machine_id,
                        epoch,
                        new_public_key,
                    },
                new_private_key: Some(next),
            }),
        ) if enrolled
            && !transaction_id.is_nil()
            && *machine_id == s.machine_id
            && *epoch == s.epoch
            && *epoch < i64::MAX as u64 - 1 =>
        {
            let next = SigningKey::from_pkcs8(next).map_err(|_| invalid())?;
            if next.public_key() != *new_public_key || next.public_key() == key.public_key() {
                return Err(invalid());
            }
        }
        (
            Status::Revoking,
            Some(Pending {
                operation:
                    ProofOperation::Revoke {
                        transaction_id,
                        machine_id,
                        epoch,
                    },
                new_private_key: None,
            }),
        ) if enrolled
            && !transaction_id.is_nil()
            && *machine_id == s.machine_id
            && *epoch == s.epoch => {}
        _ => return Err(invalid()),
    }
    Ok(())
}
#[cfg(unix)]
fn check_private(metadata: &fs::Metadata, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        Err(ClientError::Storage)
    } else {
        Ok(())
    }
}
#[cfg(unix)]
fn private_file(path: &Path, create: bool) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Ok(m) = fs::symlink_metadata(path) {
        check_private(&m, false)?;
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| ClientError::Storage)?;
    check_private(&file.metadata().map_err(|_| ClientError::Storage)?, false)?;
    Ok(file)
}
#[cfg(unix)]
fn lock_directory(directory: &Path) -> Result<File> {
    use std::os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    // Reject symlink ancestors too. The parent must be provisioned by the caller.
    let absolute = if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| ClientError::Storage)?
            .join(directory)
    };
    for ancestor in absolute.ancestors().skip(1) {
        if fs::symlink_metadata(ancestor)
            .map_err(|_| ClientError::Storage)?
            .file_type()
            .is_symlink()
        {
            return Err(ClientError::Storage);
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(directory) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(ClientError::Storage),
    }
    check_private(
        &fs::symlink_metadata(directory).map_err(|_| ClientError::Storage)?,
        true,
    )?;
    let dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(directory)
        .map_err(|_| ClientError::Storage)?;
    check_private(&dir.metadata().map_err(|_| ClientError::Storage)?, true)?;
    let lock = private_file(&directory.join("client.lock"), true)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(ClientError::Busy);
    }
    dir.sync_all().map_err(|_| ClientError::Storage)?;
    File::open(absolute.parent().ok_or(ClientError::Storage)?)
        .and_then(|f| f.sync_all())
        .map_err(|_| ClientError::Storage)?;
    Ok(lock)
}
#[cfg(not(unix))]
fn lock_directory(_: &Path) -> Result<File> {
    Err(ClientError::Unsupported)
}
#[cfg(not(unix))]
fn private_file(_: &Path, _: bool) -> Result<File> {
    Err(ClientError::Unsupported)
}

#[cfg(test)]
#[cfg(unix)]
mod tests;
