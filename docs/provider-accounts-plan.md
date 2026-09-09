# Native provider accounts

Status: proposed implementation plan; not current functionality.
Tracking: [#213](https://github.com/o-psi/voyage/issues/213).
User requirement: support multiple accounts for one provider; select/configure the
account in Helm, including device-code sign-in for local and remote execution hosts.
Credentials remain on the executing machine. Vessel CLI enrollment remains an
alternative. Browser callback relays and API-key entry in Helm are out of scope.

## Product decisions

- Helm selects accounts and provides the device-code sign-in interface. Enrollment,
  token exchange, credential storage and refresh stay on the execution host. The
  Vessel auth CLI and Helm-initiated enrollment share the same host-side service
  implementation. Voyage resolves credentials and performs inference there.
- No provider access/refresh tokens pass through Helm. Only the temporary user code
  and verified provider verification URL reach a private authentication view. The
  local browser's normal provider website cookies are separate from remote runtime
  credentials. No localhost redirect relay, SSH tunnel or token-forwarding fallback.
- Support all four native transports: OpenAI Responses, OpenAI Chat, ChatGPT OAuth,
  and Anthropic. API billing and subscription access remain distinct. Do not restore
  the removed external Codex bridge.
- One explicit account is bound to each voyage's next-run configuration. A running
  execution retains its admitted binding. Different voyages may use different
  accounts concurrently, including accounts of the same provider.
- Defaults apply to new drafts/voyages only. Changing a default never retargets an
  existing voyage. No automatic fallback or rotation on expiry, quota, or failure.
- Account selection must work for authorized local and remote Helm connections.
  Stored provider credentials never cross a Vessel connection, enter model context,
  or appear in public snapshots, histories, command receipts, or diagnostics. Device
  codes and verification links are sensitive enrollment material, not public
  account metadata; keep them out of chat, logs, ordinary events and receipts.

## Existing integration points

These are source constraints, not capabilities already delivered by this plan:

- `vessel/src/auth.rs` enrolls one native ChatGPT OAuth login through browser/device
  flow or explicit credential-file import. Its commands use one default token path.
- `voyage/src/provider/chatgpt_oauth.rs` uses the legacy
  `helm/chatgpt-oauth.json` cache and an in-process mutex. Multiple independent
  voyages need cross-process refresh coordination, not just another token filename.
- `voyage/src/config.rs` selects a transport and API-key environment variable.
  `voyage/src/launch_config.rs` persists trusted execution configuration.
- `voyage/src/provider/mod.rs` constructs native providers;
  `voyage/src/provider/inference.rs` scopes model observations to provider,
  endpoint, and credential/account context.
- `voyage/src/server/configuration.rs` already supports next-turn inference changes
  while a run is active, but rejects configuration mutation from scoped grants.
- `helm/src/process_client/ui/inference.rs` implements composer controls and slash
  pickers for live voyages and drafts. Selection must extend those flows, including
  pending-command recovery, rather than add a disconnected settings screen.
- Public/private command mapping is in `crates/voyage-protocol/src/vessel.rs`,
  `crates/voyage-protocol/src/process/types.rs`, and `vessel/src/process/api.rs`.
  Both Vessel and Voyage enforce authority; routing cannot grant missing rights.

## Account model and storage

Separate three concepts internally while keeping Helm's picker simple:

| Concept | Responsibility |
| --- | --- |
| Provider connection | The service/endpoint and permitted transports, including explicit custom compatible endpoints |
| Account profile | A named authentication/billing context, with provider identity and organization/project context where supported |
| Credential revision | The current OAuth tokens or API-key binding used to authenticate that profile |

Introduce an execution-host registry in protected private storage. Connections and
profiles have stable UUIDs and revisions. Profiles have user-assigned unique aliases,
display labels, compatible connection references, lifecycle state and an identity
generation. Labels/aliases are bounded and terminal-safe. Helm keys references by
authenticated Vessel identity plus UUID, not label or route index. Rename never
changes identity; deleted UUIDs are not reused.

Credentials live separately from safe account metadata. OAuth credentials hold
verified provider identity privately. API credentials bind either a privately
entered/stored key or an explicitly named execution-host environment variable.
A changed environment-variable value is a credential revision requiring the same
rotation/replacement checks, not an untracked account switch.
An API account profile is a user-declared context, not necessarily a provider
identity we can independently verify. Do not infer that two keys identify distinct
accounts/organizations, or that enrollment establishes credit or model entitlement.

One OpenAI API profile may explicitly support both Responses and Chat on the same
approved connection without duplicating credentials. Keep provider connection
configuration separate from credential storage. Never send a key to a new endpoint
merely because it supports a compatible transport. Account/organization/project,
endpoint, or allowed-transport changes require reviewed configuration; they cannot
be smuggled in as a label edit or routine credential rotation.

Keep metadata revisions, account identity generations, and credential revisions
separate. Freeze account/billing/endpoint identity for an admitted run, not token
bytes: OAuth refresh and safely authorized same-identity API-key rotation may serve
that run. A different OAuth login identity is an explicit replacement and invalidates
old bindings. Where an API key's identity cannot be verified, fail closed on claimed
same-identity rotation unless an explicit trusted owner attestation establishes the
intended binding; otherwise treat it as a replacement. Record that distinction
without claiming provider verification. Rename and ordinary refresh do not change
account identity. Credential revision changes still invalidate observations that
may depend on key-specific permissions; an identity-stable OAuth refresh is not
itself evidence that account capabilities changed.

Storage operations must be bounded, private, ownership checked, symlink/hardlink
safe, atomic, and crash recoverable. Reserve enrollment identity before browser or
network effects. Pending enrollment is not selectable. Failed/cancelled enrollment
must not overwrite a working account or leave a partially published selectable one.

## Executing-host enrollment and lifecycle

Vessel owns bounded enrollment operations, not session agent loops. Host-side auth
code is shared by its local CLI and authenticated human enrollment API; neither
requires a conversation or starts a competing canonical session owner. Provider
refresh remains available to independent Voyage processes through the same private
account storage and coordination primitives.

Proposed command shapes (not implemented commands):

```text
vessel auth login --provider chatgpt-oauth --account personal
vessel auth login --provider chatgpt-oauth --account work --device
vessel auth add --provider openai-responses --account work-api
vessel auth list
vessel auth status --account work
vessel auth logout --account work
```

API-key enrollment prompts in a private human terminal, never in Helm chat or a
secret-bearing command argument. Also support an explicit environment-name binding
for existing deployments. Explain that inherited environment changes may require
restarting the supervisor; stored-key enrollment should not require restarting it.
Status is cached/local unless an explicitly requested bounded validation occurs.
Listing accounts does not issue inference requests.

Provide explicit rename, reauthenticate, and remove operations. Reauthentication
must confirm the expected provider identity; a different identity is a replacement,
not routine refresh. Adding an existing alias fails instead of overwriting it.
Credential-file import targets one explicit account and never executes Codex.

Logout tombstones that account and advances its identity generation before reporting
success. All voyages check current account identity generation/state before subsequent
provider requests, retries, discovery, and refresh commits. A stale refresh cannot
resurrect a logged-out record. Requests already dispatched cannot be recalled;
logout must say so and must not claim upstream token revocation. Removal leaves
existing voyage references visibly unavailable rather than rebinding them.

## Helm experience

### Account control

Add **Account** alongside Model/Thinking/Service, with `/account` as the keyboard
entrypoint. It operates on the selected draft or voyage and shows the executing
Vessel, transport, safe account label, and local authentication state. Never display
raw token material or provider account identifiers by default. Support narrow
layouts, search, keyboard/mouse selection, cancellation, and preserved composer text.

The picker lists only accounts the caller may use for that destination/workspace
and transport. Show expired, unavailable, or sign-in-required states with reasons.
Offer **Add account / Sign in** when the selected host supports device enrollment
and the caller has permission. Otherwise explain the host-specific CLI alternative
or required permission without prompting for credentials in chat. API-key enrollment
remains a private execution-host terminal operation. Offline lists are cached
observations, not permission or confirmation that a selection can be applied.

For an existing voyage, show a short confirmation naming old/new accounts and
explaining that subsequent inference may send retained conversation/tool context
to the newly selected account and its organization. Never imply switching wipes
history. Keep the model and inference overrides when valid; if catalog evidence
shows incompatibility, offer explicit model/override changes atomically with the
account switch. Unknown support remains unknown, not a fabricated validation pass.

### Device-code sign-in

1. In the account picker, choose **Add account → Sign in**, review the authenticated
   executing Vessel/provider, and supply a safe alias. Sign-in permission is distinct
   from permission to use an already enrolled account.
2. The host reserves a stable enrollment UUID before contacting the provider. It
   starts device authorization and retains the device polling secret privately.
3. A dedicated private Helm authentication view displays the verified provider URL,
   short-lived user code, target host, expiry, and pending state. Open the URL in the
   user's local browser only on explicit action. Validate provider URLs against the
   supported provider flow; never launch arbitrary remote-supplied URLs or commands.
4. The user approves on the provider page. The execution host polls at the provider's
   required interval, handles pending/slow-down/denied/expired responses, exchanges
   authorization, verifies the returned identity, and atomically publishes the profile.
5. Helm observes success and offers the new profile for selection. Enrollment alone
   does not switch a voyage, change a default, or dispatch inference.

Access/refresh tokens and the provider's device polling secret never leave the host.
The private user-code/URL response is bounded and available only to the authorized
initiating principal. Do not persist it in Helm drafts, conversation messages,
ordinary command receipts, event streams, crash reports or model-visible tools.
If resuming a pending flow requires fetching it again, use the authenticated private
enrollment endpoint with current authority. Clear sensitive display state on closure,
expiry, cancellation or completion; reopening rechecks current authority.

Use stable enrollment/command IDs, exact deduplication and a bounded host-managed
lifetime. Closing the view or disconnecting Helm does not create another attempt;
reconnect can inspect the original operation. **Cancel sign-in** is an explicit
host mutation. Serialize cancellation/revocation against final credential publication:
if cancellation wins, discard late results; if publication already committed, report
success rather than pretending cancellation undid it. Offer explicit logout only
when authorized; otherwise explain the host-owner revocation path.
Crash recovery must not blindly repeat an uncertain token exchange. Retain unresolved
state with bounded recovery guidance; expiry is not proof a lost response had no
effect. Bound concurrent attempts and retained records, and never leave an orphaned
unlimited polling task. Upstream authorization effects cannot always be revoked.

For providers without a supported device flow, Helm shows that limitation and the
execution-host terminal alternative. A generic browser callback relay and a masked
API-key form in Helm are explicitly deferred; neither is required by this release.

### Defaults and drafts

Keep selection separate from preferences and host administration:

- The executing host supplies a configured fallback account for a connection.
- Helm may remember an explicit choice for **new voyages**, keyed by authenticated
  Vessel identity, workspace and provider connection, as a local UI preference.
- A draft shows its explicit/resolved selection. A concrete account identity is
  persisted in trusted execution configuration when the voyage is created.

For a new draft, resolve explicit choice, then applicable Helm preference, then host
fallback. An unavailable or unauthorized remembered choice requires review rather
than silently using another account. Server validation remains authoritative; a UI
preference grants no account access. **Remember for new voyages** must not mutate
shared execution-host configuration. Host fallback changes remain owner-controlled
configuration, not a second account-picker feature required for the first slice.

Resolve and show a concrete UUID when initializing the draft. Changing a preference,
server fallback, reconnecting, or reordering the registry cannot silently retarget an
existing/pending draft. Persist the exact selection in its create envelope and
validate again at admission. Changing the selected Vessel clears/re-resolves
host-local selection with visible review. Pending create commands retain their
original account identity until their outcome is resolved.

### Running voyages

Allow selecting the account for the **next run** while the current run continues.
Display both "Running with Personal" and "Next run: Work" until the run boundary.
The active root agent, its subagents, retry attempts, and title generation performed
for that run retain the admitted account/billing/endpoint identity, while allowing
same-identity credential refresh. New subagents of that run cannot pick up staged
next-run settings. If logout invalidates that binding, fail subsequent
requests rather than switch accounts. A later run adopts the committed selection.

Selection survives suspend/resume and restart. A branch inherits the selected
next-run account, subject to current authority and availability; it does not gain
access by inheriting a reference. Old history retains safe historical attribution.
Never reuse provider continuation/response IDs across account bindings; build the
new request from canonical conversation under the new account.

## Protocol, authority, and persistence

Add capability-advertised account enumeration and account-aware creation/configuration
commands, plus separate capability-advertised human device-enrollment operations.
Account enumeration returns safe descriptors and observed registry revisions only;
enrollment's private code/URL response is not part of the general control catalogue.
Use an atomic next-run inference selection envelope containing connection/account
UUIDs, expected identity/configuration versions, model, and overrides, with stable
command UUID and expected session/configuration revision. Validate the complete
combination and caller authority before durable acceptance; incompatible settings
require explicit resolution, not a partially applied account switch. Recheck at run
admission. Routine token rotation alone must not invalidate a pending selection.

Integrate exact request deduplication, conflict detection, receipts, event updates,
and recovery of lost responses. A timeout is not a failed switch to retry under a
new identity. Invalid or stale selection leaves the previous configuration intact.
Checkpoint the selected UUID/version with trusted launch configuration; store no
credential in session text. Old servers advertise unsupported account selection and
Helm shows an actionable limitation, not an optimistic local-only switch.

Remote selection needs a new narrowly scoped account-use authority and explicit
account allowlist bound to workspace grants. Existing observe/execute/create rights
must not automatically reveal the registry or authorize account switching. Existing
grants default to no new selection authority until updated by the host owner.
Enforce filtering and mutations at both Vessel and Voyage, including revocation
between picker display and admission. Do not make general Configure or provider
credential administration available to scoped callers to implement the picker.
Remote device enrollment requires an explicit host-owner-granted enrollment right,
scoped to permitted providers/connections and destinations. It is distinct from
account-use, observe, execute and create rights; existing grants do not acquire it.
Recheck authority during enrollment and before publishing credentials. Newly enrolled
profiles are private by default, with access limited to the host owner and explicitly
authorized initiating principal/workspace; never extend other clients' allowlists.
Credential replacement/deletion and host fallback administration remain separate
owner rights, not effects implied by permission to add a new account. API-key entry
and broad registry administration remain execution-host operations in this scope.

Model-facing tools gain no implicit account enumeration, selection, or enrollment
powers from these human Helm features. Existing local private transport protections
apply locally; remote enrollment requires authenticated HTTPS and pinned Vessel
identity. Route presentation cannot broaden execution-host authority.

## Refresh, caches, and migration

Use per-account cross-process locking with bounded waits and version-checked atomic
refresh publication. Readers re-read after taking the lock so independent voyages
reuse a completed rotation. An uncertain refresh response is not blindly replayed;
retain a pending/uncertain state and require safe recovery or reauthentication.
No global refresh lock that serializes unrelated accounts. Resolve the token store
through the account binding for both inference and discovery.

Scope catalog observations to account UUID/identity generation, connection/endpoint,
transport and relevant config. Invalidate on switch, replacement, logout, or
permission-relevant credential changes; ordinary OAuth token rotation preserves
account identity. Async picker/catalog replies carry the exact selection context
so old results cannot overwrite the newly selected account.

Migrate the existing single OAuth cache as a stable default account without losing
login or resurrecting a logged-out record. Make migration idempotent and crash-safe;
authoritative enrollment state and credentials must not diverge. Preserve existing
native environment-bound configurations as explicit legacy bindings before asking
users to enroll named accounts. Do not silently switch legacy API billing to OAuth.
Existing saved native configurations, drafts, and trusted launch records must load
with defined migration behavior, not require users to edit private session journals.
Mixed-version writers must be fenced or require an explicit upgrade boundary; old
binaries must not keep writing a legacy cache behind the new registry's back.

## Delivery order

Deliver vertical slices; do not build a general account-management framework before
proving the core workflow. Required safety behavior belongs in each slice, not in
an unspecified later hardening phase.

1. **Native OAuth end to end:** define only the connection/profile/credential types,
   safe migration, bounded host enrollment/refresh operations and advertised protocol
   needed to enroll two accounts through Helm device flow. Include the Vessel CLI
   alternative, minimal draft/live picker, atomic account/model/override selection,
   persistence, identity-stable active runs, logout races and observed cleanup.
   Prove enroll → select → execute → switch → resume before extending the surface.
2. **Authorized remote completion:** exercise that same flow against a remote Vessel,
   with separate enrollment/use permissions, explicit allowlists, private auth view,
   interrupted enrollment/reconnect, deduplicated commands and unsupported-server
   behavior. Design these authority contracts in slice 1; never temporarily expose
   an unrestricted remote enrollment endpoint to get the first slice working.
3. **API bindings and remaining providers:** add execution-host private-terminal key
   enrollment/environment bindings, shared OpenAI API profile transports, explicit
   custom endpoint bindings and safe rotation/replacement. Reuse the picker and
   selection contract; unsupported device providers use the stated CLI alternative.
4. **Complete the user workflows:** remembered Helm choices versus host fallback,
   branch/inheritance checks, full error/empty/offline states, narrow-layout polish
   and all legacy migrations. Basic draft preservation and visible current/next-run
   state are already required in slice 1, not deferred to polish.
5. Verify the full acceptance matrix, update current-operation documentation, publish
   implementation, and separately perform authorized installation/upgrade checks.

The first milestone is two OAuth accounts enrolled from Helm and usable end to end,
not merely multiple credential files or a disconnected picker. Do not call the full
issue complete until remote permissions and API binding workflows are delivered.

## Acceptance and verification

- Enroll two same-provider OAuth accounts from Helm on local and remote Vessels;
  codes appear only in the private auth view and tokens stay on the executing host.
  The CLI alternative uses the same lifecycle. Unsupported device providers offer
  a clear terminal path, not a broken localhost callback.
- Device pending/slow-down/denied/expired responses, view closure, disconnect,
  cancellation/publication races, permission revocation, duplicate commands and
  uncertain exchanges have bounded, accurately reported outcomes. Late completion
  cannot overwrite another account or resurrect cancelled enrollment.
- Two API bindings coexist. Add/cancel, duplicate aliases, rename and explicit
  reauthentication preserve other accounts; one authorized OpenAI API profile can
  serve both native transports without sending credentials to another endpoint.
- Helm selects accounts in drafts and existing voyages; remembered choices affect
  only new drafts and do not change host defaults. Cancel preserves composer text;
  pending creation never changes identity. Invalid preferences require review.
- Two voyages use different accounts concurrently. Staging a switch during a run
  leaves root/subagents/retries on the original identity until the next admission,
  while safe same-identity credential rotation continues to work.
- Model discovery, inference settings, continuation IDs and cached UI replies do
  not leak across accounts. Account/model/override selection is atomic; incompatible
  settings require explicit resolution and rejection leaves all old settings intact.
- Restart, suspend/resume and branch preserve selections and recheck authorization.
  Lost-response retries recover the original receipt; conflicting UUID reuse fails.
- Concurrent refresh issues one coordinated rotation; bounded lock contention,
  refresh failure/uncertainty, logout during refresh and crash recovery never restore
  stale credentials. Already-dispatched requests are reported accurately.
- Remote allowlists filter account labels; account-use alone cannot enroll/replace
  credentials. Newly enrolled accounts do not become usable by unrelated clients.
  Unauthorized selection/enrollment/host-default changes, stale grants, wrong
  host/provider/endpoint, removed accounts and old servers fail without fallback
  or credential disclosure.
- Legacy OAuth cache, logged-out cache, API environment config, native saved launch
  records and interrupted migration remain usable or give bounded recovery guidance.
- Check secret-free diagnostics, terminal-safe labels and protected storage failure
  paths. Existing credentials and user data must not be included in evidence.

Use bounded synthetic credentials and loopback provider fixtures for local runtime
verification; do not recreate the removed broad test/evaluation suites. Build/check
all affected protocol consumers and both binaries at each changed boundary. Actual
OAuth login/refresh with a provider needs approved accounts and budget. Linux storage
checks do not establish native macOS/Windows security. Record skipped checks as gaps,
not passes. Multi-account support is not yet implemented by this planning document.
