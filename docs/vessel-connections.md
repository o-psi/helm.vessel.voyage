# Connecting to Vessels inside Helm

Helm's **Vessels** panel manages human connections independently of conversation
text and model-driven coordination. A connection identifies where work executes;
it is not a conversation, a provider account, or permission to run as root.
[Linux verification](vessel-connections-verification.md) records the exercised
workflows and deployment limits.

## Open and use the manager

Start Helm normally. Click **Vessels**, press **Ctrl+G**, or enter `/vessels`.
The control remains available in narrow layouts where the conversation sidebar
is hidden. The panel supports keyboard selection and mouse input; Escape returns
to the original conversation or draft.

The panel provides:

- **Add HTTPS + pairing** for an owner-approved workspace connection.
- **Import existing access file** for an existing credential, including restricted
  conversation sharing.
- Connect/Retry, Disconnect, friendly-name changes and automatic connection at startup.
- Renew/Replace access, Forget, and recovery of retained connection records.
- New conversation on the selected Vessel and filters for the conversation list.

Saved connections are loaded by normal interactive Helm startup. Remote discovery
runs in the background; an unavailable remote does not prevent local work. Explicit
CLI subcommands remain single-target and do not implicitly fan out to saved Vessels.

### Unavailable Vessels

A failed catalogue check (including its three-second observation timeout) marks
that Vessel **Unavailable** and pauses its observer and background receipt/history
checks. Helm does not repeatedly poll it or put transport diagnostics in the shared
footer. Other Vessels and local draft editing remain usable. An unavailable Vessel
is not evidence that its voyages stopped, finished, or completed cleanup.

Open the centered **Vessels** modal with **Ctrl+G**, select the connection with
Up/Down, and press **c** (Connect / Retry). Retry checks connectivity and resumes
observation on success; it never resends a turn or terminal input. **u** retries all
unavailable active routes, including launch-only `--access-file` routes that are not
saved address-book entries. Cached history,
unsent drafts, and exact pending command identities remain retained. New sends and
remote changes are refused while the Vessel is known unavailable. Requests already
in flight retain their own outcome handling; losing observation does not cancel them.

**d** disconnects observation explicitly. Saved remote connections also offer alias,
connect-at-startup, renewal/replacement, and forget/restore controls. The startup
preference does not enable repeated retries after a failed connection check.
Access expiry, revocation, identity changes, and unsupported versions keep their
separate labels in the modal; an offline check does not erase credentials or bypass
review. A local startup failure keeps ordinary interactive chat open with its draft;
plain/scripted operations still return the connection failure.

The executing Vessel is shown beside the conversation/draft. New remote
conversations use a workspace selected from that Vessel's approved list. A local
working directory is never silently substituted for a remote workspace. Choosing
a workspace creates only a private draft; the first send retains the existing
separate, deduplicated create-and-submit workflow.

## Pair a workspace connection

1. Open **Vessels → Add HTTPS + pairing** and enter the HTTPS origin, such as
   `https://vessel.example.com`.
2. The panel displays this Helm installation's non-secret **principal UUID**.
   Give it to the executing account's owner so the invitation is bound to the
   intended principal.
3. The owner issues a short-lived invitation on the Vessel host, approving exact
   workspace directories and rights. The local administrative command is:

   ```sh
   vessel pair-invite \
     --directory /absolute/private/vessel-state \
     --endpoint https://vessel.example.com \
     --principal HELM_PRINCIPAL_UUID \
     --workspace /absolute/approved-project \
     --rights catalogue,create,observe,history,execute,steer,decide,cancel \
     --output /absolute/private/invitation.json
   ```

   Repeat `--workspace` for additional approved directories. Run this as the
   account that owns the supervisor state, not as a different administrative
   account. The output parent must be private and owned; the file must be new.
   The command prints the output path, not the secret.
4. Transfer the invitation through a private authenticated channel. Its private
   JSON contains `invitation_id` and `code`; enter them as
   `invitation_id.code` in the panel's masked invitation field. Do not put the
   code in a conversation, command argument, issue, or log.
