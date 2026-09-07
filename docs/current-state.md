# Current implementation

This inventory describes implemented source, not live-provider certification or
native-platform validation. Automated tests and evaluations remain absent by
request; see [quality](quality.md). The [implementation ledger](implementation.md)
records delivery evidence and its limits.

## Process ownership

Helm chat, one-shot runs, connected clients, workflows, managed sessions and the
outbound worker adapter use **Helm → Vessel → voyage**. Vessel launches a separate
long-lived process for each session. Only that voyage constructs the executor,
loads execution-host credentials, admits turns and writes canonical checkpoints.
Helm links shared configuration and presentation types but has no embedded agent
loop. New local voyages created inside ordinary chat preserve its startup
configuration through the private host handoff; explicit connect and remote
creation use executing-host configuration. Closing Helm detaches; it does not cancel accepted work.

Linux local discovery starts an absent supervisor from companion binaries. Local
Unix connections verify account ownership and peer credentials. SSH routes use an
explicit remote account and its private Vessel directory. HTTPS grant routes use
private credential files scoped to one session, principal, workspace, rights,
revision and expiry. Enrollment-bound grants also verify the current machine epoch.
Provider credentials are never copied between Vessels by these transports.

Vessel retains private supervision metadata, serializes starts, enforces capacity,
checks exact session/incarnation health and refuses stale endpoints. Stored PIDs
are not evidence of survival. Restart requires positive stopped or explicit recovery
evidence. Unavailable owners are fenced; uncertain tools are never automatically
replayed. Stopping a supervisor leaves independent voyage processes running.
Explicit Linux user-service installation is available. Native macOS/Windows process
supervision is unsupported. The Linux installer wizard installs versioned releases
and manages upgrades, rollback and the local user service.

Helm keeps the interface open when initial voyage creation is definitely refused,
including a full Vessel, so existing voyages remain reachable. An uncertain start
still reports its original identity rather than automatically retrying.

The left-hand voyage list is newest-first by the latest timestamped conversation
message, falling back to session creation time. Tab and Shift+Tab follow that
same order; activity changes do not change the selected voyage. Legacy messages
without timestamps use the creation fallback until new messages arrive. Owners
that do not expose timestamps, and voyages without snapshots, sort last with a
stable identity tie-break. Renaming, polling, or system compaction markers do not advance activity.

Each sidebar voyage has a clickable **⋮** Actions button. Mouse hover highlights
voyage rows, their separate Actions buttons, enabled action-menu entries and
access-mode choices with a contrasting background and underline, without changing
keyboard focus or selection. Hover uses the current rendered rectangles and is
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

Each voyage holds its exclusive session fence through idle time and cleanup.
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

The connected TUI combines local, SSH and scoped-grant routes. It retains separate
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
render readable metadata. Chat activity expands with Ctrl+T to show action names
and outcomes; raw tool result payloads remain excluded. Authored JSON is preserved,
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
F4 checks an unresolved receipt without resending work or editing the draft.
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

Todos, subagents, terminal inventories and completion accounting are session-scoped;
cooperating workspace writers retain shared arbitration. Host accounting bounds
executor slots to 64 and PTYs to 256 across voyage processes. Unknown process death
retains charges until explicit reconciliation/attestation. Completion evidence is
accounting, not proof that a claim is semantically true.

Root PTYs can persist between turns in one live voyage. Subagent and transient
resources are cleaned at their run boundary. Retained terminals keep their original
policy checks; revocation closes the manager. Model/configuration transitions and
lifecycle operations observe required cleanup. A separate private human terminal
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

`helm remote-worker` now activates a supervised outbound voyage and exits. Its
legacy enrollment relay executes inside that voyage with the original lease/grant
contract: transport authority loss cancels work. This differs from an ordinary
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
Changes require an idle live voyage, executing-account owner authority, and observed
cleanup; retained terminals close first. The runtime changes only access in its
private durable configuration, preserving the current model, credentials and other
policy settings across restart. System and participant limits still apply; requests
above those limits are refused. Named profiles retain their bound selection and
receive an explicit access override. Defaults that require a fresh workspace
activation must still be previewed and activated through the policy defaults
workflow; the chooser does not silently activate them. Unrestricted does not remove
folder limits, blocked commands or administrator policy.
