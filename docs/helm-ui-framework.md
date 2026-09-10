# Helm-specific UI framework: audit and replacement design

Status: **target design, not implemented**. The audit was performed on the dirty
checkout based on `61e70cb` on 2026-09-10. Source references describe that observed
checkout, not an immutable release. Tracking: [#216](https://github.com/o-psi/voyage/issues/216).
[Architecture](architecture.md) remains canonical; [current state](current-state.md)
describes shipped source behavior.

## Decision

Replace Helm's UI orchestration with an internal, Helm-specific application layer
on Ratatui. Replace ownership boundaries, not merely filenames. Keep Ratatui as the
renderer and preserve useful editor, transcript, transport, storage and privacy
algorithms. Do not build a public UI toolkit, virtual DOM, plugin framework, second
executor or browser frontend.

The requested rip-and-replace means one final architecture and removal of the old
orchestration at cutover. Intermediate buildable work is useful; a pilot component,
permanent compatibility wrapper around `App`, or two shipping event loops is not
completion. The no-application-rewrite constraint in the completed component
adoption #196 applied to that earlier delivery, not this new user request.

## Audit scope and method

Inspected the connected UI, supporting composer/Markdown/theme/clipboard code,
transport and terminal boundaries, frontend entrypoints, persistence/recovery, and
current architecture/component/interaction/readiness documentation. Parallel
read-only inspections covered input/rendering and asynchronous workflows.
Reviewed relevant open and closed issues, including #84, #196, #211, #14, #203 and
#213. Historical test results were not treated as present-day evidence.

At inventory time `helm/src/process_client/ui/` contained **57 Rust files and
16,278 physical lines**, including comments, blank lines and remaining inline
tests. **37 files contained 42 `impl App` / `impl super::App` blocks**; `App` had
34 fields. These are structural counts, not executable LOC, measured complexity
or proof that every large file is poorly designed.

The checkout contains unrelated protocol/runtime/account work, deleted scripts
and tests, and staged rendering comments. They are not part of this audit's
changes. Account work [#213](https://github.com/o-psi/voyage/issues/213) must be
reconciled before an implementation cutover rather than overwritten. During the
audit, independent offline-Vessel work under #217 also changed UI routing files.
Line references below are locators in the observed audit baseline; use the named
functions as well when reviewing the later tree. No #217 changes are ours.

This is a deep architectural source audit, not an exhaustive security review of
every workspace crate. No runtime defect reproduction, latency measurement,
provider call or native-platform certification was performed in this pass.

## Findings

All paths in this section are relative to `helm/src/`.

### 1. Feature files still share one application object — highest priority

`process_client/ui/mod.rs:50–84` combines connection/task ownership, all voyage
views, new drafts, clipboard state, overlays, inference controls, sidebar state and
terminal requests. Feature implementations across input, actions, inference,
sidebar, transcript, paste and lifecycle mutate that same object.

`process_client/ui/state.rs:205–225` similarly combines remote observations,
composer/images/history, pending commands, overview and terminal browser state,
transcript projection, result-retention timing and render caches in `View`.

**Consequence:** file boundaries do not enforce responsibility boundaries. A
feature change can require knowledge of unrelated overlays, persistence and
selection. This is the same class of coupling that the historical #84 file
extraction sought to reduce, now in the connected-process frontend.

**Replacement:** feature-owned state with private fields and explicit messages;
shared observations and durable operations have separate owners. A feature may
not receive `&mut App`, a mutable registry of every feature, or a global service
locator that recreates those permissions.

### 2. Input ownership is an ordered list of exceptions

`process_client/ui/input.rs:4–172` runs private Vessels input, workspace input,
paste, divider drag, inference, previews, new drafts, interactions, transcript,
sidebar, help, completion and terminals in a precedence chain. Interaction
synchronization is repeated during a single input event. Individual handlers
implement their own global-key exceptions and suppression checks.

`process_client/ui/completion.rs:140–155` decides whether completion is allowed
by inspecting sidebar focus/menu, help, explore, overview, terminal browser,
interactions and composer position. `new_draft/input.rs:34–94` separately handles
quit, new, help, archives, navigation and submission.

**Consequence:** opening a new feature requires auditing other handlers' exclusion
lists. A `bool` return says only consumed/not-consumed, not which scope owns the
event or which intentional application action should follow.

**Replacement:** explicit exclusive input scopes, modal precedence, pointer
capture and a documented global-command policy. Share editor mechanics without
flattening the different first-send and existing-voyage delivery policies.

### 3. Rendering performs state transitions, not only drawing/cache updates

`process_client/ui/render.rs:135–154` resets several hit collections and invokes
`sync_interactions()` while rendering through `&App`. `sidebar/menu.rs:118–142`
consumes follow-selection state and writes scroll position. These behaviors
matter; removing them without a replacement would lose focus semantics. Result
retention now uses timestamps rather than rendered/visited acknowledgement.

**Consequence:** event behavior depends on what happened to render, with mutable
state hidden behind shared references. Render-local mutation itself is not a bug;
the problem is no explicit boundary between layout caches, presentation evidence
and application transitions.

**Replacement:** explicit prepare/layout, paint and successful-presentation
phases. Paint may update isolated render caches and construct hit maps, but cannot
send requests, save drafts or alter focus. A successfully presented frame can
return non-sensitive presentation facts; result settling remains time-based and
independent of frame exposure.

### 4. Async task ownership is inconsistent

`process_client/ui/routes.rs:91–107` retires tracked observers and route tasks;
`mod.rs:261–279` aborts and awaits those handles at exit. In contrast,
`controls.rs:26`, `export.rs:18`, `completion.rs:206`, inference discovery and
transcript hydration contain spawned work whose handles are not retained in
those registries.

**Consequence:** stale-result rejection can protect presentation without proving
the underlying work has stopped. In particular, an export is a local filesystem
effect, not merely a discardable read. These are observed ownership gaps, not a
claim that a Tokio task survives process exit.

`routes.rs:194–205` also waits for the single global retired-observer list to empty
before completing pending activations/disconnections. This couples independent
connection cleanup rather than expressing a per-connection completion barrier.

**Replacement:** one effect supervisor with typed lifetimes, bounded admission,
retained handles and observed cleanup. Distinguish cancellation of observation,
local acquisition and requests from cancellation of an admitted voyage run.

### 5. Target fencing is stronger than request-instance fencing

Controls and export both emit `Update::Control` carrying only target,
incarnation and result (`observe.rs:60–64`). The handler checks incarnation, then
sets `view.panel` and resets scrolling (`updates.rs:149–176`).

**Source-reachable race:** request overview A, then B on the same target and
incarnation; A can finish last and replace B. `/conversation` clears `view.panel`
(`actions.rs:147–149`), but a delayed overview reply can reopen it. This follows
from the inspected control flow; no interactive reproduction is claimed.

Other features already have more specific fencing: history revision, live-text
run/offset/size, picker generations and exact command IDs. Preserve that strength
and apply it consistently, rather than using one permissive generic response.

**Replacement:** request identity and feature-instance identity in addition to
route/session/incarnation. Closing or replacing a transient panel invalidates its
request instance. Durable command observations must still be retained even when
the initiating panel is gone.

### 6. Responsiveness work belongs in this architecture

Every accepted update marks every transcript dirty (`updates.rs:37–40`), even
when it is an unrelated connection or picker result. The main loop polls clipboard,
previews, completion and transcript preparation before selecting its next event,
and redraws on a 100 ms interval (`mod.rs:219–241`).

Existing-voyage editing calls `drafts::save` on the input path
(`input.rs:246–247`). That save clones/serializes image-bearing state, writes a
temporary file, synchronizes it, publishes it and synchronizes the directory
(`drafts.rs:136–156`). New-draft storage also performs synchronous publication
(`new_draft/storage.rs:62–76`).

**Consequence:** unrelated updates can cause projection work; filesystem latency
can delay input. These costs were identified statically, not measured here.

**Replacement:** precise feature invalidation and a deadline-driven redraw
scheduler; serialized, tracked draft persistence outside input handling. A
persistence acknowledgement must identify the draft generation actually saved.
Submission must wait for durable intent publication. Do not solve latency by
silently weakening first-send recovery or declaring unflushed edits durable.

### 7. Commands and selection have multiple representations

Slash discovery is a table in `completion.rs:13–118`; dispatch branches live in
`actions.rs`, `controls.rs`, `new_draft/input.rs` and inference parsing. Sidebar
buttons ultimately reach command handlers, with preservation flags affecting
whether composer text is consumed. Navigation directly assigns selection in
several handlers (for example `input.rs:113–130` and `actions.rs:96–110`).

**Replacement:** a Helm action catalogue for command IDs, labels, keybindings,
discovery and enabled reasons. It emits typed intents and delegates validation
to feature/domain handlers. Do not put every business transition in this catalogue.
Use one navigation transition for preserving drafts, closing transient scopes,
invalidating geometry and recording departure. Keep unknown wire states explicit;
do not convert them to Ready through a convenient default enum variant.

### 8. The private terminal is a separate operating mode

`process_client/ui/mod.rs:243–255` drops the normal event stream, suspends previews
and awaits terminal attachment before restoring Helm. The private driver in
`process_client/terminal.rs` has its own screen guard, event reader, sequential
bounded write queue and observation task. It fences session incarnation, rejects
queue overflow and never retries uncertain input writes.

At `terminal.rs:186–189`, the observer is aborted and awaited; the writer is
aborted but is not awaited on all exit paths. This is a cleanup-observation gap,
not evidence that private input leaked or was replayed.

**Replacement:** explicit normal/private-terminal/restoring/shutdown modes and
exclusive terminal ownership. Private bytes must not pass through a loggable
application event bus, composer, debug serialization or model-visible history.
Await owned writer cleanup before reporting handoff completion. Keep richer
terminal cells/input fidelity under [#32](https://github.com/o-psi/voyage/issues/32);
a UI framework does not implement missing protocol data.

## Additional correctness findings from the deep audit

These are source-derived paths/interleavings, **not runtime reproductions**. They
are prioritized requirements for the replacement, not reasons to preserve the
current behavior as parity. Each was cross-checked against the relevant source.
Paths beginning `ui/` below mean `helm/src/process_client/ui/`.

### Highest priority: recovery records must not be disposable UI state

- **Failed recovery can later overwrite its source.** `ui/updates.rs` inserts a
  writable `View` after `drafts::load` fails (baseline lines 203–207). Editing or
  ordinary exit then calls `drafts::save`; that replaces the same recovery file
  (`ui/drafts.rs:118–156`, `ui/mod.rs:258`). A corrupt or unsupported record can
  lose text and pending identity after initially being described as preserved.
  Decode into temporary state; failed recovery must remain non-writable until an
  explicit, preserving recovery decision.
- **Existing-voyage draft files have no lifetime writer lock or CAS revision.**
  Their key is connection/session and save replaces the file. Two Helm windows
  can overwrite composer/pending state. New-draft locks already establish a useful
  different pattern (`ui/new_draft/storage.rs:46–59`). Protect operation records
  independently of replaceable editor saves, and define cross-window ownership.
- **Disconnected jobs can retain recovery capacity.** `dispatch()` stores `None`
  in `command_checks` while in flight; disconnect aborts the task but does not
  release that entry. Reconciliation excludes disconnected targets while counting
  their in-flight slots toward its global limit (`ui/reconcile.rs:21–57`). Four
  such entries can starve other routes. An unresolved operation is not a running
  task: retain the former, release capacity when the latter is observed stopped.
- **Late-receipt storage is write-only in the inspected Helm source.**
  `routes::retain_receipt` writes `helm-command-receipts`, but source search found
  no recovery reader. Repeated writes replace the same command file, including
  weaker `unknown` observations. Ingest authenticated outcomes before presentation
  filtering; validate exact identity and retain stronger evidence monotonically.

### High priority: the visible surface must own all input

- **Search paste targets the composer.** `ui/input.rs:16–19` invokes paste before
  transcript search. `paste_destination()` excludes several overlays but not
  search (`ui/paste.rs:93–112`); ordinary paste edits/saves the composer
  (`:232–263`) before search's own handler (`ui/transcript/input.rs:30–67`). This
  does not automatically send the text. Resolve one active editor for both typing
  and paste before acquiring, mutating or persisting anything.
- **Help/Explore can cover an active new draft.** Ctrl+N precedes Help handling;
  `select_draft()` clears selection/menu/completion but not Help/Explore
  (`ui/new_draft.rs:306–313`). Rendering still prioritizes those screens
  (`ui/render.rs:168–198`), while `new_draft_input()` receives typing/Enter first.
  After successful creation, unseen text can reach the draft's send path. Make
  navigation, visibility and input ownership one transition.
- **Undersized rendering is not an input mode.** Below 40×18 the renderer displays
  an enlargement notice (`ui/render.rs:159–166`), but normal Enter/new-draft Enter
  handlers can still dispatch when no other scope consumes the event. Preserve
  text but disable hidden consequential actions in an explicit undersized scope.
- **Result review tracking was removed.** Time-based settlement now replaces
  rendered/visited acknowledgement. Overlay exposure and navigation no longer
  affect voyage result retention; this remains presentation-only state.

### Medium priority: scopes need explicit retirement and navigation

- **History loading can stick after explicit disconnect/reactivation.** History
  jobs set loading flags but have detached handles (`ui/transcript/history.rs:
  138–166,181–213`). Reactivation moves the existing view to the new generation;
  its old reply is discarded, and an unchanged incarnation does not reset the
  loading flags. Derive loading from live task tokens and reset attempts when the
  owning observation scope retires. Same-generation observer retry introduced by
  concurrent #217 is a separate path, not proof this reactivation path is fixed.
- **Expired decision review can trap ordinary navigation.** Response eligibility
  also gates switching requests, and the disabled return precedes Esc handling
  (`ui/interactions/input.rs:39,131–179`). Focused review consumes ordinary input.
  Preserve navigation and dismissal when responding is forbidden; do not make
  expiry a permission to respond or silently deny on behalf of the user.
- **Terminal-browser keys depend on prior sidebar focus.** Sidebar input precedes
  terminal input; it yields for an open browser only under composer focus
  (`ui/sidebar/input.rs:74–122`). F3 opens the browser without setting a browser
  focus owner (`ui/terminals.rs:190–193`). Up/Down can switch voyages instead of
  programs. Opening/closing the browser must transfer/restore focus explicitly.
- **Geometry and global keys are not uniformly preprocessed.** Workspace-picker
  input consumes events before central quit handling, and does not clear its hit
  map on Resize before the next draw (`ui/new_draft/workspaces.rs:150–235`). This
  creates inconsistent quit behavior and a stale-hit window. Transcript wheel
  handling also ignores pointer location (`ui/transcript/input.rs:127–145`).
  Route mouse input from the final screen plan and invalidate geometry first.
- **Startup errors can bypass explicit observer cleanup.** Observers start before
  fallible recovery/initial creation (`ui/mod.rs:202–216`), outside the later
  cleanup block. A dropped JoinHandle detaches. Establish the task/screen guard
  before the first owned effect, not only around the main event loop.

### Further presentation and transport work

- Transcript search matches wrapped rows (`ui/transcript/layout.rs:502–512`), so
  a phrase spanning a wrap is missed and results can depend on terminal width.
  Search logical text and map matches to rows. Retain anchors, improving their
  source-position mapping rather than replacing them with scroll offsets.
- Action fields use `Composer` but manually move cursors by scalar boundaries
  (`ui/sidebar/input.rs:293–354`); search uses a `String` with `.pop()`. Share
  bounded editor adapters and grapheme semantics, but keep private fields separate.
- Local SSE establishment has a connection timeout but no response-header deadline
  around `.send().await` (`helm/src/process_client/local.rs:175–187`). Decoder
  stall bounds apply only after response establishment; remote SSE has a separate
  establishment bound. An accepted socket that withholds headers can strand the
  local observer. No hang was reproduced here.
- Within one route, observation refresh awaits snapshot/terminal inventory inline
  (`ui/observe.rs:148–153,214–226,289–324`). Bound and schedule per-session reads;
  do not claim route-independent observers eliminate within-route blocking.
- Local model discovery creates a temporary supervised voyage and discards it at
  the async tail (`helm/src/process_client/frontend/models.rs:7–24`). Treat this as
  resource-bearing work: aborting a discovery future is not observed cleanup of
  that temporary process. During private attachment, the normal loop also stops
  draining its bounded update channel; specify coalescing/backpressure explicitly.

### Backend boundary finding: private-input cutoff needs synchronization

`voyage/src/tools/process.rs:574–590` marks the writer private before returning a
human input handle. In `voyage/src/tools/process/input.rs:165–180`, a model write
checks the atomic flag separately from writing its next (up to 1,024-byte) chunk.
An already-running writer can pass the check, pause, and write that chunk after the
private flag is set. A flag alone does not establish writer quiescence at attach
acknowledgement. This is a source-level input-ownership race, not reproduced
private-output leakage; output capture has a separate lock/cutoff.

Coordinate correction and verification with #32. A Helm-only task join cannot
retract bytes already admitted remotely. Do not claim a stronger exclusive-input
guarantee until the runtime barrier is implemented and observed. Richer terminal
cells, child-mode paste/key fidelity and native behavior remain separate existing
#32 acceptance, not delivered by this audit.

## What to keep, what to replace

| Keep the algorithms and contracts | Replace the orchestration around them |
| --- | --- |
| `composer.rs`: grapheme editing, bounded undo, UUID-owned markers and paste anchors | Repeated new/live editor dispatch and cross-feature suppression |
| `markdown.rs`, `theme.rs`: bounded sanitizing rendering, semantic styles and passive color adaptation | Global redraw/invalidation and implicit layout side effects |
| `ui/transcript/`: logical message/tool/run anchors, hydration, search and canonical/live reconciliation | Network tasks and application access inside transcript handlers |
| `process_client/transport.rs`, `sse.rs`, `connections/`: bounded public API, connection pinning and no implicit mutation retry | Scattered request scheduling, busy flags and task lifetimes |
| `ui/drafts.rs`, `new_draft/storage.rs`, launch and receipt rules | Disk I/O on input paths; workflow state spread across UI objects |
| `ui/right_panel.rs`, Vessel typed form focus and zeroizing fields | Parallel overlay flags and per-feature hit-map clearing rules |
| Preview worker limits, clipboard process cleanup, authorized attachment upload | Cross-feature acquisition decisions and terminal-output ownership |

Helm currently re-exports many Voyage types in `helm/src/lib.rs`; that is compile-time
coupling, not evidence of an embedded agent loop. The UI layer should depend on
narrow protocol/read-model/application-service interfaces. Removing every runtime
crate dependency is not a prerequisite and should not expand this into a backend
rewrite. Plain/CLI clients stay supported and must not acquire a Ratatui dependency
through shared application services.

## Proposed structure and contracts

The following modules and names are **proposed**, not current paths/APIs:

```text
helm/src/ui/
  shell/          terminal ownership, event pump, scheduling, shutdown
  app/            navigation, shared observations, action routing
  effects/        task supervisor, request identities, deadlines, completion
  layout/         regions, clipping, frame hit map, focus and pointer capture
  components/     narrow shared forms/buttons/pickers/panel chrome
  features/
    voyages/      sidebar, filtering, selection, result presentation
    conversation/ transcript and per-voyage presentation
    compose/      editor, images, paste and preview presentation
    drafts/       new-voyage setup and first-send workflow
    delivery/     durable pending operations and outcome reconciliation
    settings/     inference/access; integrate account work without duplication
    interactions/ questions, permission review and confirmation
    vessels/      connection forms, workspace selection and recovery
    actions/      lifecycle review and overview panels
    terminals/    inventory and private-terminal handoff intent
```

Use concrete structs/enums first. Shared component interfaces should emerge from
real reuse, not require a trait implementation for every widget. A small top-level
message router is expected; an all-knowing reducer containing feature internals is
not. Module privacy should make peer mutation impossible without changing the API.

### State ownership

- `Navigation`: one active destination (`Empty`, `Draft(id)` or `Voyage(key)`),
  focused region, explicit undersized mode, one exclusive modal scope, and
  explicitly permitted side panels. Visibility and input use the same screen plan.
- `Observations`: authenticated catalogue/snapshot/history data keyed by immutable
  connection and session, with activation generation and incarnation fences. This
  is a read model, never a second canonical conversation.
- `ComposeSession`: editor, image ownership, prompt recall and save generation for
  one draft/destination, with explicit writer ownership and recovery-failed state.
  First-send and existing-voyage policies remain distinct.
- `Delivery`: immutable envelopes, saved text/images and recovery phase, independent
  of which component is visible. Keep UI request IDs separate from command IDs.
- Each feature: local selection, search, scroll, loading/error and transient
  request-instance state. No access to peer-private state.
- `RenderCache`: wrapped rows, measured geometry and decoded preview references;
  explicitly separate from persisted state and remote observations.

Retain current on-disk compatibility initially. Internally decode optional fields
and flags into explicit valid workflow phases; preserve exact uncertain envelopes
and original IDs. A new state enum is not permission to rewrite saved commands.

### Event and effect flow

```text
normal terminal input ──> scope router ──> feature message
public observation ────> identity gate ──> read-model/feature update
                                         │
                           state + typed intents/effects
                                         │
                         tracked effect supervisor
                                         │
                         fenced completion message

state ──> prepare/layout ──> paint ──> present ──> presentation facts
```

Feature update receives its own mutable state, narrow read context and an explicit
clock value where needed. It returns messages/effects and invalidation, rather
than calling `tokio::spawn`, filesystem APIs or other features directly.

Effects carry only the identities relevant to their operation: connection and
activation, session/incarnation, optional run/revision, feature/request instance,
and durable command ID where applicable. There must not be a universal rule that
all completions require a current snapshot revision: that would discard valid
durable receipts and some reads. Each effect class defines its acceptance rule.

Effect classes:

1. **Replaceable reads:** cancel/supersede obsolete discovery and panels; apply only
   to the matching live request instance.
2. **Observations:** bounded per-route tasks, subscription cursors and resync;
   coalesce redundant invalidations, not decision outcomes or exact receipts.
3. **Local acquisitions/publications:** clipboard, image preparation, draft save,
   export and launch configuration; explicit ownership and real cleanup. A blocking
   worker cannot be assumed stopped because its async wrapper was aborted. Temporary
   discovery voyages require retained resource identity and explicit cleanup.
4. **Durable mutations/recovery:** persist immutable intent before dispatch;
   uncertain completion triggers observation/resolution of that identity, never
   blind retry or replacement. Closing a view does not erase the obligation.
5. **Private terminal operations:** separate human-only driver and non-recordable
   input queue, not an ordinary serializable effect payload.

Preserve bounded admission and fairness (current reconciliation admits at most four
recovery jobs). Reserve responsiveness for terminal input and command outcomes;
a noisy route must not starve others. Cleanup barriers are per owning scope/route,
not a global wait for unrelated work. Shutdown retains unresolved durable records,
flushes required saves, cancels and observes local work, then restores the terminal;
it does not cancel remote voyages.

### Focus and layout

Normalize key release/repeat behavior once. Route app-level quit according to an
explicit mode policy: Ctrl-C quits normal Helm, but belongs to the program in
private-terminal mode; Ctrl+] returns. Modal text/paste never falls through to the
composer. Completion owns Tab only when active; scoped forms own traversal.

Each prepared frame produces one bounded hit map with frame/geometry generation,
layer, clipped rectangle, stable control identity and action target. Hit maps hold
identifiers, not private text. Resize, target change and modal transitions invalidate
incompatible hits immediately. Pointer drag captures a control until release or
explicit invalidation. Disabled/clipped controls cannot activate.

Click and keyboard activation converge on the same typed intent and validation.
Approval choice is not approval confirmation. Access confirmation preserves the
requirement to see the disclosure. Focus restoration validates the old target
instead of silently redirecting input to another voyage.

## Workflow parity required before deleting the old UI

| Workflow | Required normal and failure behavior |
| --- | --- |
| Startup/new draft | No voyage before first send; retained workspace/config/text; per-draft locking; invalid/contended storage preserves originals |
| First send | Durable start and submit identities; explicit phases for owner-created/turn-not-attempted, refused and uncertain; no replay of creation, submission or uploads |
| Existing compose/steer | Exact target/run; retain pending envelope separately from next editable text; images freeze appropriately; rejection preserves input |
| Navigate/reconnect | New/live/archive views, source draft retention on branch, immutable connection identity, stale generation rejection; disconnect is not cancellation |
| Transcript | Canonical text and tool outcomes, partial/live reconciliation, full-message/earlier-history loading, anchor-preserving resize/search/expansion, time-based result retention |
| Decisions/access | Background target binding, expiry/incarnation/revision checks, pending disablement, separate choice/confirmation, question skip and approval deny/back semantics |
| Lifecycle/overviews/export | Rename, branch, cancel, compact/clear, archive/restore/delete; exact destructive confirmation, cleanup distinctions, stale overview replies and owned export publication |
| Inference/accounts | Model/thinking/service/access controls and slash parity, next-turn versus admitted settings, catalog context/generation; reconcile #213 account/device scopes at the actual cutover baseline |
| Vessels/workspaces | Pair/import/rename/renew/forget/reconnect, private field clearing, retained pending setup identities, allowed remote workspaces and scoped refusals |
| Clipboard/images | Explicit acquisition, left-biased paste anchor, text/image ownership, undo boundaries, bounded decode/upload, cancellation and metadata/no-color fallbacks |
| Terminals | Exact inventory identity, stale/closed terminal refusal, exclusive input, bounded ordered writes without replay, restoration on errors/return/quit |
| Discovery/operator paths | Existing help, slash completion, read-only tools/tasks/agents/policy/workflow/resource overviews and reachable operator-tool commands; no invented editors for known product gaps |

Use [UX readiness](ux-readiness.md) as the broader product inventory, reconciled
with source. Framework parity does not close outstanding UX acceptance, account
implementation or private-terminal fidelity issues.

## Delivery sequence and deletion gates

1. **Baseline and writer coordination.** Recheck dirty state and #213/#217 integration;
   record the exact source revision and existing workflow entrypoints. Agree file
   ownership before parallel implementation. Do not move another writer's files.
2. **Application services and kernel.** Extract durable delivery/persistence and
   observation adapters without changing wire/storage meaning. Implement task,
   scope, navigation, frame and private-terminal ownership contracts. Keep the old
   entrypoint only as a temporary migration boundary.
3. **Complete vertical workflow.** Run startup → draft → first send → conversation
   → next send → reconnect through the new architecture, including refusal and
   uncertainty. This is an intermediate integration milestone, not full delivery.
4. **Migrate every remaining feature.** Decisions/access, lifecycle/overviews,
   settings/accounts, Vessels/workspaces, transcript hydration, clipboard/previews
   and terminals; preserve both keyboard and mouse paths and CLI seams.
5. **Cut over and delete.** Switch all full-screen entrypoints; remove old `App`,
   parallel input routers, render-side application transitions and unowned spawn
   sites. No feature may call through a legacy whole-application adapter.
6. **Verify and publish.** Record exact-build checks and real failure-path evidence,
   update implemented-state docs, integrate/push main, and close only delivered
   scope. Source counts shrinking alone do not satisfy acceptance.

## Verification plan, not claimed results

For implementation, run affected locked Helm checking, a runnable development
build, targeted formatting and Clippy. Check both protocol ends only if contracts
change. Do not run competing builds or recreate the removed broad automated suite.

Use bounded, isolated local/PTy diagnostics with synthetic inputs and no provider
calls for ordinary layout/input paths. Use a synthetic local service/provider only
where full delivery/decision/recovery paths require it. Exercise stale request
completion, old route generations, lost replies, failed recovery without overwrite,
cross-window draft writers, disconnected recovery-slot release, denied/failed
saves, concurrent updates, duplicate activation, shutdown during effects and
terminal-return errors.
Private-field diagnostics must use fake values; do not record real human input.

Cover keyboard/mouse, Unicode/markers, pending states, 40×18/80×24/wide layouts,
resize, NO_COLOR and restored terminal modes. Measure long-transcript redraw,
background-update invalidation, editor/save latency and bounded preview work
before/after; report distributions/environment, not an unsupported speed claim.
Verify actual handles/processes have finished where cleanup is claimed. Test
existing saved drafts and uncertain commands across the cutover without replay.

Documentation-only audit delivery requires path/link/manifest/diff checks, not a
Rust build. The pre-existing deletion of `scripts/release-documents.txt` and the
packaging/quality scripts prevents verifying archive guide inclusion here; do not
restore them incidentally or report packaging as passing. Native macOS/Windows,
real terminal graphics/multiplexers, live account enrollment and named product-owner
visual acceptance require their own evidence.