5. Validate and review the authenticated Vessel identity, workspace scope, rights
   and expiry. Choose a friendly name and whether to reconnect automatically,
   then explicitly save the reviewed access.

The principal is public metadata, not a password. The invitation is secret,
one-time and principal-bound. Invitations default to ten minutes and cannot exceed
fifteen minutes (`--ttl-seconds`). A redeemed connection grant lasts thirty days.
Failed attempts are bounded. After an uncertain response, use the panel's pending
pairing recovery; Helm retains the original command identity instead of issuing a
second redemption. Successfully redeemed invitations can recover that original
credential after invitation expiry, but not after grant revocation or expiry.

The owner's default rights are `catalogue,create,observe,history,execute`.
The example above additionally allows steering, human decisions and cancellation.
`decide` is needed to answer runtime questions/approvals. Add `lifecycle` only when
archive/delete and process lifecycle controls are intended; add `terminal` only
when private terminal attachment is intended. Terminal input also requires
`execute`. These permissions never bypass the runtime's tool policy, workspace
limits or local account permissions.

Pairing uses the executing host's configuration and provider credentials. It does
not permit arbitrary configuration-file paths, administrator policy changes,
participant-session takeover or provider-token copying. Unknown provider readiness
is reported as unconfirmed; actual account/model availability requires host setup
and, separately, an explicitly authorized inference check.

## Import existing conversation access

Use **Import existing access file** and select the private credential path. Helm
validates and copies the exact credential into its own private storage before
saving the reviewed connection, so the original download can subsequently be
moved without breaking the saved connection.

Legacy credentials remain **Shared conversation** access. Only the granted
session is available; importing does not create workspace-wide authority. Obtain a
new owner-approved pairing invitation for additional conversations. Existing
startup `--access-file` routes and their drafts can be explicitly imported without
changing their original delivery identities. Old SSH drafts are preserved but
never automatically mapped to HTTPS.

## Disconnect, replace and recover

**Disconnect** stops this Helm's observation. It does not stop or cancel accepted
remote work. Reconnect retrieves state and resolves retained command receipts; it
never replays a prompt or private terminal input automatically.
If first creation or submission never reached admission, supported servers can
durably close the original command ID and restore editable text. If creation is
confirmed but the first turn was never attempted, press Enter to continue
explicitly. Unconfirmed legacy admission remains fenced rather than guessed.

**Renew/Replace** reviews new access as a distinct connection. It cannot silently
change the principal, endpoint, scope or credentials associated with an old pending
operation. The old recovery records remain available. A changed Vessel identity
requires explicit new review, not automatic repinning.

**Forget** is local connection management, not server revocation or conversation
deletion. Helm retains private recovery material and does not claim that unfinished
deliveries disappeared. Use retained-connection recovery to restore the original
identity when needed. Server revocation is an explicit executing-owner operation:

```sh
vessel revoke-connection \
  --directory /absolute/private/vessel-state \
  --grant GRANT_UUID --expected-revision OBSERVED_REVISION \
  --command-id NEW_COMMAND_UUID
```

Retain that command ID and exact arguments if its outcome is uncertain. Revocation
and expiry invalidate subsequent admissions and the existing runtime authority
watcher; cleanup still requires actual observation. Disconnecting is deliberately
different from revoking execution authority.

## Lost installation or credentials

If the original Helm still has its private connection records, use the panel's
retained-connection recovery. If those files are lost, pair the replacement Helm
as a new installation. Its new public principal UUID does not inherit old access.

On the executing Vessel host, the state-owning account can discover existing
workspace grants even when the old Helm is offline:

```sh
vessel list-connections --directory /absolute/private/vessel-state
```

The command returns JSON containing grant and principal UUIDs, Vessel UUID,
revision, approved workspaces, rights, expiry and revocation state. It includes
expired and revoked records, sorted by grant UUID. It omits credentials and their
hashes, invitation codes, provider data and conversation contents. The directory
must already exist and be canonical, private and owned by the current account.
An existing unpaired directory returns an empty list without creating access state.
Unsafe or malformed records fail the whole listing without partial output;
concurrent pairing changes can return a bounded busy error that can be retried.
Inventory is bounded to 4,096 grants and 16 MiB of output. It does not grant access.

