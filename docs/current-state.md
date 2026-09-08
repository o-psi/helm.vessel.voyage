# Current implementation

This inventory describes implemented source, not live-provider certification or
native-platform validation. Focused regression and explicitly enabled live checks
are described in [quality](quality.md). The [implementation ledger](implementation.md)
records delivery evidence and its limits.

## Process ownership

Provider account exhaustion (`usage_limit_reached` or `insufficient_quota` in an
HTTP 429 response) stops the turn without transient-rate-limit retries. Helm shows
newly failed runs as Failed and retains an authored usage-limit explanation across
turns and runtime suspension. Raw provider error bodies are not exposed in these
public failure summaries. Ordinary transient throttling retains bounded retries.
This handling cannot restore an exhausted provider account's allowance.

Ordinary Helm chat opens a private local draft. Opening and closing the interface,
editing its first message, or pressing Ctrl+N does not create a session or launch
a voyage process. Blank Ctrl+N requests reuse the window's blank draft for that
route/workspace. Drafts appear separately above the voyage list; Tab switches
out of a draft, and clicking its row returns to it. Nonempty drafts and changed
settings survive interface exit. Concurrent Helm windows lock individual drafts
and do not take over each other's first sends. Plain chat also waits for a first
nonempty message; EOF and `/quit` before that create no voyage.

In a draft, `/model NAME`, `/access read-only|approval|unrestricted`, and
`/workspace /absolute/path` edit local launch settings; `/discard` discards a draft
only before first send. Remote drafts use executing-host configuration and need
an explicit absolute workspace with `/new /absolute/path`. Session-scoped grant
connections cannot create another voyage. Explicit `connect new`, resume and
branch retain their existing intentional session semantics.

First send saves stable session, start and turn command identities locally before
requesting Vessel creation, then saves the exact revision-bound submission before
sending it. This uses the existing deduplicated start and submit protocol; the two
steps are not a cross-process atomic transaction. An interruption between them can
leave an owner with no admitted turn, attached to the recoverable first-send draft.
Helm automatically continues the saved, user-authorized creation and any first
submission that has not been attempted, including after reopening Helm. Once
submission was attempted, recovery resolves its original identity without replaying it. Helm never automatically
sends an uncertain turn again or creates a replacement session. Pending text/settings are frozen until the outcome resolves.
Definite initial creation refusal preserves an editable draft; definite first-turn
refusal preserves its text on the created voyage. Recovery retains pinned policy
selection and explicit overrides, which are revalidated before local launch.

The on-disk registration's `starting` field is initial metadata, not a current
health reading: Vessel derives live state using authenticated runtime inspection.
Existing empty or unavailable voyages are not removed by draft handling.

When a runtime disappears without clean suspension evidence, an ordinary history
or turn request asks Vessel to run the existing exclusively fenced recovery with
no operator attestations. Recovery records interrupted work without replaying it,
then respawns the same session in a fresh voyage process. Unknown cleanup,
unresolved tools or retained resources remain fenced for explicit recovery rather
than being inferred safe. Vessel retains the last bounded canonical voyage name as
catalogue metadata, so Helm can identify saved conversations while recovery runs.

After a constructed run stops, its voyage retains cleanup tasks, resource managers,
checkpoint ownership and host reservations until cleanup is observed. A failed
cancellation monitor explains an interruption but does not permanently poison
cleanup: its original in-flight database read is retained until observed, and
outstanding checkpoint callbacks continue to hold the run fence. Cleanup observes
subordinate tasks, tools, root terminals, runtime controls and compatibility
processes. Finished terminals are explicitly closed before the run blocker clears.

Cleanup makes at most three attempts per failed component, with paced retries.
An overdue task keeps its original handle; observation never launches a duplicate
while that task is still running. Helm shows cleanup progress and the unresolved
component groups. A fresh, valid explicit send can request another cleanup batch;
if cleanup still blocks admission, the message remains an unsent draft. Retrying a
previously accepted or definitively rejected command does not reset cleanup.
Only an explicit later turn continues the same saved voyage; cleanup replays no
provider requests or tools and does not turn interrupted work into success.

