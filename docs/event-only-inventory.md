# Event-only interaction inventory (#374)

This is a source inventory and target mapping, **not implemented event-only
behavior**. The core baseline is `a323aad391d9026c646515e235211fffda967d0b`,
with the staged #381 observation additions at `f0bdb093d71ffb5ae4400b7e81fe6b5b1107d662`.
The Web baseline is `706383d8` in `o-psi/webhelm`. Refresh the inventory against
published v1.0.3 before removing routes; its current release candidate contains
additional unpublished operations. Delivery records: [core #374](https://github.com/o-psi/helm.vessel.voyage/issues/374)
and [Web #6](https://github.com/o-psi/webhelm/issues/6).

The authoritative top-level allowlist is
[`VesselCommand`/`VoyageCommand`](../crates/voyage-protocol/src/vessel.rs).
It contains 56 Vessel variants (including the untagged Voyage routing wrapper)
and 42 Voyage variants. The table maps all **97 serialized operations** exactly
once; `Voyage(VoyageRequest)` is routing, not another wire operation. Nested
operations and narrower browser allowlists remain separate checks below.

## Operation map

| Contract / family | Current operations | Required replacement |
| --- | --- | --- |
| VesselCommand / Connection and routing | `Identity`, `TrustVessel`, `Capabilities`, `Socket`, `Granted`, `Grant`, `RevokeGrant` | Typed connection/authority entities and scoped correlated outcomes; wrappers retain their original authority and never become client execution grants. |
| VesselCommand / Catalogue and lifecycle | `Catalogue`, `CatalogueChanges`, `Inspect`, `Restart`, `Stop`, `Recover` | Bounded catalogue/process entities, bootstrap barrier, owner-change/recovery events and exact lifecycle outcomes. |
| VesselCommand / Execution identity | `Execution` | Typed inventory/review/transition entities and pinned outcomes; preserve separate administrator and ordinary-user authority. |
| VesselCommand / Updates | `UpdatePrepare`, `UpdateStatus`, `UpdateApply`, `UpdateDiscard` | Pinned review entity plus durable updater progress/outcome events; independent updater and readiness/rollback receipts survive disconnect. |
| VesselCommand / Accounts and model discovery | `DiscoverModels`, `Accounts`, `AccountUsage`, `AccountSetDefault`, `AccountDefaults`, `AccountModels` | Scoped account/catalogue/default/usage entities; explicit unknown observations; credentials and host configuration remain on the executing host. |
| VesselCommand / Profiles | `Profiles`, `SaveProfile`, `DeleteProfile`, `SetDefaultProfile` | Revisioned profile entities and correlated mutation outcomes; no profile refresh polling. |
| VesselCommand / Launch and account enrollment | `StartSettings`, `StartAccount`, `ResolveStartAccount`, `EnrollAccount`, `ResolveAccountEnrollment`, `CancelAccountEnrollment`, `PrivateAccountEnrollment`, `Start`, `ResolveStart`, `StartConfigured` | Draft/launch/enrollment entities and durable admission outcomes; private sign-in traffic keeps its independent fencing and does not become replayed public state. |
| VesselCommand / Import, branch and transfer | `ManagedImport`, `Import`, `Branch`, `PrepareTransfer`, `ExportTransfer`, `AcceptTransfer`, `TransferChunk`, `UploadTransferChunk`, `ActivateTransfer` | Bounded transfer/import content sequences with exact identities, digests and progress/outcomes; preserve reviewed activation and immutable branch prefix. |
| VesselCommand / Participants and assignments | `AcceptParticipant`, `RemoveParticipant`, `Assign`, `FenceAssignment`, `ObserveAssignment`, `CancelAssignment` | Participant/binding/assignment entities and outcomes; human Vessel grants, scoped process grants and participant bindings remain distinct. |
| VesselCommand / Notifications and socket provenance | `Notifications`, `HostBrowserDisconnected` | Scoped preferences/inbox/delivery events and exact transport provenance. Disconnect notifications stay transport-owned and cannot be forged as human browser actions. |
| VoyageCommand / Retired session observation | `Snapshot`, `Events` | Remove Snapshot and public-v1/public-v2 negotiation/fallback; replace initial attach and retention gaps with bounded typed initialization/replay/reinitialization. |
| VoyageCommand / Canonical content and output | `History`, `MessageChunk`, `RunOutput`, `ReadArtifact`, `UploadImage` | Correlated bounded history/message/output/artifact sequences and upload outcomes; stable canonical indices, UTF-8 offsets and immutable content identity, with no monolithic session payload. |
| VoyageCommand / Admission and exact commands | `Submit`, `SubmitContent`, `Receipt`, `Resolve`, `Cancel`, `Steer`, `Respond` | Typed admission/decision/steering/terminal outcomes tied to durable command IDs. Socket request IDs stay ephemeral; unknown effects are reconciled, never replayed automatically. |
| VoyageCommand / Session settings and lifecycle | `Rename`, `SetAccess`, `Configure`, `SetInference`, `SetAccountInference`, `SetModel`, `Archive`, `Delete`, `Clear`, `Compact` | Revisioned public setting/lifecycle/projection entities plus exact mutation receipts; canonical checkpoints/history are preserved under existing explicit semantics. |
| VoyageCommand / Goals and accounting | `GoalRead`, `GoalReconcile`, `GoalUpdate`, `ProviderAttempts` | Scoped Goal/usage/attempt entities and bounded diagnostics/results; billing and request occupancy stay distinct and private payloads remain excluded. |
| VoyageCommand / Workspace and tools | `WorkspaceChanges`, `Controls`, `OperatorTool`, `ExecuteTool`, `Github` | Bounded workspace/inventory/tool-result entities and exact reviewed execution outcomes; no general executor in Helm or Vessel. |
| VoyageCommand / Workflow and assignment | `WorkflowInputs`, `WorkflowPreview`, `WorkflowSubmit`, `AssignmentObserve` | Bounded workflow/review/assignment entities and outcomes; private workflow values retain their private channel and cannot enter public replay. |
| VoyageCommand / Browser | `HostBrowser`, `PrepareBrowser`, `Browser` | Event-derived revision/incarnation/control fences plus bounded browser status/DOM/control sequences; retain separate consent for existing opt-in local browser execution. |
| VoyageCommand / Terminal and decisions | `Terminal`, `Decisions` | Bounded screen/output/lifecycle and decision entities; private terminal input stays outside public replay and model-visible capture. |

## Nested contracts and authority

The migration must enumerate and preserve each nested action, rather than treating
its enclosing operation as blanket authority:

- `TerminalAction` has Attach, Snapshot, Write and Resize. Replace repeated screen
  Snapshot reads with bounded screen/lifecycle events; Attach/Write/Resize need
  correlated outcomes and original run/terminal/control fences. This terminal
  screen state is distinct from the retired session snapshot route.
- Host-browser operations are defined in
  [host_browser.rs](../crates/voyage-protocol/src/host_browser.rs), with the
  [host-browser protocol](host-browser-protocol.md). DOM snapshots and private
  child-frame mirrors remain supported browser representations. Replace the
  general session Snapshot used for owner/revision discovery; retain localized
  visual fallback, element references, private input and exact action receipts.
- The opt-in local browser contract is
  [browser.rs](../crates/voyage-protocol/src/browser.rs). Its local consent and
  reverse BrowserWork notification are independent of host-browser authority.
- Notifications use [notifications.rs](../crates/voyage-protocol/src/notifications.rs);
  execution reviews use [execution_review_control.rs](../crates/voyage-protocol/src/execution_review_control.rs);
  Goals use [goals.rs](../crates/voyage-protocol/src/goals.rs). Their nested variants
  must become typed entities/results without flattening approval, billing or
  private-input distinctions.
- Browser socket exposure is narrower than the native enum:
  [allowed](../vessel/src/process_http/browser.rs) mirrors Web's `gateway/protocol.js`.
  Do not expand either allowlist just to reuse a common envelope. Native-only
  operations stay native-only unless separately authorized.
- Laravel authentication, tenant isolation and ticket acquisition remain Laravel
  responsibilities. Private RuntimeRequest/RuntimeResponse IPC, on-disk Session
  checkpoints, backups and owner journals are not retired by this cutover.

## Production callers and removal targets

| Surface | Core caller/producer | Web caller | Removal and convergence oracle |
| --- | --- | --- | --- |
| Connection/envelopes | [duplex contract](../crates/voyage-protocol/src/duplex.rs), [Vessel socket](../vessel/src/duplex.rs), [HTTP bridge](../vessel/src/process_http.rs), [Helm transport](../helm/src/process_client/transport.rs) | `resources/js/vessel-client.js`, `vessel-fleet.js` | One versioned typed command/event connection; all reads/mutations have correlated bounded outcomes. Old peers produce an upgrade response, with no silent v1 fallback. |
| Attach and reconnect | [TUI observe](../helm/src/process_client/ui/observe.rs), [reconcile](../helm/src/process_client/ui/reconcile.rs), [plain session](../helm/src/process_client/plain/session.rs) | `resources/react/workspace.ts`, `resources/js/conversation-stream.js` | Remove initial/refresh Snapshot and public-v1 invalidation. Explicit initialization barrier gates actions; replay or bounded reinitialization handles every gap/incarnation change. |
| Catalogue | [TUI catalogue watch](../helm/src/process_client/ui/catalogue_watch.rs), [Vessel service](../vessel/src/process/service.rs), [scoped connection](../vessel/src/process/access/connection.rs) | `resources/js/vessel-fleet.js`, `resources/react/App.tsx` | Remove fallback catalogue polling; scoped initial entities plus ordered changes converge for simultaneous clients and slow consumers. |
| Canonical history, export and navigation | [history](../helm/src/process_client/ui/transcript/history.rs), [navigation](../helm/src/process_client/ui/transcript/navigation.rs), [frontend](../helm/src/process_client/frontend.rs) | `workspace.ts`, `sidebar-actions.js`, `App.tsx` | Replace snapshot-based revision/name/total lookups and large-content reads with correlated paged/chunked entity sequences. Preserve reading anchors and full exact evidence. |
| Live content, tools and command outcomes | [journal observations](../voyage/src/attachment/journal/observations.rs), [TUI reducer](../helm/src/process_client/ui/live_reducer.rs), [live contract](../crates/voyage-protocol/src/live_events.rs) | `conversation-stream.js`, `workspace.ts` and run/decision panels | Replace metadata-only refresh triggers and unknown-event fallback. Sparse cursor order, duplicates, offsets and durable receipts are explicit; no replay of uncertain effects. |
| Accounts/models/profiles/settings | [TUI accounts](../helm/src/process_client/ui/accounts.rs), inference/profile UI, public allowlists | `settings.ts`, `Settings.tsx`, `ProfileActions.tsx`, `InferenceControls.tsx`, `account-enrollment.js`, `NewVoyage.tsx` | Revisioned scoped entities and private enrollment progress; refresh/subscription failures cannot broaden authority or reveal credentials. |
| Goals and execution reviews | [TUI Goals](../helm/src/process_client/ui/goals.rs), identity review controllers | `Goal.tsx`, `ExecutionPanel.tsx` | Exact Goal/review entities and outcomes, with current owner/epoch fences; no inferred automatic continuation or administrator consent. |
| Workspace, artifacts and operator/workflow tools | TUI inspection/operator/workflow bridges, public history/transfer operations | `LiveInspection.tsx`, `WorkspaceFiles.tsx`, `ReviewChanges.tsx`, `ComposerDiscovery.tsx`, `attachments.js` | Replace result-observation refresh timers, canonical fetches and snapshot-based action checks; retain bounded transfer and explicit execution review. |
| Host browser | [Helm browser revision](../helm/src/process_client/transport.rs), TUI browser bridge | `resources/js/host-browser.js` | Remove session Snapshot owner checks; authenticate event fences. Browser DOM/mirror traffic stays bounded and task-host-owned, including private mode and cleanup. |
| Terminal | [terminal client](../helm/src/process_client/terminal.rs), [TUI terminals](../helm/src/process_client/ui/terminals.rs) | Existing narrower browser capabilities | Remove screen/status polling in supported clients; private input, screen revisions, cleanup and exact controls survive reconnect. Do not invent a Web terminal grant. |
| Updates and lifecycle | [TUI updates](../helm/src/process_client/ui/updates.rs), archive/new-draft flows | `VesselUpdate.tsx`, `vessel-update.js`, `sidebar-actions.js`, `new-voyage-delivery.ts` | Remove repeated status/recovery Snapshot calls and progress polling; retain pinned review, exact receipts, updater independence and rollback. |

Every actual caller must be checked during conversion; grouped module pointers are
not proof that the implementation covers all branches. The plain/CLI frontends
(resume, export, workflow and GitHub) count as Helm interactions too.

## Timers and channels

Remove timers whose purpose is state polling or snapshot reconciliation:
Web's five-second fleet/workspace loop, degraded-event refresh, inspection output
refresh and update-status refresh; TUI's catalogue/snapshot/status recovery loops;
and terminal screen polls. Replace them with bounded subscriptions and explicit
error/reinitialization events.

Retain bounded connection deadlines, cancellation, heartbeat/liveness checks,
credential renewal, reconnect backoff, upload limits and local presentation/draft
timers where justified. A repaint timer or clipboard notice is not a state poll.
Server-side bounded journal observation may wait internally; it must emit enough
typed content that a client does not fetch a snapshot to interpret it.

## Known cross-repository delta

Web main adds `workspace_file` in `resources/react/workspace.ts`, guarded by the
advertised capability and workspace-read authority. That operation is absent from
this core baseline's public enum and exists in the still-unpublished v1.0.3 release
candidate. Include its exact final published contract in the bounded workspace
entity/transfer family before implementation. Do not claim the current two main
branches are a fully matched event-only contract.

## Verification record to produce

Record a per-operation completion/removal map against final integrated source.
Trace both clients' transport for initial load, late subscription, long history,
large values, simultaneous clients, duplicate/sparse/reordered frames, retention
gaps, disconnect, owner restart, decisions, cancellation, failed reads/commands,
scoped revocation, private-data exclusion and updater interruption. Zero legacy
session-snapshot calls and zero hidden polling fallback are required, including
non-chat panels and CLI commands. Preserve stable command IDs and reconcile
uncertain outcomes without replaying tools.

This inventory completes source mapping only. Typed schemas, runtime/Vessel
producers, both reducers, route/type removal, tests, final coverage, built/installed
TUI and deployed Web qualification remain #374 obligations. Keep the issue open.