Identify the lost installation's grants using its recorded principal and approved
scope, then run `vessel revoke-connection` above for each intended grant using its
observed revision. The server stores no friendly device name: workspace scope
alone may not distinguish installations. Do not guess or revoke unrelated access.
If the revision changed, list again and review before issuing a new command ID.
For a lost revocation reply, retry the original command ID and exact arguments.

Pair the replacement using a new invitation and explicitly review its scope.
Old grants remain independent until revoked or expired. This does not recover
lost local drafts or pending delivery records, replay their commands, or delete
remote conversations. Session-only sharing grants use their separate
[process-access revocation](process-access.md) workflow.

## Storage and trust boundaries

The manager uses the private `helm-connections` directory alongside the local
Vessel directory (normally `~/.local/state/voyage/helm-connections`). Its versioned
address book, private immutable credentials, pending pairing records and recovery
records are separate from provider configuration. Updates use locks, per-record
revision checks and atomic writes; unsafe ownership, permissions or symlinks are
refused rather than silently repaired.

### Encrypted connection secrets (Linux)

New managed `.credential` and `.redemption` writes in Helm, and the Vessel's
`access/pairing/state.json` journal, require a separately provisioned encryption
key. Missing provisioning refuses the write; there is no plaintext fallback and
no automatically generated replacement key. This can prevent new pairing/import
or owner grant changes after upgrading until the executing process is provisioned.
Ordinary local voyages and provider credentials do not use this key.

Each host sets `VOYAGE_CREDENTIAL_KEY_FILE` to an **absolute path**, not key material.
The file must contain exactly 32 high-entropy random bytes, be owned by the executing
account, owner-only, a single-link regular file without symlink path components,
and reside on Linux **tmpfs**. Persistent key files are refused. Use an independent
secret manager/unlock service to provision the same key at startup; never put the
bytes in argv, environment values, conversations, issues or logs. Do not change the
key or provisioning file while Helm/Vessel or a migration is using it. This release
does not provide online key rotation. Host keys must remain independent; pairing
never transfers the encryption key.

For example, after private provisioning (the commands below contain only paths):

```sh
VOYAGE_CREDENTIAL_KEY_FILE=/run/user/1000/voyage-secrets/connections.key helm
VOYAGE_CREDENTIAL_KEY_FILE=/run/user/1000/voyage-secrets/connections.key \
  vessel connection-audit --directory /absolute/private/vessel-state
```

Substitute the actual executing UID and provisioned path. For an independently
running/headless Vessel, configure that service's environment and unlock dependency
explicitly; a variable set later in Helm cannot unlock an already-running Vessel.
A systemd runtime credential is usable only if its actual file satisfies the checks
above. Its durable source must have independently protected key custody (for example,
an appropriately configured TPM-backed or external secret manager), not an unprotected
host key beside the encrypted archive. The installer does not generate keys or
silently install an unlock service. No key is fetched over a network by Helm/Vessel,
no unlock prompt can hang unattended execution, and errors do not disclose key bytes.

Records use AES-256-GCM with a fresh random nonce and authenticated record-purpose
binding. Helm credentials retain their original filenames; renaming ciphertext
invalidates that binding. A private authenticated marker catches using a different
key for subsequent Helm secret writes. Both applications retain private-file checks,
bounded reads, exclusive writer locks and atomic publication. The key is reread rather
than cached between storage operations. Removing provisioning prevents subsequent
protected reads/writes; this is **not** a cancellation or revocation mechanism for
credentials already loaded into a connection or running operation.

**Threat boundary:** encryption protects copied managed persistent files and their
backups when the thief does not have the separately held key. It does not protect
against the owning account, root, a compromised running process, memory capture,
unencrypted swap/hibernation, or an attacker who can roll back whole storage snapshots.
Tmpfs can swap: provision encrypted/disabled swap and appropriate hibernation policy
if disk-theft protection includes those assets. Authentication detects changed
ciphertext, not a coherent old backup. Application policy is not an OS sandbox.

