# Implementation plan: one TUI, independent voyage runtimes

Status: planned, not implemented. Tracks [#77](https://github.com/o-psi/voyage/issues/77),
with runtime delivery in [#78](https://github.com/o-psi/voyage/issues/78) and interface
delivery in [#14](https://github.com/o-psi/voyage/issues/14). This plan implements the
[canonical multiplexing requirement](voyages.md#one-tui-multiple-live-voyages-planned).

The result must be one Helm TUI operating multiple concurrently running voyages,
both local and reached through attached Vessels. Each voyage runtime has its own
process on its executing Helm, independently of the interface. Vessel authenticates
and routes; it does not run agents or hold their provider credentials. Ordinary
local chat needs no Vessel or enrollment.

## Starting point and reusable code

Source paths below are relative to the repository and describe current code.

| Existing boundary | What implementation must change or retain |
| --- | --- |
| `helm/src/tui_runtime.rs`, `helm/src/tui.rs` | Workspace runtimes and active-run admission currently live with the frontend. Replace runtime borrowing with explicit voyage connections; a workspace is not a voyage identity. |
| `helm/src/tui/checkpoint.rs`, `helm/src/tui/bridge.rs` | UI-thread persistence and in-process response channels cannot be the durable runtime boundary. Move checkpoint acknowledgement to the owner; introduce serializable requests and events. |
| `helm/src/attachment/runtime.rs`, `helm/src/attachment/journal/` | Reuse managed admission, revision checks, immutable command receipts, checkpoints, cancellation and cleanup obligations. These are foundations, not a background service. |
| `helm/src/managed.rs`, `helm/src/remote_worker.rs` | Both supply foreground execution paths. Extract their reusable execution lifecycle; the remote worker currently binds one dedicated session. |
| `helm/src/session.rs`, `helm/src/attachment/migration.rs` | Preserve session UUIDs, canonical text and single-writer authority. The explicit transfer library does not yet integrate ordinary frontends. |
| `helm/src/voyage_client.rs`, `helm/src/tui/voyage_setup.rs` | Current client performs read-only presence discovery; setup saves drafts. Neither implements live remote voyage control. |
| `crates/voyage-protocol/src/`, `vessel/src/main.rs` | Extend both ends of negotiated remote contracts, routing and authorization. Keep Helm connections outbound. |
| `helm/src/tui/recent.rs`, `helm/src/tui/terminals.rs`, `helm/src/supervision.rs` | Retain useful presentation, but bind every view and action to its voyage and executing host. |

See [TUI boundaries](tui-architecture.md), [managed ownership](managed-session-owner.md),
[local managed sessions](local-managed-sessions.md), [session transfer](session-journal-transfer.md)
and [dedicated remote sessions](remote-sessions.md) for current guarantees and limits.

## Architectural decisions

- Introduce a transport-neutral voyage connection contract. Local IPC and Vessel
  routing implement it; the TUI never needs a local process handle for remote work.
- Use the managed journal as the common authoritative runtime store. Integrate
  existing saved sessions through explicit, quiescent admission using the existing
  transfer foundation. Preserve exact history, UUIDs and recovery blockers; never
  permit JSON and journal writers to execute the same session independently.
- Extract a reusable runtime host, instantiated once per executing voyage process.
  It owns provider configuration, tool registry, checkpoints, approvals/questions,
  terminals, subagents, todos, budgets, cancellation and cleanup. Shared accounting
  and workspace arbitration must remain process-safe across those instances.
- Add an explicitly started local supervisor that discovers and supervises runtime
  processes. Normal startup must make its background lifetime clear and connect
  to an existing supervisor when present. Service installation and automatic host
  startup are separate explicit operations. Closing the TUI only detaches.
- Separate durable canonical history from authorized event projections. A local
  observer is not automatically allowed to publish private history through Vessel.
  Provider continuation state and credentials remain on the execution host.
- Multiplexing independent voyages does not require coordinator handoff or several
  participant Helms within one voyage. Those remain required, separate work in the
  [coordination design](voyage-coordination.md); connection identities must leave
  room for its coordinator epochs and participant attribution.

## Delivery sequence

Each milestone is a reviewable implementation increment with its own tests. A
local-only increment is useful progress, but does not complete mixed multiplexing.

### 1. Specify and test the connection contract

Owners: #78, #9, #10, #79. Dependencies: none beyond existing contracts.

Define typed capability negotiation, catalogue/snapshot/replay, create/open,
submit/steer, decision response, exact-run cancellation and detach operations.
Define lifecycle commands for rename, branch, archive, confirmed deletion and
next-turn model changes; unavailable operations report explicit capability limits.
Every command carries an immutable operation ID and appropriate session/run,
expected revision, execution incarnation, authority context and expiry. Keep event
cursor, session revision and ownership epoch distinct.

Specify separate run, transport, delivery and cleanup states. An unknown command
outcome is resolved by querying its receipt, never by generating another ID.
Persist acceptance before effects and publish checkpoint events only after durable
commit. Slow observers cannot block checkpoints. Use bounded subscriptions and
snapshot-plus-cursor recovery when retained replay has a gap. Specify ordering,
retention, size limits and overflow responses before freezing the schema.

Exit tests: local and remote adapters run the same contract suite with fakes;
stale revisions/epochs, duplicate IDs with changed payloads, expired requests,
malformed frames, replay gaps and incompatible peers fail without dispatch.

### 2. Extract authoritative per-voyage execution

Owners: #78, #5, #21. Depends on milestone 1.

Move run admission, checkpoint persistence, steering delivery and title/model
updates from the TUI into the runtime owner. Use one execution lease and one active
mutating run per voyage, including CLI clients. Reuse managed cleanup registration
before resource construction and observed cleanup before admission is reopened.
Provide a runtime event sink independent of whether a UI is attached.

Integrate ordinary new/resumed chat, plain chat and run with the common owner.
Explicitly handle busy saved sessions, failed transfer/publication, ambiguous tool
outcomes and preserved canonical text. A read-only catalogue must not create a
session, transfer authority or start a provider. Configuration drafts stay drafts
until a successful explicit create/open operation returns a real session identity.

Exit tests: run without a TUI and persist streamed output; kill at admission,
checkpoint and cleanup boundaries; contend from two processes; reopen after a
failed write; verify exact text, receipt identity and no replay of uncertain tools.

### 3. Add independent local processes and supervision

Owners: #78, #18, #71. Depends on milestone 2.

Introduce the runtime executable mode and supervisor control surface. Use private,
authenticated local IPC: Unix sockets with peer/ownership validation on Unix and
named pipes with native access checks on Windows. Authenticate process incarnation,
not PID alone. Serialize concurrent starts and reject stale endpoint impersonation.
Do not expose a local TCP task port as a substitute for private IPC.

Define explicit start, inspect, detach, cancel-run and stop-runtime semantics.
Supervisor lifetime must survive terminal hangup and TUI exit without inheriting
its terminal descriptors. Persist nonsecret runtime registrations and operation
receipts. On restart, reconcile actual owners and resource obligations before
allowing new work; neither a stored PID nor a heartbeat timeout proves survival
or cleanup. Do not automatically restart and replay an interrupted run.

Give terminals and subagents explicit voyage/run ownership. Audit existing
workspace-keyed stores and completion locks so two voyages cannot overwrite each
other's inventories or circumvent shared workspace writer arbitration. Preserve
safe refusal for conflicting work; offer explicit worktree selection rather than
silently changing roots. Enforce host-wide runtime/resource caps and shared budget
accounting across processes, with visible bounded refusal when capacity is full.

Exit tests: two voyage PIDs distinct from the TUI; concurrent execution in separate
workspaces; same-workspace conflict refusal; close/kill/reopen TUI while both work;
kill one runtime without stopping the other; supervisor restart, PID reuse, startup
races, forged IPC peers and capacity exhaustion. Observe cleanup rather than infer it.

### 4. Make the TUI a multiplexing client

Owners: #14, #78, #32. Depends on milestones 1–3; develop against fake remote
connections until milestone 5 integrates the real transport.

Replace the borrowed active runtime with a bounded collection of voyage connections
and per-voyage view state: composer draft, scroll position, unread cursor, activity,
pending decisions and terminal attachment. Merge local and authorized Vessel
catalogues while retaining origin, coordinator and freshness attribution. Switching
views is allowed during work and never issues cancellation.

Target input, steering, approvals, questions, model/policy changes, terminal input
and cancellation using the identity captured when the action was prepared. A late
reply cannot act on the currently selected voyage instead. Preserve drafts after
refusal or disconnect. Display pending decisions for background voyages and allow
navigation to them without resolving a different voyage's modal. PTY input remains
a separate private channel; detach before switching its input target.

Exit tests: start two voyages, switch while both stream, return to preserved drafts
and scroll positions, answer a background decision, and reconnect to missed output.
Test rapid switching with delayed responses, Unicode/paste, narrow/resized terminals,
notification fairness and a flooded background stream without starving controls.

### 5. Route multiple authorized remote voyages through Vessel

Owners: #9, #10, #78, #79, #14. Depends on milestones 1–3; integrates milestone 4.

Extend the outbound Helm router to dispatch by authorized session and runtime
incarnation instead of its current singleton binding. An executing Helm supervises
its voyage processes; Vessel routes bounded requests and events to those owners.
Add real authenticated catalogue, open/create and history projection support with
explicit local authorization. Enrollment or machine discovery must never imply
sharing existing sessions, granting roots or selecting provider credentials.

Implement the remote connection adapter and scoped credential lifecycle. Recheck
current authority on catalogue, snapshot, replay, mutation and event publication.
Negotiate capabilities so older peers retain their existing bounded behavior.
Handle stale connection generations, duplicate operations, reconnect and grant
withdrawal across both transport ends without exposing private state.

Distinguish interface-to-Vessel disconnect from execution-host-to-Vessel authority
loss. Interface detach must not cancel the runtime. Current dedicated workers
cancel conservatively on transport loss: do not silently relax that rule. The new
runtime must implement the authorized lifetime/lease contract, fence new work
when it cannot establish authority, and expose unknown enforcement/cleanup states.

Exit tests: one local and two remote voyages across real Helm/Vessel processes;
concurrent progress, exact routing and replay; interface disconnect/reconnect;
worker link loss, revoked sharing, expired authority, incompatible versions and
unauthorized catalogue/history requests. Verify native providers remain host-local.

### 6. Complete decisions, lifecycle and operational delivery

Owners: #21, #72, #18, #79, #14. Depends on milestones 4–5. Decision protocol and
service design should start with milestone 1 so they do not require a late redesign.

Replace process-local approval/question response channels with durable, exact-target
requests. Bind each answer to the request, run, authority and expiry; authorize the
responder at the executing Helm. Concurrent interfaces cannot answer twice. An
absent interface must produce a bounded policy-defined pending/refused outcome,
never indefinite unattended waiting or implicit approval. Notifications carry
minimal metadata; opening one still requires current authorization.

Finish session lifecycle controls from milestone 1 under runtime ownership,
including confirmations and quiescence for destructive actions. Policy changes
must retain resource cleanup and authority-ceiling checks. Explicitly report any
unsupported terminal/provider capability; existing remote restrictions on MCP and
the compatibility bridge cannot be removed until their cleanup is observable.

Ship supervisor/service setup and diagnostics for supported platforms, secure
credential access without the TUI environment, resource/log limits, explicit
shutdown/recovery operations and documentation. Keep installer previews honestly
labelled until provisioning really executes. Native service/security tests are
required where platform support is claimed.

Exit tests: two interfaces race a decision, answer after cancellation/revocation,
restart with a pending decision, disconnect all observers, full lifecycle actions,
secret-canary checks and platform service stop/start with real resource ownership.

## Integration gates and completion

Use deterministic native-provider fixtures during development; no paid/live model
calls are needed to prove process, transport or UI isolation. Add multi-process
system fixtures with controllable barriers around persistence and dispatch, rather
than timing-only sleeps. Keep private operator data out of fixtures and artifacts.

For each runtime delivery run focused Rust and system tests, then the complete
[`./scripts/check-quality`](quality.md) suite from a clean committed checkout.
Record the exact revision, test results and retained evidence. Linux results do
not establish native macOS/Windows security or service behavior. Approved live
provider and deployment evidence remain separate gates under #18, #22 and #83.

The final acceptance scenario is: start a local voyage and a Vessel-routed voyage
from one TUI; see distinct execution processes; switch and steer both while they
work; resolve a background decision; exit and reopen the interface; recover output
without resubmitting actions; cancel only one run; verify observed cleanup while
the other continues. Repeat with runtime crash, network loss and withdrawn consent.
All six milestones and their applicable failure gates are required to call the
multiplexing workflow complete. This plan alone closes none of its runtime epics.

Implementation should begin with milestones 1 and 2. Milestone 3 establishes the
local process boundary; milestone 4 makes that useful in the TUI; milestone 5 proves
the same abstraction across Vessel; milestone 6 completes operational and decision
behavior. Track these milestones in the existing issues before code changes and
split bounded implementation issues as needed, preserving this dependency order.
