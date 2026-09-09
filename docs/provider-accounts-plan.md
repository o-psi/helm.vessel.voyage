# Native provider accounts

Status: proposed implementation plan; not current functionality.
Tracking: [#213](https://github.com/o-psi/voyage/issues/213).
User requirement: support multiple accounts for one provider; select/configure the
account in Helm. Login may remain a local `vessel` command for this release.

## Product decisions

- Helm selects accounts. Vessel's local auth CLI enrolls and manages credentials on
  the execution host. Voyage resolves credentials and performs inference there.
- Support all four native transports: OpenAI Responses, OpenAI Chat, ChatGPT OAuth,
  and Anthropic. API billing and subscription access remain distinct. Do not restore
  the removed external Codex bridge.
- One explicit account is bound to each voyage's next-run configuration. A running
  execution retains its admitted binding. Different voyages may use different
  accounts concurrently, including accounts of the same provider.
- Defaults apply to new drafts/voyages only. Changing a default never retargets an
  existing voyage. No automatic fallback or rotation on expiry, quota, or failure.
- Account selection must work for authorized local and remote Helm connections.
  Credentials never cross a Vessel connection, enter model context, or appear in
  public snapshots, histories, command receipts, or diagnostic output.

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

Introduce an execution-host account registry in protected private storage. Each
record has an immutable UUID, user-assigned unique alias, display label, allowed
native transport(s), endpoint binding, credential kind, lifecycle state, and record
revision. Labels and aliases are bounded and terminal-safe. UUIDs are host-local:
Helm keys references by authenticated Vessel identity plus account UUID, not label
or route index. Rename never changes identity; deleted UUIDs are not reused.

Credentials live separately from safe account metadata. OAuth records hold tokens
and verified provider identity privately. API records bind either a privately
entered/stored API key or an explicitly named execution-host environment variable.
A binding is not proof of provider account identity, available credit, or readiness.
Do not infer that two API keys belong to different accounts or organizations.

OpenAI API credentials may explicitly permit both Responses and Chat on the same
bound endpoint. Custom compatible endpoints require deliberate bindings. Never
send a selected key to a new endpoint merely because the transport is compatible.
Changing credential identity, endpoint, or allowed transports requires a reviewed
replacement or new record, not an unnoticed edit under a friendly label.

Keep separate versions for metadata edits, login/credential replacement, and OAuth
refresh rotation. Account UUID and login generation identify the security/billing
binding. Token refresh revisions coordinate writes without pretending a routine
refresh is an account switch. A metadata rename must not invalidate in-flight work.

Storage operations must be bounded, private, ownership checked, symlink/hardlink
safe, atomic, and crash recoverable. Reserve enrollment identity before browser or
network effects. Pending enrollment is not selectable. Failed/cancelled enrollment
must not overwrite a working account or leave a partially published selectable one.

## Vessel enrollment and lifecycle

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

Logout tombstones that account and advances its login generation before reporting
success. All voyages check current credential generation/state before subsequent
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
If none exist, explain which Vessel host needs `vessel auth` enrollment; Helm does
not perform login or accept secrets in this release. Offline lists are cached
observations, not permission or confirmation that a selection can be applied.

For an existing voyage, show a short confirmation naming old/new accounts and
explaining that subsequent inference may send retained conversation/tool context
to the newly selected account and its organization. Never imply switching wipes
history. Keep the model and inference overrides when valid; if catalog evidence
shows incompatibility, offer explicit model/override changes atomically with the
account switch. Unknown support remains unknown, not a fabricated validation pass.

### Defaults and drafts

Helm offers **Use for this draft/voyage** and **Make default for new voyages** as
distinct actions. Default scope is execution Vessel + workspace + transport, stored
on the execution host; only the host owner may change shared defaults. Scoped
clients may choose an allowed account for their draft/voyage but cannot alter other
users' defaults. No implicit "last used" override of these defaults.

Resolve a default to a concrete UUID when a draft is initialized and show that
selection. Admission validates it again. A later default change, reconnect, or
registry reorder cannot silently retarget a saved/pending draft. Changing the
selected Vessel clears/re-resolves host-local account selection with visible review.
Submitting a pending create operation preserves its original account identity.

### Running voyages

Allow selecting the account for the **next run** while the current run continues.
Display both "Running with Personal" and "Next run: Work" until the run boundary.
The active root agent, its subagents, retry attempts, and title generation performed
for that run retain the admitted binding. New subagents of that run cannot pick up
staged next-run settings. If logout invalidates that binding, fail subsequent
requests rather than switch accounts. A later run adopts the committed selection.

Selection survives suspend/resume and restart. A branch inherits the selected
next-run account, subject to current authority and availability; it does not gain
access by inheriting a reference. Old history retains safe historical attribution.
Never reuse provider continuation/response IDs across account bindings; build the
new request from canonical conversation under the new account.

## Protocol, authority, and persistence

Add capability-advertised account enumeration and account-aware creation/configuration
commands. Return safe descriptors and observed registry revisions only. Use an
atomic next-run inference selection envelope containing account UUID, expected
login/record versions, model, and overrides, with stable command UUID and expected
session/configuration revision. Validate account/endpoint/transport compatibility
and caller authority before durable acceptance. Recheck binding at run admission.

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
Account enrollment, credential replacement, shared defaults, and registry-wide
administration remain host-owner operations. Model-facing tools gain no implicit
account enumeration or selection powers from the human Helm feature.

## Refresh, caches, and migration

Use per-account cross-process locking with bounded waits and version-checked atomic
refresh publication. Readers re-read after taking the lock so independent voyages
reuse a completed rotation. An uncertain refresh response is not blindly replayed;
retain a pending/uncertain state and require safe recovery or reauthentication.
No global refresh lock that serializes unrelated accounts. Resolve the token store
through the account binding for both inference and discovery.

Scope catalog observations to account UUID/login generation, endpoint, transport,
and relevant config. Invalidate on switch, replacement, or logout; ordinary token
rotation preserves account identity. Async picker/catalog replies carry the exact
selection context so old results cannot overwrite the newly selected account.

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

1. Define registry/credential binding types, versions, migration and storage
   transactions. Specify safe public descriptors and command/capability contracts.
2. Implement Vessel CLI enrollment for OAuth and API bindings, account lifecycle,
   migration, cross-process refresh/replacement/logout coordination and recovery.
3. Wire Voyage provider construction, discovery, admitted binding snapshots,
   continuation isolation, persisted selection and safe historical attribution.
4. Implement public/private selection and creation contracts, account allowlists,
   authorization, atomic defaults, receipts, revocation and old-server behavior.
5. Add Helm account control for draft/live flows, confirmation, pending/active
   display, default action, remote filtering and reconnect recovery.
6. Verify the complete workflows, update current-operation documentation, publish
   changes, and separately perform any authorized installation/upgrade checks.

Ship an end-to-end slice before calling this supported: two OAuth accounts enrolled
through Vessel and selectable in Helm, including persistence and run-boundary
semantics. The full issue also requires API bindings and authorized remote selection;
those are not silently deferred by completing the first slice.

## Acceptance and verification

- Two same-provider OAuth accounts coexist; two API bindings coexist. Add/cancel,
  duplicate aliases, rename and explicit reauthentication preserve other accounts.
- Helm selects accounts in drafts and existing voyages; changing defaults affects
  only new drafts. Cancel preserves text; pending creation never changes identity.
- Two voyages use different accounts concurrently. Staging a switch during a run
  leaves root/subagents/retries on the original account until the next admission.
- Model discovery, inference settings, continuation IDs and cached UI replies do
  not leak across accounts. Incompatible settings require explicit resolution.
- Restart, suspend/resume and branch preserve selections and recheck authorization.
  Lost-response retries recover the original receipt; conflicting UUID reuse fails.
- Concurrent refresh issues one coordinated rotation; bounded lock contention,
  refresh failure/uncertainty, logout during refresh and crash recovery never restore
  stale credentials. Already-dispatched requests are reported accurately.
- Remote allowlists filter account labels; unauthorized selection/default changes,
  stale grants, wrong host/provider/endpoint, removed accounts and old servers fail
  safely without fallback or credential disclosure.
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