| Asset | Protection and ownership |
| --- | --- |
| Helm immutable credentials, pending redemption requests (including invitation codes), retained forgotten-access/recovery secrets | Encrypted on new writes, including temporary publication bytes. |
| Helm principal UUID, address book, pending-operation IDs and preview metadata | Owner-private filesystem records, not secret authentication material; not an encrypted conversation store. |
| Vessel pairing journal: original returned credentials, invitation verifiers, exact redemption/revocation intents and audit | Encrypted on new writes, including temporary publication bytes. |
| Vessel connection grants | Owner-private authority metadata and high-entropy bearer verifiers; inventory remains available without unlocking the journal. No bearer token is stored in these files. |
| Invitation output and imported access files | Explicit private exchange/export files; not automatically encrypted with a key the other host lacks. Transfer through an authenticated confidential channel, control their retention, and protect their backups separately. Import does not erase the source. |
| Provider credentials, Vessel signing identity, session-only sharing and participant stores | Separate authority/storage surfaces, not migrated or claimed protected by this connection feature. |

### Existing data, backups, key loss and interrupted migration

Existing plaintext records remain readable and are **not** silently rewritten on
startup. This is legacy compatibility, not acceptance of filesystem-only protection
for an installation claiming encrypted storage. Provision the original host-local
key, retain separately protected recovery material, stop concurrent administration,
and explicitly migrate each applicable host:

```sh
helm protect-connections --directory /absolute/private/helm-connections
vessel protect-connections --directory /absolute/private/vessel-state
```

Both commands require `VOYAGE_CREDENTIAL_KEY_FILE` in their environment. They perform
no network/provider request and preserve credentials, principals, grant scope and
pending command identities. Helm also protects retained legacy `.tmp` records;
Vessel protects legacy `.pending-UUID` journal files. Already encrypted orphan
publication files remain retained, not promoted into canonical state. Migration is
atomic **per file**, not across the directory: an interruption can leave a mixed
legacy/encrypted directory. Repeat the same command with the same key to finish.
Unreadable/unsafe/damaged records fail without replacing that record or guessing a
new identity. A failed run is not proof that earlier files were unchanged.

Back up ciphertext and exact identity/recovery records together; keep the decryption
key separately under independent protection. Migration cannot erase old filesystem
blocks, plaintext backups or independently exported files. Inventory/rotate those
copies explicitly before claiming the entire backup set encrypted. Do not delete
pending records or edit envelope fields to bypass a locked state. Downgrading to an
older binary cannot read the new envelopes; never strip encryption to make it run.

A missing or incorrect key leaves encrypted records intact. Restore the **same** key
through the independent provisioning channel, then reopen/retry the original pending
operation. There is no key escrow, password reset, implicit re-pair or key resurrection.
Permanent key loss means the protected recovery contents cannot be recovered. Helm
can be replaced as a new installation using the lost-installation procedure above.
If the Vessel journal key is lost, credential-free inventory is still available but
journal-based pairing/revocation/audit cannot proceed: restore the key or explicitly
retire access through the host's service/network administration and establish a new
Vessel identity. Stopping a gateway alone does not establish cleanup of existing
voyages. Never claim old grants revoked or processes cleaned up without evidence.

### Current-grant lifecycle audit

The executing account can read a bounded, content-free projection:

```sh
vessel connection-audit --directory /absolute/private/vessel-state --limit 64
```

This is a local-owner operation, including with Vessel stopped, not a remote model
or scoped-client audit API. It opens the encrypted pairing journal and therefore
requires its key, unlike `list-connections`. It never initializes, reconciles or
repairs state. Private ownership checks and a shared nonblocking pairing lock apply;
contention returns a bounded busy error. Output is prepared completely before printing,
with at most 128 events per page and a 512 KiB output cap.

