# In-app Vessel connections

Status: proposed implementation plan, not implemented capability.
Tracking: [#194](https://github.com/o-psi/voyage/issues/194).

## User outcome

Open Helm normally, once. Choose **Vessels → Add Vessel**, connect to an HTTPS
endpoint, and use its permitted conversations without restarting Helm or changing
its desktop launcher. Saved connections return on the next normal launch. Local
work remains immediately usable while remote machines connect or fail.

A file picker alone is not the complete fix: the current remote credential grants
one conversation, whereas an owner expects to connect to a Vessel and start more
conversations. Deliver both the in-app connection lifecycle and explicit
workspace-scoped remote creation authority. Keep conversation sharing restricted.

## Observed implementation constraints

- `helm/src/process_client/cli.rs` constructs all clients from startup arguments.
- `helm/src/process_client/ui/mod.rs` owns a `Vec<Client>`; observation jobs start
  once through `ui/observe.rs`. There is no dynamic connection manager.
- `ui/state.rs::Target` and asynchronous updates identify routes by vector index.
  Removing/reordering those entries would risk targeting a different connection.
- `ui/drafts.rs` and `ui/new_draft/storage.rs` persist route-derived identities and
  exact pending creation/submission envelopes. They cannot be discarded or
  silently rebound when an address or credential changes.
- `process_client/access.rs` already validates private grant files, HTTPS and
  protocol versions, refuses redirects, and supports authenticated SSE.
- `vessel/src/process/access/routing.rs` filters catalogue/operations to a grant's
  single session and workspace. With lifecycle permission the API can create that
  exact preassigned UUID, but cannot create arbitrary additional conversations.
  The current TUI refuses creation on any access-file route.
- `[vessel.remotes]` configures model-driven coordination on the executing host;
  it is not a remembered Helm connection list. Do not conflate these authorities.
- Both ends of the public API, the installer readiness path, existing private
  storage and real desktop entrypoints must be covered by delivery.

## Interaction design

### Always-available Vessels control

Add a visible **Vessels** control above the conversation sidebar. It opens a panel
usable by mouse and keyboard, with `/vessels` as an equivalent command. When the
sidebar is hidden, expose the same control in the compact header and help; no
particular function key or wide terminal is required.

The panel lists **This computer** and saved remote connections with friendly names,
endpoint, access scope and state: Connecting, Connected, Offline, Access expired,
Access revoked, Identity changed, or Unsupported version. Actions are Add,
Connect/Retry, Disconnect, Rename, Renew/Replace access and Forget. Distinguish a
local display name from the authenticated remote identity.

Closing the panel restores the untouched composer and selected conversation.
Connecting never creates a voyage, changes its provider, or sends a draft.
Disconnect means stop this Helm's observation, not cancel remote execution.
Forget removes local connection configuration; it does not revoke server access,
archive/delete conversations or pretend unresolved commands never existed. Explain
retained drafts and pending deliveries, and keep their recovery records available.
Remote revocation is a separate authorized action, not an effect hidden in Forget.

### Add and remember a connection

Primary flow:

1. Enter the HTTPS address and an optional friendly name.
2. Complete a short-lived owner-authorized pairing flow in the connection panel.
3. Show the authenticated Vessel identity, allowed workspaces, access rights and
   expiry before saving. Reject unsupported servers with an actionable explanation.
4. Save the connection and its private credential, then load permitted voyages in
   the current window. Enable automatic reconnect by default, with a visible toggle.

Also support **Import existing access file**. Read it securely, validate the server,
then copy the credential into Helm-managed private storage; do not depend on a file
in Downloads continuing to exist. Clearly mark this mode **Shared conversation**
when it has only session authority. Offer explicit import of existing startup
credentials and detect duplicates; do not scan arbitrary private directories.

Our already-provisioned remote can immediately use the import path. Upgrading it
to workspace-wide access requires a new owner-authorized pairing; importing its
current credential must not elevate permissions.

### Selecting machines and starting conversations

Keep a unified conversation list with persistent, readable Vessel badges and a
Vessel filter. Do not force a user to disconnect one machine to use another.

**New conversation** shows the destination Vessel and a workspace picker populated
only with server-authorized workspaces. Remember the last selection per Vessel,
never reuse a local path as a remote path, and display the executing machine beside
the composer. Preserve draft-before-first-send semantics. Session-only sharing
shows a clear limitation and a request-for-broader-access action rather than a
New button that inevitably fails.

Provider readiness and model choices come from the execution host. If sign-in is
missing, explain how to authorize that host without copying local provider tokens.

## Implementation order

### 1. Stable connection registry and persistence

Introduce an immutable local `ConnectionId` and a separate connection generation.
Replace positional route identity in targets, observer messages, draft lookups,
terminal requests, pending commands and first-send recovery. Store connections by
ID; never reuse an ID for another principal or Vessel.

Keep a versioned, atomically updated, owner-private Helm address book, separate
from provider/execution configuration. Store credentials separately with the
existing file-ownership protections. Retain the pinned Vessel identity, credential
reference, scope, alias, reconnect preference and workspace preference. Specify
concurrent-window locking/conflict handling before wiring UI writes.

Migrate existing local/HTTPS draft keys by exact legacy route identity. Preserve
original command IDs, principal, grant binding, payload and expiry. Old SSH drafts
remain preserved and unsupported, not mapped to a new HTTPS connection. Do not
merge privileges from distinct credentials merely because they name one Vessel.

### 2. Dynamic observation and the in-app manager

Move observer ownership into a connection manager that supports add, stop and
restart per connection. Every asynchronous result carries connection ID and
generation; ignore stale results after disconnect, replacement or reconnect.
Use bounded concurrency, timeouts and cancellation/cleanup observation. A dead
remote cannot block input, local launch, other routes, or another route's recovery.

Adding/removing connections must not switch an in-flight command's destination.
Disconnecting stops watchers, but retains uncertain commands and their exact
receipts for later resolution. Reconnecting observes, never resubmits. Credential
replacement is not permission to replay an old command under a new principal.
Resolve old uncertainty only with authority supported by the server; otherwise
retain an explicit unresolved state. Suspend private terminal input on loss;
never buffer or replay keystrokes through a replacement connection.

Wire the Vessels panel, private credential import and persistent startup loading
through all interactive entrypoints, including plain `helm` and ordinary chat.
Keep explicit CLI subcommands deterministic: saved connections must not turn a
single-target `connect list` or script into implicit fan-out.

### 3. Scoped HTTPS pairing and remote creation

Add a versioned, explicitly advertised credential type for a Helm principal's
access to approved workspace IDs. Do not reinterpret old session grants or turn a
missing session ID into unrestricted access.

The executing account owner creates/approves a pairing invitation, using a local
administrative action or a narrowly scoped authenticated approval page. Helm's
connection panel redeems it over HTTPS. Design this as a distinct human-client
pairing flow, not outbound-worker enrollment or participant-Vessel execution.

Invitations must be short-lived, one-time, rate-limited and bound to the intended
Vessel, principal and reviewed workspace/rights scope. Use a private connection
input, never conversation text, for pairing secrets. Bind redemption to a stable
operation identity: a lost response cannot consume the invitation twice, mint a
second grant or leave an unrecoverable credential. Expired/replayed invitations
fail closed. Do not expose operator tokens to Helm or the model.

Define separate permissions for catalogue, create, observe/history, execute,
steer, decisions, cancel, lifecycle and terminals. Enforce workspace membership
and canonical paths on the server before dispatch, with revocation/expiry checked
through the existing execution boundary. A paired client may choose among
owner-approved workspace/configuration profiles, not supply arbitrary host config
paths, edit host policy or acquire OS account authority.

Return bounded permitted-workspace and provider-readiness metadata. Support
explicitly authorized multi-voyage observation/SSE rather than the current gateway's
one-subscription restriction. Retain revision-bound pagination and frame limits.
Use the existing deduplicated creation plus initial-submit protocol; a reconnect
must not create a duplicate voyage after an uncertain first send.

Preserve old session grants for sharing. Display real expiry, warn before access
expires, and provide renewal/replacement in the same panel with owner authorization
where required. Renewal cannot silently increase scope. Identity changes require
new review, not automatic repinning. Keep provider credentials on the remote host.
Human connection setup must not automatically expose that route to model-driven
Vessel coordination; that remains a separate explicit host-policy choice.

### 4. Deploy and finish the actual desktop workflow

Ship compatible Helm/Vessel builds and update the installed applications, not only
source. Existing scoped connections stay usable with older servers; pairing/create
controls remain unavailable unless the server advertises the necessary capability.
No SSH fallback and no certificate-verification bypass.

Verify the Cloudflare Tunnel path using ordinary trusted HTTPS and SSE, including
connection loss and stream buffering behavior. Preserve the existing local service,
provider setup and remote conversations. No infrastructure secret belongs in the
public issue, documentation, transcript or test output.

## Acceptance and verification

The feature is not done until these journeys pass on runnable Linux builds:

1. Open plain Helm, type an unsent local draft, add a remote from the panel, use its
   conversation, and return to the unchanged draft without restarting Helm.
2. Quit/reopen normally: saved Vessels reconnect, the local interface is immediately
   usable, names/preferences persist, and no voyage or turn is implicitly created.
3. Pair with approved workspace access, create two distinct remote conversations,
   switch between them and local work, and see the correct host throughout.
4. Import the existing session-only credential: only that session is visible;
   creation, unrelated history and broader workspace access are denied server-side.
5. Add duplicates, reorder/filter, disconnect one route while another runs, and
   deliver a late observer/command result: no update or input reaches another host.
6. Disconnect during create, first send, a running response and private terminal
   attachment. Reconnect without duplicate creation, prompt replay, terminal-input
   replay or cancellation of accepted remote work.
7. Exercise invalid TLS, unavailable DNS, tunnel/proxy denial, incompatible server,
   expired/revoked access, changed Vessel identity, and failed renewal. Keep the UI
   responsive and distinguish refused operations from uncertain delivery.
8. Verify crash-safe private storage, existing draft/pending-command migration,
   concurrent Helm windows, pairing replay/expiry, and invitation redemption after
   a lost response. Credentials never appear in public history or logs.
9. Check keyboard, mouse and narrow-terminal paths with real PTY interaction, not
   just parser or screenshot checks. Verify actual installer activation/readiness
   and remote supervised creation, streaming, suspension and reconnect over HTTPS.

Use targeted build/static checks, bounded protocol checks and real PTY/deployment
exercises. The previously removed automated suite is not evidence; wholesale suite
recreation is outside this plan. Record the exact evidence and any platform limits.

Stages 1–2 can make the current grant usable without relaunching, but must be
labelled partial delivery. Close #194 only after the complete pairing, remembering,
new-conversation and failure-recovery workflow is installed and verified.