This repairs cleanup after interruption; cancellation monitoring still stops a run
when its five-second observation budget fails. It does not make every computer
suspend transparent. A dead owner or an older live runtime without retained cleanup
handles still requires evidence-based explicit recovery. Restart never converts
legacy uncertainty into observed cleanup. Construction failures before the cleanup
coordinator is installed retain their existing conservative recovery behavior.

Helm chat, one-shot runs, connected clients, workflows, managed sessions and the
outbound worker adapter use **Helm → Vessel → voyage**. Vessel launches a separate
long-lived process for each session. Only that voyage constructs the executor,
loads execution-host credentials, admits turns and writes canonical checkpoints.
Helm links shared configuration and presentation types but has no embedded agent
loop. New local voyages created inside ordinary chat preserve its startup
configuration through the private host handoff; explicit connect and remote
creation use executing-host configuration. Closing Helm detaches; it does not cancel accepted work.

Linux local discovery starts an absent supervisor from companion binaries. The
supervisor binds an ephemeral literal-loopback HTTP endpoint and atomically publishes
its bearer credential in the owned private Vessel directory. Helm validates that
directory, credential file and endpoint before every request. Commands use bounded
POST requests. The connected TUI and plain/run followers receive durable
invalidations over authenticated SSE and fetch canonical snapshots/output only when
notified; stream reconnect uses the last snapshot cursor and never resubmits work.
Scoped remote routes use HTTPS and private credential files bound to one session,
principal, workspace, rights, revision and expiry. The SSH account adapter remains a
compatibility path and relays to the same local HTTP endpoint. Enrollment-bound
grants also verify the current machine epoch.
Provider credentials are never copied between Vessels by these transports.

Vessel retains private supervision metadata, serializes starts,
checks exact session/incarnation health and refuses stale endpoints. Stored PIDs
are not evidence of survival. Restart requires positive stopped or explicit recovery
evidence. Unavailable owners are fenced; uncertain tools are never automatically
replayed. Stopping a supervisor leaves independent voyage processes running.
Vessel has no concurrent voyage-count cap. The legacy `local-serve --capacity`
option is accepted but ignored; capabilities report `capacity: null`. The 4,096
registration retention bound, transfer receipt safeguards, connection bounds and
runtime host resource limits still apply.
Explicit Linux user-service installation is available. Native macOS/Windows process
supervision is unsupported. The Linux installer wizard installs versioned releases
and manages upgrades, rollback and the local user service. Upgrade resolves the
latest published GitHub release by default; explicit `--dev` fetches and builds a
pinned main commit, while `--bin-dir` selects local binaries. Preparation retains
one verified artifact through review/apply and bounds network/build work and
cancellation cleanup. Missing published releases never fall back to main. See
[installation](../installer/README.md) for prerequisites, trust and restart limits.

Helm keeps the interface open when initial voyage creation is definitely refused,
including admission refused by older Vessels, so existing voyages remain reachable.
An uncertain start still reports its original identity rather than automatically retrying.

The left-hand voyage list is newest-first by the latest timestamped conversation
message, falling back to session creation time. Tab and Shift+Tab follow that
same order; activity changes do not change the selected voyage. Legacy messages
without timestamps use the creation fallback until new messages arrive. Owners
that do not expose timestamps, and voyages without snapshots, sort last with a
stable identity tie-break. Renaming, polling, or system compaction markers do not advance activity.

The conversation log keeps one blank row before saved message headings, pending
delivery, live response blocks, tool activity groups and individual visible tool
calls when preceding content is not already blank. Expanded groups use the same
spacing; wrapped lines within a call stay together. This separates chats and
activity without extra leading space or changes to saved message text.