Events distinguish `audit_started`, `invitation_recorded`,
`grant_publication_intended`, `grant_publication_observed`, `revocation_intended`,
and `revocation_observed`. Intent and its event share one atomic journal write before
publishing authority. Publication observation and its persistent deduplication marker
share another. Interrupted finalization leaves uncertainty; an explicit exact retry
observes the existing grant and finalizes once. It never invents a second invitation,
replays a voyage command or recreates a grant known to have been published and then
removed. Full authority fields are checked before accepting a retained publication.

Events expose only typed lifecycle kind, sequence, observation time, relevant UUIDs,
revision/expiry, workspace UUIDs and enumerated rights. They exclude tokens/verifiers,
codes, endpoints, paths/names, conversation/provider data, raw errors and free text.
An invitation event does not prove delivery of its private output file. A principal
UUID is public metadata, not an authenticated person's identity. Expiry is enforced
from the retained deadline, not fabricated as an event when someone reads history.
Anonymous failures, connection/disconnection/forget actions, session grants and
participant bindings are not covered. Renewal/replacement is a distinct grant.
Revocation reports `cleanup: not_observed` on first success and retry: Voyage authority
watchers enforce changes independently, and neither an audit nor a CLI receipt proves
that resources stopped.

Ordering uses checked monotonically increasing sequence numbers, not wall time.
The first page pins an upper sequence and digest; pass its exact `next_cursor` as
`--cursor` with the same `--limit`. Concurrent appends cannot enter that page sequence,
and the continuation works after restart with the same journal. It is an observation
cursor, not an authority token. Changed generation/horizon, invalid cursor, sequence
gaps or retention overtaking the continuation cause explicit refusal rather than
silently skipping history. Start a new read explicitly after such a refusal.

Retention keeps the newest **4,096 events**, not a promised number of days. The
response reports the first retained sequence and `history_pruned`; operation markers
outlive event pruning so exact retries do not duplicate old lifecycle events. Existing
invitation/revocation capacity limits still apply, and unresolved new publication
intents are not silently discarded merely because their grant expiry passed. Recording
begins on the first new journal mutation. Legacy pre-audit history is explicitly
unavailable, never reconstructed from inventory or described as empty. No old enrollment
audit data is imported and no retired enrollment subsystem is restored.

Focused offline verification commands (use the shared build lock in coordinated
workspaces):

```sh
cargo test -p voyage-storage -p helm -p vessel --locked --lib
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 vessel/tests/connection_protection.py --bin-dir target/debug
```

The Rust cases exercise authenticated envelopes, wrong/missing keys, explicit
migration, corrupted/private-file refusal, exact publication recovery failpoints,
deduplicated audit events, cursor horizons and retention. The process fixture uses
isolated HOME/XDG directories, synthetic credentials and loopback-only pairing;
it makes no provider requests or native macOS/Windows claim. Its printed private
evidence directory retains process/CLI results. These commands describe the checks;
actual run results must accompany the delivered source revision.

Human connections do not automatically populate `[vessel.remotes]` or grant a
model coordination authority. That configuration belongs to the executing host
and remains a separate explicit decision.

Use trusted HTTPS. Credentials in URLs, HTTP redirects and certificate-verification
bypasses are not supported. Literal-loopback HTTP remains a development exception,
not a LAN transport. With Cloudflare Tunnel, TLS can terminate at Cloudflare and
forward to the Vessel gateway's private loopback HTTP listener. An interactive
Cloudflare Access login page is not a supported Helm authentication method; the
Vessel credential authenticates API access. Preserve authenticated WebSocket upgrade
at `/v1/vessel/socket` with the `voyage.vessel.v1` subprotocol; commands and events
share this socket. Helm does not fall back to HTTP/SSE if upgrade fails. See
[duplex transport](duplex-transport.md). HTTP pairing also needs
`/v1/vessel/pair` and `/v1/vessel/pair/capabilities`.

Vessel and native private-storage support remain Linux-focused. A build or local
fixture is not evidence for native macOS/Windows operation or arbitrary reverse
proxy configurations. See the implementation's recorded verification for actual
platform and deployment evidence.
