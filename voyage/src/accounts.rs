//! Execution-host private accounts. Callers enforce account-use authority separately.
//! All mutations are atomic, bounded transactions; no network effects hold the registry lock.
use crate::attachment::local_actor::storage::Directory;
use crate::provider::ChatGptTokenStore;
pub use crate::provider::chatgpt_oauth::OAuthTokens;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use uuid::Uuid;
pub use voyage_protocol::accounts::*;
pub mod device;
pub mod usage;

const LIMIT: usize = 65_536;
#[derive(Clone)]
pub struct Registry {
    root: PathBuf,
}
/// Private input only. Do not serialize this into commands or model-visible input.
pub enum ApiKeyInput {
    Stored(String),
    Environment(String),
}
#[derive(Clone, Serialize, Deserialize)]
enum Credential {
    Stored(String),
    Environment { name: String, digest: String },
    OAuth(OAuthTokens),
    None,
}
#[derive(Clone, Serialize, Deserialize)]
struct Account {
    descriptor: AccountDescriptor,
    credential: Credential,
    refresh: Option<Uuid>,
    attested: bool,
    provider_identity: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
struct Database {
    revision: u64,
    #[serde(default)]
    default_account: Option<AccountBinding>,
    #[serde(default)]
    default_revision: u64,
    #[serde(default)]
    default_commands: std::collections::BTreeMap<Uuid, (String, u64, AccountBinding, u64)>,
    #[serde(default)]
    usage: std::collections::BTreeMap<Uuid, AccountUsageObservation>,
    connections: Vec<ConnectionDescriptor>,
    accounts: Vec<Account>,
    enrollments: Vec<device::Record>,
    legacy: Option<Uuid>,
    #[serde(default)]
    legacy_binding: Option<AccountBinding>,
    #[serde(default)]
    legacy_api: std::collections::BTreeMap<String, AccountBinding>,
}
impl Registry {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn default_host() -> Result<Self> {
        let root = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("private data directory unavailable"))?
            .join("helm");
        // Reuse the credential store's ancestor creation/verification boundary.
        crate::provider::chatgpt_oauth::storage::prepare(&root)?;
        Ok(Self::new(root.join("accounts")))
    }
    fn transaction<T>(&self, f: impl FnOnce(&mut Database) -> Result<T>) -> Result<T> {
        let directory = Directory::open(&self.root)
            .map_err(|_| anyhow::anyhow!("private account storage unavailable"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(750);
        let _lock = loop {
            match directory.lock() {
                Ok(lock) => break lock,
                Err(error)
                    if error.chain().any(|e| {
                        e.downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
                    }) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(_) => bail!("private account storage busy or unavailable; retry"),
            }
        };
        let mut db: Database = match directory.read_bounded("registry.json", LIMIT)? {
            Some(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid private account registry"))?,
            None => Database::default(),
        };
        // A manually damaged private record must not become terminal control content.
        ensure!(
            db.accounts.len() <= 64 && db.connections.len() <= 32 && db.enrollments.len() <= 64,
            "account registry exceeds bounds"
        );
        for a in &db.accounts {
            text(&a.descriptor.alias)?;
            text(&a.descriptor.label)?;
        }
        for c in &db.connections {
            text(&c.label)?;
            ensure!(
                c.endpoint.len() <= 2048 && c.endpoint.bytes().all(|b| b.is_ascii_graphic()),
                "invalid stored endpoint"
            );
        }
        let before = serde_json::to_vec(&db)?;
        let result = f(&mut db)?;
        ensure!(
            db.accounts.len() <= 64 && db.connections.len() <= 32 && db.enrollments.len() <= 64,
            "account registry capacity reached"
        );
        if serde_json::to_vec(&db)? != before {
            db.revision = db
                .revision
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("registry revision exhausted"))?;
            let bytes = serde_json::to_vec(&db)?;
            ensure!(bytes.len() <= LIMIT, "account registry capacity reached");
            directory.publish("registry.json", &bytes)?;
        }
        Ok(result)
    }
    pub(crate) fn legacy_store_binding() -> Result<Option<(Self, AccountBinding)>> {
        let root = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("private data directory unavailable"))?
            .join("helm")
            .join("accounts");
        match std::fs::symlink_metadata(&root) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => bail!("private registry unavailable"),
            Ok(_) => {}
        }
        let registry = Self::new(root);
        let binding = registry.transaction(|db| {
            let Some(id) = db.legacy else {
                return Ok(None);
            };
            let a = db
                .accounts
                .iter()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("legacy binding unavailable"))?;
            let c = db
                .connections
                .iter()
                .find(|c| c.id == a.descriptor.connection_id)
                .ok_or_else(|| anyhow::anyhow!("legacy connection unavailable"))?;
            // Legacy references retain the original migration identity, never
            // silently adopt a later replacement at the same profile UUID.
            Ok(Some(db.legacy_binding.clone().unwrap_or(AccountBinding {
                account_id: id,
                connection_id: c.id,
                identity_generation: 1,
                connection_revision: 1,
                transport: Transport::ChatgptOauth,
            })))
        })?;
        Ok(binding.map(|b| (registry, b)))
    }
    pub(crate) fn legacy_logged_out(&self, binding: &AccountBinding) -> Result<bool> {
        self.transaction(|db| {
            let a = db
                .accounts
                .iter()
                .find(|a| a.descriptor.id == binding.account_id)
                .ok_or_else(|| anyhow::anyhow!("account unavailable"))?;
            ensure!(
                a.descriptor.identity_generation == binding.identity_generation
                    && a.descriptor.state != AccountState::Removed,
                "account binding revoked"
            );
            Ok(a.descriptor.state == AccountState::SignInRequired)
        })
    }
    pub fn add_connection(
        &self,
        label: String,
        endpoint: String,
        transports: Vec<Transport>,
    ) -> Result<ConnectionDescriptor> {
        text(&label)?;
        ensure!(
            endpoint.bytes().all(|b| b.is_ascii_graphic()),
            "endpoint must be printable ASCII without whitespace"
        );
        crate::provider::validate_native_endpoint(&endpoint)?;
        let url =
            reqwest::Url::parse(&endpoint).map_err(|_| anyhow::anyhow!("invalid endpoint"))?;
        ensure!(
            url.query().is_none(),
            "connection endpoint must not include query credentials"
        );
        ensure!(
            !transports.is_empty() && transports.len() <= 4 && endpoint.len() <= 2048,
            "invalid connection"
        );
        self.transaction(|db| {
            ensure!(db.connections.len() < 32, "connection capacity reached");
            let connection = ConnectionDescriptor {
                id: Uuid::new_v4(),
                revision: 1,
                label,
                endpoint,
                transports,
            };
            db.connections.push(connection.clone());
            Ok(connection)
        })
    }
    /// Metadata-only, stable built-in enrollment destination. No credential effects.
    pub fn ensure_chatgpt_connection(&self) -> Result<ConnectionDescriptor> {
        self.transaction(|db| {
            let id = Uuid::from_u128(0x6b8a43c98b3d4d178f00798f37f66213);
            if let Some(connection) = db.connections.iter().find(|c| c.id == id) {
                ensure!(
                    connection.endpoint == "https://chatgpt.com/backend-api/codex"
                        && connection.transports == vec![Transport::ChatgptOauth],
                    "built-in connection conflict"
                );
                return Ok(connection.clone());
            }
            ensure!(db.connections.len() < 32, "connection capacity reached");
            let connection = ConnectionDescriptor {
                id,
                revision: 1,
                label: "ChatGPT".into(),
                endpoint: "https://chatgpt.com/backend-api/codex".into(),
                transports: vec![Transport::ChatgptOauth],
            };
            db.connections.push(connection.clone());
            Ok(connection)
        })
    }
    pub fn connections(&self) -> Result<Vec<ConnectionDescriptor>> {
        self.transaction(|db| Ok(db.connections.clone()))
    }
    /// Private host-owner import. Returned tokens/identity must never enter public history.
    pub fn add_oauth(
        &self,
        connection: Uuid,
        alias: String,
        label: String,
        tokens: OAuthTokens,
    ) -> Result<AccountDescriptor> {
        crate::provider::chatgpt_oauth::validate_tokens(&tokens)?;
        self.transaction(|db| {
            ensure!(
                db.connections
                    .iter()
                    .any(|c| c.id == connection && c.transports == [Transport::ChatgptOauth]),
                "not an OAuth connection"
            );
            insert(
                db,
                connection,
                alias,
                label,
                Credential::OAuth(tokens),
                None,
            )
        })
    }
    /// Host-owner recovery only; ordinary enrollment rights never authorize this operation.
    /// Same-login recovery preserves generation; explicit different-login replacement advances it.
    pub fn reauthenticate_oauth(
        &self,
        id: Uuid,
        expected_generation: u64,
        tokens: OAuthTokens,
        allow_identity_replacement: bool,
    ) -> Result<AccountDescriptor> {
        crate::provider::chatgpt_oauth::validate_tokens(&tokens)?;
        self.transaction(|db| {
            let a = db
                .accounts
                .iter_mut()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown account"))?;
            ensure!(
                a.descriptor.identity_generation == expected_generation
                    && a.descriptor.state != AccountState::Removed,
                "stale or removed account"
            );
            ensure!(
                db.connections
                    .iter()
                    .any(|c| c.id == a.descriptor.connection_id
                        && c.transports == [Transport::ChatgptOauth]),
                "not an OAuth account"
            );
            let identity = crate::provider::chatgpt_oauth::login_identity(&tokens)?;
            if identity.is_none() || a.provider_identity != identity {
                ensure!(
                    allow_identity_replacement,
                    "different OAuth identity requires explicit replacement"
                );
                a.descriptor.identity_generation += 1;
            }
            a.provider_identity = identity;
            a.credential = Credential::OAuth(tokens);
            a.refresh = None;
            a.descriptor.state = AccountState::Ready;
            a.descriptor.credential_revision += 1;
            a.descriptor.capability_revision += 1;
            Ok(describe(a))
        })
    }
    /// Native OAuth endpoint-safe factory; custom subscription endpoints are not supported.
    pub fn oauth_provider(
        &self,
        binding: &AccountBinding,
    ) -> Result<crate::provider::ChatGptOauthProvider> {
        let c = self.connection(binding.connection_id)?;
        ensure!(
            c.endpoint == "https://chatgpt.com/backend-api/codex",
            "unsupported OAuth endpoint"
        );
        Ok(crate::provider::ChatGptOauthProvider::from_store(
            self.token_store(binding)?,
            crate::provider::OAuthEndpoints::default(),
        ))
    }
    /// Filter BEFORE returning descriptors to a caller. This function grants no authority.
    pub fn list(
        &self,
        allowed: impl Fn(&AccountDescriptor) -> bool,
    ) -> Result<(u64, Vec<AccountDescriptor>)> {
        self.transaction(|db| {
            Ok((
                db.revision,
                db.accounts
                    .iter()
                    .filter(|a| allowed(&a.descriptor))
                    .map(describe)
                    .collect(),
            ))
        })
    }
    /// Host-internal provenance for grant filtering. This is not a grant of account-use rights.
    pub fn enrollment_actor(&self, account: Uuid) -> Result<Option<EnrollmentActor>> {
        self.transaction(|db| Ok(device::enrollment_actor(db, account)))
    }
    pub fn connection(&self, id: Uuid) -> Result<ConnectionDescriptor> {
        self.transaction(|db| {
            db.connections
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown connection"))
        })
    }
    pub fn freeze(&self, id: Uuid, transport: Transport) -> Result<AccountBinding> {
        self.transaction(|db| {
            let a = db
                .accounts
                .iter()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("account unavailable"))?;
            let c = db
                .connections
                .iter()
                .find(|c| c.id == a.descriptor.connection_id)
                .ok_or_else(|| anyhow::anyhow!("connection unavailable"))?;
            let b = AccountBinding {
                account_id: id,
                connection_id: c.id,
                identity_generation: a.descriptor.identity_generation,
                connection_revision: c.revision,
                transport,
            };
            checked(db, &b)?;
            Ok(b)
        })
    }
    pub fn validate_binding(&self, binding: &AccountBinding) -> Result<AccountDescriptor> {
        self.transaction(|db| Ok(describe(checked(db, binding)?)))
    }
    pub fn add_api(
        &self,
        connection: Uuid,
        alias: String,
        label: String,
        input: ApiKeyInput,
    ) -> Result<AccountDescriptor> {
        let credential = api_input(input)?;
        self.transaction(|db| {
            let c = db
                .connections
                .iter()
                .find(|c| c.id == connection)
                .ok_or_else(|| anyhow::anyhow!("unknown connection"))?;
            ensure!(
                !c.transports.contains(&Transport::ChatgptOauth),
                "OAuth connection cannot accept API credentials"
            );
            insert(db, connection, alias, label, credential, None)
        })
    }
    /// Synchronous Config::api_key seam. Recheck on every dispatch/retry/discovery.
    pub fn resolve_api_key(&self, binding: &AccountBinding) -> Result<String> {
        self.transaction(|db| match &checked(db, binding)?.credential {
            Credential::Stored(key) => Ok(key.clone()),
            Credential::Environment { name, digest } => {
                let key = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "bound environment key unavailable; supervisor restart may be needed"
                    )
                })?;
                ensure!(
                    fingerprint(&key) == *digest,
                    "environment credential changed; explicit rotation or replacement required"
                );
                Ok(key)
            }
            _ => bail!("account is not an API binding"),
        })
    }
    pub fn rotate_api(
        &self,
        binding: &AccountBinding,
        input: ApiKeyInput,
        owner_attests_same_identity: bool,
    ) -> Result<AccountDescriptor> {
        let credential = api_input(input)?;
        self.transaction(|db| {
            let a = checked(db, binding)?;
            ensure!(
                matches!(
                    a.credential,
                    Credential::Stored(_) | Credential::Environment { .. }
                ),
                "not an API account"
            );
            a.credential = credential;
            a.attested = owner_attests_same_identity;
            a.descriptor.credential_revision += 1;
            a.descriptor.capability_revision += 1;
            if !owner_attests_same_identity {
                a.descriptor.identity_generation += 1;
            }
            Ok(describe(a))
        })
    }
    /// Host-owner recovery/rotation, including a logged-out API account. Removed UUIDs stay removed.
    pub fn reauthenticate_api(
        &self,
        id: Uuid,
        expected_generation: u64,
        input: ApiKeyInput,
        owner_attests_same_identity: bool,
    ) -> Result<AccountDescriptor> {
        let credential = api_input(input)?;
        self.transaction(|db| {
            let a = db
                .accounts
                .iter_mut()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown account"))?;
            ensure!(
                a.descriptor.identity_generation == expected_generation
                    && a.descriptor.state != AccountState::Removed,
                "stale or removed account"
            );
            ensure!(
                db.connections
                    .iter()
                    .any(|c| c.id == a.descriptor.connection_id
                        && !c.transports.contains(&Transport::ChatgptOauth)),
                "not an API account"
            );
            if !owner_attests_same_identity {
                a.descriptor.identity_generation += 1;
            }
            a.credential = credential;
            a.attested = owner_attests_same_identity;
            a.refresh = None;
            a.descriptor.state = AccountState::Ready;
            a.descriptor.credential_revision += 1;
            a.descriptor.capability_revision += 1;
            Ok(describe(a))
        })
    }
    pub fn rename(&self, id: Uuid, alias: String, label: String) -> Result<()> {
        text(&alias)?;
        text(&label)?;
        self.transaction(|db| {
            ensure!(
                !db.accounts
                    .iter()
                    .any(|a| a.descriptor.id != id && a.descriptor.alias == alias)
                    && !device::alias_reserved(db, &alias, None),
                "alias exists or is reserved"
            );
            let a = db
                .accounts
                .iter_mut()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown account"))?;
            a.descriptor.alias = alias;
            a.descriptor.label = label;
            a.descriptor.metadata_revision += 1;
            Ok(())
        })
    }
    /// Tombstones locally; cannot recall dispatched requests or revoke upstream tokens.
    pub fn logout(&self, id: Uuid, remove: bool) -> Result<()> {
        self.transaction(|db| {
            ensure!(
                !remove
                    || !db
                        .default_account
                        .as_ref()
                        .is_some_and(|b| b.account_id == id),
                "Choose a replacement default account before removing this account"
            );
            let a = db
                .accounts
                .iter_mut()
                .find(|a| a.descriptor.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown account"))?;
            // Removal is absorbing: a later ordinary logout cannot make this UUID recoverable.
            if a.descriptor.state == AccountState::Removed {
                return Ok(());
            }
            a.descriptor.identity_generation += 1;
            a.descriptor.credential_revision += 1;
            a.descriptor.capability_revision += 1;
            a.descriptor.state = if remove {
                AccountState::Removed
            } else {
                AccountState::SignInRequired
            };
            a.credential = Credential::None;
            a.refresh = None;
            Ok(())
        })
    }
    pub fn token_store(&self, binding: &AccountBinding) -> Result<ChatGptTokenStore> {
        self.validate_binding(binding)?;
        ensure!(
            binding.transport == Transport::ChatgptOauth,
            "not an OAuth binding"
        );
        Ok(ChatGptTokenStore::bound(self.clone(), binding.clone()))
    }
    pub(crate) fn oauth_load(&self, binding: &AccountBinding) -> Result<OAuthTokens> {
        self.transaction(|db| {
            let a = checked(db, binding)?;
            if a.refresh.is_some() {
                return Err(crate::provider::chatgpt_oauth::RefreshPending.into());
            }
            match &a.credential {
                Credential::OAuth(t) => Ok(t.clone()),
                _ => bail!("OAuth login required"),
            }
        })
    }
    pub(crate) fn refresh_begin(
        &self,
        binding: &AccountBinding,
        expected: &OAuthTokens,
    ) -> Result<Uuid> {
        self.transaction(|db| {
            let a = checked(db, binding)?;
            ensure!(a.refresh.is_none(), "OAuth refresh busy or uncertain");
            ensure!(
                matches!(&a.credential, Credential::OAuth(t) if t == expected),
                "OAuth credentials changed; retry resolution"
            );
            let id = Uuid::new_v4();
            a.refresh = Some(id);
            Ok(id)
        })
    }
    pub(crate) fn oauth_save(
        &self,
        binding: &AccountBinding,
        tokens: &OAuthTokens,
        fence: Option<Uuid>,
    ) -> Result<()> {
        self.transaction(|db| {
            let a = checked(db, binding)?;
            ensure!(a.refresh == fence, "stale OAuth refresh fence");
            let identity = crate::provider::chatgpt_oauth::login_identity(tokens)?;
            ensure!(
                identity.is_some() && a.provider_identity == identity,
                "OAuth identity unavailable or changed; explicit replacement required"
            );
            a.credential = Credential::OAuth(tokens.clone());
            a.refresh = None;
            a.descriptor.credential_revision += 1;
            if fence.is_none() {
                a.descriptor.capability_revision += 1;
            }
            Ok(())
        })
    }
    /// Materialize a trusted legacy native environment configuration exactly once.
    /// Returns the original identity on later calls, never silently retargeting saved configurations.
    pub fn migrate_legacy_api(
        &self,
        endpoint: String,
        transport: Transport,
        environment: String,
    ) -> Result<AccountBinding> {
        ensure!(
            transport != Transport::ChatgptOauth,
            "OAuth is not an API-key migration"
        );
        ensure!(
            endpoint.bytes().all(|b| b.is_ascii_graphic()),
            "endpoint must be printable ASCII without whitespace"
        );
        crate::provider::validate_native_endpoint(&endpoint)?;
        let url =
            reqwest::Url::parse(&endpoint).map_err(|_| anyhow::anyhow!("invalid endpoint"))?;
        ensure!(
            url.query().is_none() && endpoint.len() <= 2048,
            "legacy endpoint requires bounded credential-free configuration"
        );
        let identity = fingerprint(&serde_json::to_string(&(
            &endpoint,
            transport,
            &environment,
        ))?);
        self.transaction(|db| {
            if let Some(binding) = db.legacy_api.get(&identity) {
                return Ok(binding.clone());
            }
            let credential = api_input(ApiKeyInput::Environment(environment))?;
            ensure!(
                db.connections.len() < 32 && db.legacy_api.len() < 32,
                "legacy binding capacity reached"
            );
            let c = ConnectionDescriptor {
                id: Uuid::new_v4(),
                revision: 1,
                label: "Legacy native API".into(),
                endpoint,
                transports: vec![transport],
            };
            db.connections.push(c.clone());
            let a = insert(
                db,
                c.id,
                format!("legacy-api-{}", &identity[..16]),
                "Existing native API binding".into(),
                credential,
                None,
            )?;
            let binding = AccountBinding {
                account_id: a.id,
                connection_id: c.id,
                identity_generation: a.identity_generation,
                connection_revision: c.revision,
                transport,
            };
            db.legacy_api.insert(identity, binding.clone());
            Ok(binding)
        })
    }
    /// Explicit upgrade boundary: all old writers MUST be stopped by the owner.
    /// The original cache is retained, never deleted; a recorded migration never rereads it.
    pub fn migrate_legacy_oauth(&self, old_writers_stopped: bool) -> Result<Option<Uuid>> {
        ensure!(
            old_writers_stopped,
            "stop old binaries before account migration"
        );
        let path = ChatGptTokenStore::default_path()?;
        self.transaction(|db| {
            if let Some(id) = db.legacy {
                return Ok(Some(id));
            }
            let bytes = crate::provider::chatgpt_oauth::storage::read_tokens(&path)?;
            let tokens: Option<OAuthTokens> = match bytes {
                Some(b) => serde_json::from_slice(&b)
                    .map_err(|_| anyhow::anyhow!("invalid legacy cache"))?,
                None => None,
            };
            if let Some(tokens) = &tokens {
                crate::provider::chatgpt_oauth::validate_tokens(tokens)?;
            }
            let connection = ConnectionDescriptor {
                id: Uuid::new_v4(),
                revision: 1,
                label: "ChatGPT".into(),
                endpoint: "https://chatgpt.com/backend-api/codex".into(),
                transports: vec![Transport::ChatgptOauth],
            };
            db.connections.push(connection.clone());
            let a = insert(
                db,
                connection.id,
                "legacy-chatgpt".into(),
                "Existing ChatGPT login".into(),
                tokens.map(Credential::OAuth).unwrap_or(Credential::None),
                None,
            )?;
            db.legacy = Some(a.id);
            db.legacy_binding = Some(AccountBinding {
                account_id: a.id,
                connection_id: connection.id,
                identity_generation: a.identity_generation,
                connection_revision: connection.revision,
                transport: Transport::ChatgptOauth,
            });
            Ok(Some(a.id))
        })
    }
}
fn checked<'a>(db: &'a mut Database, b: &AccountBinding) -> Result<&'a mut Account> {
    ensure!(
        db.connections.iter().any(|c| c.id == b.connection_id
            && c.revision == b.connection_revision
            && c.transports.contains(&b.transport)),
        "connection binding unavailable or changed"
    );
    let a = db
        .accounts
        .iter_mut()
        .find(|a| a.descriptor.id == b.account_id)
        .ok_or_else(|| anyhow::anyhow!("unknown account"))?;
    ensure!(
        a.descriptor.connection_id == b.connection_id
            && a.descriptor.identity_generation == b.identity_generation
            && a.descriptor.state == AccountState::Ready,
        "account binding revoked or unavailable"
    );
    Ok(a)
}
fn text(s: &str) -> Result<()> {
    ensure!(
        !s.trim().is_empty() && s.len() <= 96 && s.chars().all(|c| c.is_ascii() && !c.is_control()),
        "label must be bounded printable ASCII"
    );
    Ok(())
}
fn fingerprint(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}
fn api_input(input: ApiKeyInput) -> Result<Credential> {
    let (key, name) = match input {
        ApiKeyInput::Stored(key) => (key, None),
        ApiKeyInput::Environment(name) => {
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && !name.as_bytes()[0].is_ascii_digit(),
                "invalid environment name"
            );
            (
                std::env::var(&name).map_err(|_| anyhow::anyhow!("environment key unavailable"))?,
                Some(name),
            )
        }
    };
    ensure!(
        !key.is_empty() && key.len() <= 8192 && !key.chars().any(char::is_control),
        "invalid API credential"
    );
    Ok(match name {
        Some(name) => Credential::Environment {
            name,
            digest: fingerprint(&key),
        },
        None => Credential::Stored(key),
    })
}
fn insert(
    db: &mut Database,
    connection_id: Uuid,
    alias: String,
    label: String,
    credential: Credential,
    enrollment: Option<Uuid>,
) -> Result<AccountDescriptor> {
    text(&alias)?;
    text(&label)?;
    ensure!(db.accounts.len() < 64, "account capacity reached");
    ensure!(
        !db.accounts.iter().any(|a| a.descriptor.alias == alias)
            && !device::alias_reserved(db, &alias, enrollment),
        "alias already exists or is reserved"
    );
    let descriptor = AccountDescriptor {
        id: Uuid::new_v4(),
        connection_id,
        alias,
        label,
        metadata_revision: 1,
        identity_generation: 1,
        credential_revision: 1,
        capability_revision: 1,
        availability: CredentialAvailability::Available,
        state: if matches!(credential, Credential::None) {
            AccountState::SignInRequired
        } else {
            AccountState::Ready
        },
    };
    let provider_identity = match &credential {
        Credential::OAuth(t) => crate::provider::chatgpt_oauth::login_identity(t)?,
        _ => None,
    };
    let account = Account {
        descriptor,
        credential,
        refresh: None,
        attested: false,
        provider_identity,
    };
    let safe = describe(&account);
    db.accounts.push(account);
    Ok(safe)
}

/// Private execution-host terminal input; not a remote credential entry surface.
pub mod private_input;

fn describe(a: &Account) -> AccountDescriptor {
    let mut descriptor = a.descriptor.clone();
    descriptor.availability = if a.refresh.is_some() {
        CredentialAvailability::RefreshPendingOrUncertain
    } else {
        match &a.credential {
            Credential::None => CredentialAvailability::Missing,
            Credential::Stored(_) => CredentialAvailability::Available,
            Credential::Environment { name, digest } => match std::env::var(name) {
                Err(_) => CredentialAvailability::EnvironmentUnavailable,
                Ok(value) if fingerprint(&value) != *digest => {
                    CredentialAvailability::EnvironmentChanged
                }
                Ok(_) => CredentialAvailability::Available,
            },
            Credential::OAuth(tokens) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                if tokens.expires_at <= now {
                    CredentialAvailability::Expired
                } else {
                    CredentialAvailability::Available
                }
            }
        }
    };
    descriptor
}

#[cfg(test)]
mod tests;