Muted horizontal rules separate sidebar voyages in both current and archived lists,
using the existing padding row without increasing entry height.
Each sidebar voyage has a clickable **⋮** Actions button. Mouse hover highlights
voyage rows, their separate Actions buttons, enabled action-menu entries and
access-mode choices with a contrasting background, without changing keyboard
focus or selection. Voyage rows use no underline across their wrapped text or padding. Hover uses the current rendered rectangles and is
suppressed behind modal input; disabled actions retain their explanatory styling.
Mouse-motion reporting requires a supporting terminal. Pointer state clears on
reported terminal focus loss and private-terminal handoff. With an empty composer
or sidebar focus, Up/Down navigate voyages; Right focuses the button and Enter
opens its submenu. Left/Escape back out, typing focuses the composer, and F9 opens
Actions even when the narrow layout hides the sidebar. The menu offers Rename,
Archive/Restore, Branch, Cancel for an observed active run, Details, and separated
Delete; Export is intentionally absent. Disabled actions explain their state
requirements. Rename/branch input and typed DELETE confirmation are separate from
the saved composer draft. Menus retain their target/incarnation and deletion
confirmation checks that the observed history revision has not changed. Archived
voyages must be restored before rename, branch or delete. Successful deletion
shuts down the runtime, retains its receipt with cleanup evidence, and removes
the tombstone from Helm's voyage lists without reusing its identity.

Typing `/` in the Helm composer opens a filtered command menu above the input.
Up/Down select and Tab completes a command, then shows contextual arguments.
Enter dispatches the typed command; Escape dismisses completion, restoring normal
Tab voyage navigation. Model, tool and terminal options come from the selected
owner; pending decisions and voyage choices come from observed state. Local paths
have bounded filesystem suggestions; remote paths and free-form names, answers
and JSON arguments show input guidance. Clear/delete confirmations must still be
typed explicitly. Unavailable metadata shows a retry hint rather than invented
options. The menu lists the current process-client controls, not retired commands.

## Persistence, migration and lifecycle

Each active voyage holds its exclusive session fence through turn execution and
cleanup. After a terminal turn and positively observed cleanup, the process exits
with an internal suspended disposition. The next submission automatically starts
a new incarnation with the same session UUID, saved configuration and canonical
history, using the current Vessel runtime binary. Helm labels successful completion
Finished for 24 hours after its durable completion timestamp, then Settled. Failed,
cancelled and cleanup-pending outcomes remain distinct. Bounded one-shot helpers
serve suspended observations without waking an executor. Initialization and
management-only processes retire after a short idle grace; volatile private workflow
preparation retains its existing bounded lifetime. Root terminals close before
suspension; terminal metadata does not imply a live process across turns.
Canonical SQLite history, command bindings, streamed checkpoints, decisions,
steering and cleanup obligations survive interface disconnect. Exact command
retries retain their original outcome, including definite rejections. A changed
principal or payload conflicts. An unknown outcome requires receipt inspection.

Ordinary JSON sessions import on explicit resume with the original UUID, source
revision and fingerprint. The source is fenced and retired before destination
execution. Legacy shared managed journals migrate the selected session, receipts
and actor identity into a dedicated owner; unrelated sessions remain untouched.
Abandoned legacy work has an explicit voyage maintenance recovery path. Import
publication failures retain provenance and never fall back to a writable old copy.

Agent-construction failures retain resource observers and attempt bounded cleanup.
Only positively observed cleanup releases run admission and host executor charges.
Snapshots and Helm show an authored startup-stage summary and distinguish a
retryable failed run from unconfirmed cleanup; underlying diagnostics are excluded.
Previously stranded runs still require explicit recovery.

Archiving an idle voyage preserves its conversation and causes its runtime to
shut down under the admission lock. Only positively observed cleanup releases
Vessel capacity. A bounded archive summary and the archive receipt accompany the
private stop evidence, so a fresh Helm can list the archived voyage without
starting it. F5 or `/archived` opens Archives; `/restore` explicitly restarts its
same session identity and clears the archive disposition. Restore can be refused
when capacity is full; archived history remains intact. Unavailable or unclean
owners remain fenced and consume capacity. Existing pre-upgrade runtime processes
need a clean stop/restart to acquire the new archive shutdown behavior.

Idle lifecycle commands support rename, next-turn model/configuration, branch,
archive/restore, clear, compaction and confirmed deletion. Branches have new UUIDs
and omit provider continuation. Clear resets the current conversation; prior run
and receipt evidence remains. Compaction removes earlier whole conversation turns
with an explicit omission marker. Delete purges retained canonical text and session
resources after cleanup, preserving identity and deduplication tombstones. This is
application deletion on the executing host, not forensic erasure of storage media
or removal of independently retained source/transfer archives on other hosts. `--no-save` uses a
temporary durable voyage and then deletion; unknown cleanup prevents deletion.

Metadata events use a durable `public-v1` cursor and bounded retention (2,048 events).
Replay gaps lead to an authorized snapshot. History and output use revision-bound
pages and UTF-8 byte chunks within 4 MiB frames. Exports retrieve a complete public
history revision before atomically publishing a new local Markdown file. Provider
continuation, unsent drafts and private terminal input are excluded.

## Interfaces and controls

Helm uses the independently versioned public Vessel API at `/v1/vessel/command`
and `/v1/vessel/events`. Its explicit session operations are distinct from private
runtime commands. Vessel selects owners under lifecycle arbitration, translates
requests and normalizes responses; live-resource actions retain exact incarnation
fences. Public SSE follows session owners and reports observed incarnation changes.
The private runtime IPC remains protocol v1. See [process access](process-access.md#wire-and-retained-state)
for wire examples and the coordinated Helm/Vessel gateway upgrade requirement.

The connected TUI combines local HTTP, scoped HTTPS and SSH compatibility routes. It retains separate
drafts, prompt navigation, scroll, pending command identities and observation
cursors per voyage. Switching views does not redirect in-flight actions. Background
voyages show unread state and pending decisions; slow remote observation runs
outside the input/render loop. Plain chat, one-shot streaming and JSON managed
output observe the same owner. Reconnect never resubmits a turn or terminal input.

Approvals and questions are durable requests bound to session, incarnation, run,
principal, revision and deadline. Responses require current authority. Waiting is
bounded by policy and at most 120 seconds; silence never authorizes an effect.
The connected TUI opens selected-voyage approvals and questions automatically,
with highlighted choices, Up/Down selection, Enter confirmation and Esc to deny
or skip. Custom answers use an explicit editor; conversation drafts are preserved.
Left/Right navigates simultaneous requests and PageUp/PageDown reads long details.
Steering is durable and delivered at a supported safe model boundary. Operator
runs do not accept steering. Native providers remain distinct from the optional
compatibility provider and its narrower capabilities.

Helm exposes named interactive terminals through the persistent F3 action. The
browser lists observed state and host, selects with Up/Down, and attaches with
Enter using the inventory's exact owning run and observed incarnation. Ctrl+]
returns from private input to the saved conversation draft. Stale inventories
cannot attach. F1 opens a scrollable guide; operator panels and command receipts
render readable metadata. Chat shows compact public tool targets and outcomes directly for up to three
consecutive calls. Longer sequences use independently clickable accordions, with
Ctrl+T controlling all groups; raw result payloads remain excluded. Turn separators
show elapsed time when the runtime recorded start and finish timestamps. Authored JSON is preserved,
and operator summarization requires explicit message metadata. Runtime projections
include message times, bounded turn outcomes and a reconciled live suffix, so
committed text is not repeated by the streaming preview. Managed execution reuses
the admitted user message exactly, including its timestamp, at the first checkpoint.
Ctrl+F searches loaded rows; PageUp/PageDown preserve a message-relative reading
anchor across appends and resize, and Ctrl+End returns to latest. Ctrl+Home loads
128 earlier messages. Oversized messages load automatically through revision-bound
history and message chunks, bounded to 16,384 messages / 64 MiB per reading window.
Archived views preserve cached history but cannot fetch from a stopped owner;
restoration is required after reopening an uncached archive. Older owners without
stream-boundary metadata show canonical text without a speculative live preview.
F8 opens a keyboard chooser for existing read-only overviews without editing the
draft. A quiet voyage rail, borderless conversation and compact composer adapt
the supplied conversation-interface reference to terminal cells.
Helm automatically resolves uncertain ordinary runtime commands, including interaction
responses, without resending work. `/receipt` remains an optional diagnostic command. Helm saves its full public request, ID, revision and expiry before dispatch.
Each ordinary status check makes at most three bounded attempts. Unresolved outcomes
are checked again after five seconds, without overlapping checks or racing the initial
send. At most four background recovery jobs run at once across all views and first-send
drafts, scheduled oldest-due first so unavailable routes cannot monopolize recovery.
Recovery does not require selecting a view. No F4 check action is needed. Under the exclusive runtime
dispatch gate (or the cleanly suspended execution fence), resolution returns an
existing receipt or durably closes an unadmitted command ID. A delayed copy of that
ID cannot subsequently execute. `not_admitted` restores an editable draft; confirmed
admission clears pending delivery. Transport failures and older owners without
`resolve` leave the original pending, rather than treating a failed status check
as a refused submission. Legacy payload-free pending commands can be resolved with
local host authority; scoped clients need the original public request. After a
supervisor upgrade, suspended or cleanly stopped resolution uses the updated
one-shot runtime under the saved session fence, even if the retired executable
predates `resolve`; it does not restart an agent or change the incarnation. Vessel
branch and stopped-archive restart workflows keep their separate receipt-only recovery.
Private terminal input and workflow secret inputs never enter these envelopes.

The runtime's event long polls do not hold the dispatch gate. Idle suspension runs
outside the listener accept loop, excludes mutation dispatch, and drains queued
connections into explicit non-dispatch responses where transport permits. Lost
connections still require command resolution, not inferred success or replay.

Approval tools distinguish explicit refusal, expiry without an answer, cancellation,
authority invalidation and an unavailable approval interface. An answer durably
recorded before expiry remains the answer even when read after the deadline.
Ordinary screens use names and plain-language summaries rather than runtime
identifiers and schemas. Console rendering preserves blank/wide cells and uses
the runtime's cursor; a supplied program title appears in its private header.

Tools, policy, todos, subagents, terminals, workflows and model metadata have runtime
controls. Operator tool calls use the real authorized registry and admitted run
resources, including approval and completion accounting, without inventing a model
request. Saved workflows retain digest-bound trust and typed public inputs; secret
shell bindings travel through an expiring private input channel and are excluded
from durable command/history payloads. GitHub operator commands run in voyage with
exact attended publication decisions and canonical session references.

## Resources and execution policy

Native OpenAI Chat, OpenAI Responses, Anthropic and ChatGPT OAuth transports retain
streaming/tool behavior. Native API providers do not require Codex. OAuth and API
billing remain distinct; the optional compatibility bridge executes its own
app-server. Linux cleanup observes its original process session. MCP stdio tools
retain their bounded transports and observed child-session cleanup.

The live registry supplies tools and runtime instructions. Local roots, command
denials, approvals, cancellation, administrator ceilings and resource limits apply
to root and subordinate work. Application policy is not an OS sandbox. Execution
configuration is loaded and revalidated on the executing host; routing cannot
broaden it. External content remains untrusted.

At the model's final response, Voyage derives completion automatically from the
run's recorded task and agent outcomes. Completed items need no separate review,
fingerprint exchange or disposition call. Unfinished todos and unsuccessful agents
remain incomplete; missing records and active agents remain unresolved. Remaining
owned agents receive bounded shutdown and a fresh observation before the final
decision is saved under the existing writer lock. Resource cleanup still requires
actual observation before another turn can start. Completion does not ask the model
for an extra reconciliation turn or impose a bookkeeping deadline.

The standard tool registry no longer advertises the internal `completion` tool.
Default instructions make todos and delegation discretionary when useful. Existing
review records and sealed decisions remain readable without rewriting their history;
Helm describes historical bookkeeping calls by operation rather than “Completion”.
Recorded completion is not proof that an answer is semantically correct.

Todos, subagents, terminal inventories and completion accounting are session-scoped.
The subagent writer lease protects that session's persistent agent tree, so distinct
voyages can execute and delegate concurrently in the same workspace. Writers of the
same session state still coordinate through its completion lock and exclusive owner.
Sharing a workspace does not isolate file edits; tools retain their existing policy,
stale-content and Git conflict checks. Host accounting bounds
executor slots to 64 and PTYs to 256 across voyage processes. Unknown process death
retains charges until explicit reconciliation/attestation. Completion evidence is
accounting, not proof that a claim is semantically true.

The shared inference database waits up to two seconds for a competing SQLite
writer, within the accounting worker's ten-second deadline. Persistent contention
fails the operation; it never causes a provider request to be replayed. Dispatched
attempts with unconfirmed accounting retain their unknown outcome.

Root PTYs remain available during their turn and close before the voyage suspends.
Subagent and transient resources are cleaned at their run boundary. Terminals keep
their original policy checks; revocation closes the manager. Model/configuration
transitions and lifecycle operations observe required cleanup. A separate private human terminal
channel binds exact run and terminal identity; its writes are never journalled or
replayed. Saved terminal records do not resurrect processes after restart.

## Participants, transfer and outbound compatibility

Participant execution requires an explicit locally accepted binding, bounded
context disclosure and a current parent grant. The receiver creates a distinct
subordinate voyage session under its own local policy. The parent remains canonical
owner and records immutable assignment admission, attributed result and cleanup.
Unknown admission blocks reassignment. Revocation or membership removal drains or
cancels explicitly; late cleanup can be reconciled from an idle parent without
launching another run.

Owner movement is separate. Locally pinned Vessel signing identities authenticate
destination preparation and source manifests. The source durably relinquishes and
stops before the destination verifies the checkpoint and activates a new generation.
A private courier journal makes exact retries reviewable. Reservations do not
permit timeout takeover. Providers, credentials and live process state do not move.

`helm remote-worker` activates a supervised outbound voyage and exits. A separate
transport-only relay retains the enrollment connection without owning the canonical
session or an agent loop. Turns wake independent voyage workers; fenced one-shot
helpers serve suspended remote observations. Private IPC carries the current,
bounded transport lease into each worker: authority loss cancels work and cleanup
must be observed before suspension. This differs from an ordinary
Helm interface disconnect, which does not cancel. Local consent withdrawal and
legacy recovery run through the voyage executable. Enrollment alone does not share
an existing session. The Vessel web page remains a status page; a browser execution
console is deferred.

Declarative extensions, repository onboarding, configuration drafts and local
credential enrollment retain their explicit operator workflows. See
[operations](operations.md), [configuration](configuration.md) and
[security](security.md) for usage and authority boundaries.

Voyage Actions also includes **Access**: Read only, Ask first, or Unrestricted.
`/access` opens the same chooser; `/access read-only`, `/access approval` and
`/access unrestricted` open a confirmation for that mode. Slash completion offers
all three. The chooser shows effective access and preserves the composer draft.
Changes require executing-account owner authority and are allowed during an active
run. New tool admissions use the updated mode; pending tool approvals are invalidated
and stale admissions must retry. Already-started work and terminals are not stopped,
but read-only mode refuses new terminal input. Idle changes still require observed
cleanup and close retained terminals first. Ordinary configuration remains idle-only.
The runtime changes only access in its
private durable configuration, preserving the current model, credentials and other
policy settings across restart. System and participant limits still apply; requests
above those limits are refused. Named profiles retain their bound selection and
receive an explicit access override. Defaults that require a fresh workspace
activation must still be previewed and activated through the policy defaults
workflow; the chooser does not silently activate them. Unrestricted does not remove
folder limits, blocked commands or administrator policy. Built-in tools remain
available for later access changes; MCP servers omitted when a run starts in
read-only mode are not started by a mid-run access change.

Outbound enrollment-relay commands retain their separate remote-owner receipt
and transport-lease authority; local `Resolve` refuses remote-bound sessions.
