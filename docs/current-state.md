# Current implementation

This inventory describes implemented source, not live-provider certification or
native-platform validation. Focused regression and explicitly enabled live checks
are described in [quality](quality.md). The [implementation ledger](implementation.md)
records delivery evidence and its limits.

## Shared local browser

Helm's F6 and `/browser` open a dedicated local Chromium companion. Browser actions
and results use the same authenticated full-duplex Helm–Vessel socket as ordinary
commands; the helper has private local stdio only. Human/agent control shares one
browser context, with local origin grants, per-effect confirmation, private capture
suspension and explicit return-to-agent epochs. Remote policy remains independent.
A socket identity/loss change fences sharing, not the remote Voyage. Typed visual
tool results retain their original call provenance through authorized artifacts.

This is Linux-qualified source functionality, not certification of arbitrary
websites, a production TLS proxy or native macOS/Windows behavior. Setup is an
explicit `helm browser setup` operation; no browser or model credentials are
installed automatically. See [local browser](local-browser.md),
[runtime browser binding](shared-local-browser-runtime.md) and
[visual tool results](visual-tool-results.md) for exact scope and limits.

On Linux, an existing voyage can recreate its deleted working directory on the
next message while retaining conversation history and access settings. Recovery
creates an empty private directory at the same path, blocks accidental parent Git
discovery, and explains that previous files were not restored. Saved reads and
delivery resolution do not require directory recreation. See
[deleted working directories](runtime-contract.md#deleted-working-directories) for bounds and verification.

## Tool reliability

Native action schemas reject irrelevant per-action keys before execution. Shell
policy uses bounded syntax-aware inspection, including literal heredoc handling,
without bypassing roots or approvals. New persisted tool outcomes
distinguish command exit, policy refusal, execution error and incomplete output;
Helm displays these distinctions with legacy-history fallbacks. Vessel inspection
provides a compact observed overview with progress excerpts and coordination/cleanup
observations. Explicit detail reads page public snapshot fields;
full-message and run-output reads stream bounded chunks with private expiring
cursors, preserving redaction and exact continuations without a whole-record cap.
Regex expansion retains a 4 MiB per-message source bound. An overview that cannot fit is withheld with a
smaller detail-read request. History pages fit the output budget automatically;
regex conversation search returns bounded matching-message pages with full-message
and context reads, revision-bound continuations and explicit unsearched gaps.
See [Vessel coordination](vessel-coordination.md) and
[tool validation and outcomes](runtime-contract.md#tool-validation-and-outcomes)
for supported syntax, compatibility and platform limits.

Completed tool action rows show the executing Voyage's monotonic invocation duration
(milliseconds, seconds, or minutes), retained in `tool_outcome.elapsed_ms` across
history reloads and bounded projections. Timing includes policy/approval waits but
excludes result checkpointing and the lifetime of programs or independent voyages
started by a call. Returned errors and refusals are timed too. Legacy results show
“timing unavailable”; interrupted calls without a recorded result have no fabricated
duration. This is completed-call timing, not a live running stopwatch.

## Terminal color adaptation

Full-screen Helm and private-terminal chrome use passive terminal color detection
and `HELM_COLOR=auto|never|16|256|truecolor` overrides. Automatic mode honors any
nonempty `NO_COLOR`; explicit modes take precedence. Final visible-frame adaptation
covers legacy widgets and syntax colors without changing plain output or input
routing. Shared semantic roles cover Markdown defaults, transcript, sidebar,
forms, actions and terminal chrome, with non-color focus/selection cues. See
[UI styles and fallbacks](ui-styles.md) for dependencies and bounds, and
[component decisions](ui-components.md) for selected libraries and follow-up scope.

The private Vessel panel uses scoped focus for form fields, reconnect toggles,
submission and confirmation controls. Tab/Shift+Tab traverse that scope; ordinary
composer completion and voyage switching retain their own routing. See
[scoped interaction](ui-interaction.md).

The voyage sidebar gives every voyage one title row plus a divider, without
repeated status prefixes or a separate status/host row. Existing semantic status
styles remain; when the visible list spans multiple Vessels, a muted Vessel label
follows the title and may be clipped before it. The selected voyage header retains
explicit status text, including in no-color mode. Current source does not track
unread voyages or display an unread badge.

The conversation footer combines send hints and navigation shortcuts on one row,
shortening shortcuts as space decreases. Routine connection-refresh notices are
hidden. Pending and next-turn hints remain visible; other notices share that row
when they fit, otherwise using up to three footer rows in total.

Normal new/existing composers support grapheme-aware editing and selection with
bounded text undo/redo. Attachment ownership changes establish an undo boundary;
deleted image labels cannot return without their owned attachment. See
[composer editing](ui-editor.md) for keys, retention and paste behavior.

## Direct image paste

The normal new/existing Helm composers accept **Ctrl+V / Alt+V** clipboard input
and pasted local image paths. Native clipboard acquisition is asynchronous and
cancellable, with a left-biased caret anchor so continued typing retains the
requested insertion location. Owned `[Image N]` elements support atomic cursor
movement/removal and exact text/image interleaving. Normal metadata rows display
name/type/size/dimensions. The image modal and in-app screenshot-capture flow have
been removed; use OS screenshot tools and paste their results.

Alt+P toggles [inline composer previews](ui-previews.md), initially off. One bounded
background worker uses retained image bytes, with Kitty graphics when explicitly
supported by passive geometry/color selection, halfblocks otherwise, and metadata
only for no-color or oversized inputs. Previews do not alter pending submissions.

Private drafts retain verified bytes and marker ownership, including legacy image
draft migration without changing immutable pending commands. The existing bounded,
authorized Vessel upload/admission path, session-scoped SHA-verified raster storage,
resume/branch retention and native provider encoders remain in place. Unsupported
models, active image steering and unsupported transports fail with the draft intact.
Native clipboard reads are explicit local input, never background model tools.
See [Images and pasted screenshots](multimodal-implementation.md) for platform,
resource, terminal-key and verification details.

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
and do not take over each other's first sends. Completed or discarded draft JSON
is removed after completion is durably saved; the lock file remains for concurrent
window safety. Legacy finished records are preserved as `.finished` files and
excluded from recovery without decoding obsolete launch settings. Unfinished
records retain full validation and exact pending command identities. Plain chat
also waits for a first nonempty message; EOF and `/quit` before that create no voyage.

Model discovery for a local draft (including automatic capability refresh and the
picker) and `helm models` uses the owner-local `DiscoverModels` Vessel operation.
Vessel supervises a bounded `voyage discover-models` helper, not a registered
voyage or agent loop. Private launch configuration travels over authenticated local
HTTP and framed child stdin; it is not saved as a launch file, session, command
receipt or conversation. Runtime policy, provider display validation and cleanup tracking
still apply. Discovery is limited to four concurrent children per Vessel, a
12-second helper deadline, and bounded supervisor termination/exit observation.
The supervising task retains ownership after a requesting Helm disconnects.
Success, failure and cancellation therefore need no temporary-voyage deletion.
Older Vessels without `sessionless_models` return an update-required result; Helm
never falls back to creating a temporary voyage. Scoped grants cannot submit this
owner-local configuration operation; existing-session model controls retain their
own authority checks. Provider credentials stay on the executing host.

An untouched local draft has no saved draft JSON; edited text/images or settings
are still retained, as are explicit remote workspace choices. Empty, whitespace-only
and over-limit one-shot prompts are rejected before creating a voyage. Workflow
input already cancelled before creation is rejected there as well; cancellation
and admission are checked again during execution handoff. Intentional creation APIs
and interrupted first sends keep their exact durable identities. Previously saved
records are not blanket-deleted or hidden merely because their conversation appears
empty. Untitled entries whose observation is pending or failed say “Loading voyage”
or “Unavailable voyage”; missing observations never prove emptiness.

In a draft, `/model NAME`, `/access read-only|approval|unrestricted`, and
`/workspace /absolute/path` edit local launch settings; `/discard` discards a draft
only before first send. Remote drafts use executing-host configuration and an explicitly selected
server-authorized workspace. The Vessels panel provides workspace pairing and a
workspace picker; `/new` can also select an approved path or workspace UUID.
Session-scoped sharing connections cannot create another voyage. Explicit `connect new`, resume and
branch retain their existing intentional session semantics.

The composer has four clickable controls: Model, Thinking, Service and Access.
Access shows the current mode and opens the existing voyage access confirmation
flow, or a keyboard picker for local drafts, without replacing message text.
Remote drafts retain executing-host policy and cannot edit access locally;
first-send-pending drafts remain frozen. `/access` remains available.
The inference selectors have matching
`/model`, `/thinking` and `/service` commands. Selectors support search, keyboard
selection and cancellation while preserving composer text. Local drafts carry
explicit selections into creation; remote drafts retain executing-host defaults.
Model changes with existing overrides require confirmation to clear or keep them.
`inherit` clears an explicit Thinking or Service override. `/service default` is
an explicit adapter-specific choice, distinct from inheritance.

Inference changes travel through the public Vessel API to the owning Voyage as
revision-bound, deduplicated commands. Vessel retains the immutable intent before
forwarding; Voyage validates and persists the outcome and next-turn configuration.
An active turn, including its tool continuations, retains its admitted settings.
Confirmed changes survive runtime suspension and supervisor restart. Helm retains
pending commands and reconciles their original identities after reconnect without
replaying uncertain commands. Stale or unsupported changes preserve prior values.

Named provider connections/accounts keep credentials on the execution host. Helm's
Account control stages an atomic account/model/override choice independently of the
active run; private device sign-in exposes only temporary verification material to
its human view. Resume and branches retain trusted account identity. API credentials
are entered in an execution-host private terminal or bound to an explicit environment
name. Account-use and enrollment grants are distinct and default-denied. See
[named provider accounts](provider-accounts.md) for lifecycle, migration, uncertainty,
and evidence limits; no automatic quota rotation or credential forwarding is provided.

OpenAI Responses, Chat Completions and ChatGPT OAuth adapters encode explicit
reasoning-effort and service-tier fields. Native ChatGPT catalog discovery retains
advertised reasoning and service choices and their optional defaults. A shared
resolver distinguishes transport-only suggestions, advertised support, unsupported
settings and unknown account entitlement. Inherited catalog defaults are sent on the
wire when available; missing defaults remain provider-managed, not guessed. Explicit
ChatGPT `default` suppresses catalog tier selection and is omitted on the wire;
OpenAI API `default` is sent explicitly. Anthropic rejects these generic overrides
rather than silently discarding them.

Discovery is bounded and scoped to provider, endpoint and credential/account,
with five-minute freshness and 30-second automatic failure backoff. Snapshots schedule
nonblocking discovery; local draft discovery stays in a supervised Voyage. Pickers
refresh metadata for all three controls, not just Model. Runtime turn dispatch
validates and freezes resolved defaults across tool continuations/retries. The
active-turn snapshot separately reports the last completed response's service tier
when supplied; it is not a billing ledger. Unknown metadata after suspension/restart
remains unknown until rediscovery. Remote drafts retain executing-host settings.
See [configuration](configuration.md#next-turn-inference-controls) for inheritance,
provider differences, freshness and cost semantics. Neither metadata nor offline
verification establishes live account entitlement or paid-provider acceptance.
[Linux verification results](inference-resolution-verification.md) record the
supervised runtime, API and PTY checks performed for this resolution pass.

First send saves stable session, start and turn command identities locally before
requesting Vessel creation, then saves the exact revision-bound submission before
sending it. This uses the existing deduplicated start and submit protocol; the two
steps are not a cross-process atomic transaction. An interruption between them can
leave an owner with no admitted turn, attached to the recoverable first-send draft.
Reopening or reconnecting observes the saved creation and turn identities without
replaying Start, Submit or image uploads. Servers advertising `start_resolution`
can durably fence an unadmitted creation ID and restore the editable draft; older
servers can confirm an existing owner but leave absent-owner uncertainty fenced.
A confirmed owner with no attempted turn requires an explicit Enter to continue.
Attempted submissions use receipts and exact-envelope resolution, never replay.
Pending text/settings remain frozen until admission is known or the original ID
is durably closed. Helm never silently creates a replacement session.
Definite initial creation refusal preserves an editable draft; definite first-turn
refusal preserves its text on the created voyage. Recovery retains pinned policy
selection and explicit overrides, which are revalidated before local launch.

The on-disk registration's `starting` field is initial metadata, not a current
health reading: Vessel derives live state using authenticated runtime inspection.
Existing empty or unavailable voyages are not removed by draft handling.

When a runtime disappears without clean suspension evidence, an ordinary history
or turn request asks Vessel to run the existing exclusively fenced recovery with
no operator attestations. Recovery records interrupted work without replaying it,
then respawns the same session in a fresh voyage process. Cleanup that cannot be
verified is retained as unresolved history, separately from next-turn admission.
Linux launches include a separate child-subreaper guardian. After the runtime exits,
the guardian stops and reaps remaining descendants, including detached sessions,
and atomically records incarnation-bound evidence. Recovery uses this evidence to
close local terminal obligations and resolve that incarnation's host cleanup records.
It appends unknown outcomes for interrupted tool calls, never success or replay.
A changed Linux boot identity also establishes that the recorded local processes
are gone. Remote participant obligations remain unresolved until independently
observed; they are never replayed as part of continuation.

Recovery uses one stable automatic command identity per incarnation and checks
again when later requests arrive; cleanup uncertainty no longer disables retries.
Authorized saved snapshots, history, output and receipts remain readable through
an exclusively fenced helper even when cleanup cannot be verified. Invalid saved
execution settings withhold optional model/access metadata, rather than the history.
Helm shows a nonblocking notice when previous effects remain unknown and preserves drafts.
Vessel retains the last bounded canonical voyage name as
catalogue metadata, so Helm can identify saved conversations while recovery runs.

After a constructed run stops, its voyage retains cleanup tasks, resource managers,
checkpoint ownership and host reservations until cleanup is observed. A failed
cancellation monitor explains an interruption but does not permanently poison
cleanup: its original in-flight database read is retained until observed, and
outstanding checkpoint callbacks continue to hold the run fence. Cleanup observes
subordinate tasks, tools, root terminals and runtime controls. Finished terminals
are explicitly closed before the run blocker clears.

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
suspend transparent. Older owners have no guardian evidence; their history remains
readable, but unresolved cleanup cannot be retrospectively declared observed.
Missing process evidence and pending remote cleanup no longer permanently block new
turns. Inaccessible storage, a live owner fence, invalid execution configuration or
ambiguous tool history can still prevent continuation.
Guardian cleanup is bounded to ten seconds and does not establish rollback or the
completion of external effects. Linux evidence does not establish native behavior
on macOS or Windows. The guardian does not own a session journal or run an agent loop.

Helm chat, one-shot runs, connected clients, workflows and managed sessions use **Helm → Vessel → voyage**. Vessel launches a separate
long-lived process for each session. Only that voyage constructs the executor,
loads execution-host credentials, admits turns and writes canonical checkpoints.
Helm links shared configuration and presentation types but has no embedded agent
loop. New local voyages created inside ordinary chat preserve its startup
configuration through the private host handoff; explicit connect and remote
creation use executing-host configuration. Closing Helm detaches; it does not cancel accepted work.

Linux local discovery starts an absent supervisor from companion binaries. The
supervisor binds an ephemeral literal-loopback HTTP endpoint and atomically publishes
its bearer credential in the owned private Vessel directory. Helm validates that
directory, credential file and endpoint before opening one authenticated duplex
socket per connection activation. Commands, correlated replies, durable invalidations
and the typed browser notification extension share that socket. The connected TUI
and plain/run followers fetch canonical snapshots/output when notified; reconnect
uses the last snapshot cursor and never resubmits work. Remote HTTPS origins upgrade
to WSS with private credentials; local connections use protected loopback WS.
There is no silent HTTP/SSE fallback. See [duplex transport](duplex-transport.md). Legacy sharing grants remain
bound to one session, principal, workspace, rights, revision and expiry. Explicit
owner-approved pairing grants permit catalogue access and creation across selected
canonical workspaces, with separate rights and runtime-checked revocation/expiry.
They do not grant OS account or host-administrator authority. SSH transport is
removed.
Provider credentials are never copied between Vessels by these transports.

Vessel retains private supervision metadata, serializes starts,
checks exact session/incarnation health and refuses stale endpoints. Stored PIDs
are not evidence of survival. Restart requires positive stopped or fenced recovery
evidence. Unavailable owners are fenced; uncertain tools are never automatically
replayed. Stopping a supervisor leaves independent voyage processes running.
Vessel has no concurrent voyage-count cap. The legacy `local-serve --capacity`
option is accepted but ignored; capabilities report `capacity: null`. The 4,096
registration retention bound, transfer receipt safeguards, connection bounds and
per-voyage resource controls still apply.
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

Drag the voyage sidebar's right-hand divider (marked **↔**) with the left mouse
button to resize it. The divider highlights on hover or while dragging. Its width
starts at 30 columns and is remembered across voyage switches in the current Helm
instance, not across restarts. Width is limited to 24–64 columns while retaining
at least 80 columns for the conversation. The sidebar still hides when its available
pane is narrower than 110 columns. Temporary layout constraints do not overwrite
the preferred width. Close action menus, requests, or inference pickers before
resizing. Release the mouse to finish; focus loss, terminal resize, keyboard input,
and private-terminal handoff also end a drag. Mouse reporting requires a supporting
terminal.

Sidebar voyage titles retain state colours without repeated labels: running is dark blue,
idle/finished is green, needs attention is orange, and failed is red. Terminal
results retain their status for five minutes by default, then become **Settled**.
Set `HELM_SETTLE_AFTER_SECS` to a nonnegative integer number of seconds before
starting Helm to change this period; `0` settles immediately. Invalid values fail
before terminal setup. This is a Helm presentation setting, loaded at startup.

Settled voyages are grey. They sort below active and recent results, retaining
newest turn-end order within each group (including failed, cancelled and interrupted
turns). Voyages with no recorded turn end use creation time; unknown timestamps
sort last, with stable identity ties. In-flight messages do not change recency.
Every entry uses two terminal rows (title and divider),
with mouse and keyboard using the same order. Muted rules separate entries; titles
stay on one clipped row. Selection uses bold/reverse styling. A muted trailing
Vessel label appears only when the list spans multiple Vessels; Actions → Details
retains Vessel identity even when that label is omitted or clipped. Explicit status
remains in the selected voyage header, including when colors are disabled.

There are no voyage Unread markers or review acknowledgements. Viewing, leaving,
or covering a conversation does not change its timer. The timer uses the matching
run's canonical completion timestamp; older peers without that timestamp use a
Helm-local first-observed time saved with the draft by run UUID. Idle voyages use
their creation time, with the same local fallback. Restarting Helm, polling,
renaming, and process incarnation changes do not reset a saved timer. A new run
gets a new timer. Existing acknowledgement fields are ignored when loading older
drafts. Failed timing persistence is reported and may reset the fallback after a
restart. Separate Helm installations can have different fallback times for older
peers; canonical timestamps use the executing host's clock.

Settled describes presentation independently of whether the executor is live or
suspended. Settling does not suspend a process or change canonical history.
Active work, pending delivery, decisions and unresolved cleanup prevent settling.
Disconnected, unavailable and stopped owners retain their explicit status rather
than being inferred settled. Full terminal outcomes remain in the conversation.
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
Archive/Restore, Branch, Cancel for an observed active run, Details, Compact older
messages, Clear conversation and separated Delete; Export is intentionally absent. Disabled actions explain their state
requirements. Rename/branch input and typed DELETE confirmation are separate from
the saved composer draft. Menus retain their target/incarnation and deletion
confirmation checks that the observed history revision has not changed. Clear
requires `CLEAR`; compact requires `KEEP N` (1–100000), with the same history
revision check. These controls explain retained evidence/omitted content and keep
the unsent composer; compaction does not generate a summary. Archived
voyages must be restored before rename, branch or delete. Successful deletion
shuts down the runtime, retains its receipt with cleanup evidence, and removes
the tombstone from Helm's voyage lists without reusing its identity.

Actions and runtime question/permission requests share the [right-panel controls](ui-right-panels.md):
rounded chrome, padded content, hover/selection/disabled styles, and mouse buttons
for confirmation, back/close, scrolling and text paste/clear. A heading Actions
button remains available when voyage navigation is hidden. Access review scrolls
in narrow layouts and enables confirmation only at the end of its disclosure.
Esc still skips a question, denies permission, or backs out of a custom editor.

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
history, using the current Vessel runtime binary. Conversation summaries label successful completion
Finished for 24 hours after its durable completion timestamp, then Settled; the sidebar
uses the time-based styling and compact title presentation described above. Failed,
cancelled and cleanup-pending outcomes remain distinct. Bounded one-shot helpers
serve suspended observations without waking an executor. Supervisor-owned reads
and scoped stop checks use the currently configured runtime binary, not a retired
executable saved before an upgrade; this does not change the persisted owner
incarnation or its cleanup proof. Initialization and
management-only processes retire after a short idle grace; volatile private workflow
preparation retains its existing bounded lifetime. Root terminals close before
suspension; terminal metadata does not imply a live process across turns.
Canonical SQLite history, command bindings, streamed checkpoints, decisions,
steering and cleanup obligations survive interface disconnect. Exact command
retries retain their original outcome, including definite rejections. A changed
principal or payload conflicts. An unknown outcome requires receipt inspection.

Steering accepts an observed session revision at or below the current revision:
ordinary checkpoints and other steering admissions do not invalidate guidance
for the exact active run. Future revisions are refused. The observed revision
remains part of the immutable command payload; exact retries retain their original
outcome, including older `stale_revision` refusals. Process incarnation, active-run
identity, current authority, deadlines and queue bounds still apply. This relaxation
is specific to steering, not configuration or history-sensitive mutations. Existing
runtime processes must be replaced through normal lifecycle handling before they
use changed admission behavior; a source update does not hot-patch a running voyage.

Queued steering is checked at the next safe execution boundary. If its deadline
has passed, Voyage atomically records `not_applied` with reason `expired` and
continues the run without adding that guidance to canonical or provider history.
Timely guidance behind an expired message can still apply in FIFO order. Expiry
alone neither fails the turn nor triggers another provider request; receipt lookup
retains the rejection after suspension. This does not extend admission deadlines
or weaken authority, cancellation, or real checkpoint-failure handling.

Ordinary JSON sessions import on explicit resume with the original UUID, source
revision and fingerprint. The source is fenced and retired before destination
execution. Legacy shared managed journals migrate the selected session, receipts
and actor identity into a dedicated owner; unrelated sessions remain untouched.
Abandoned legacy work has an explicit voyage maintenance recovery path. Import
publication failures retain provenance and never fall back to a writable old copy.

Agent-construction failures retain resource observers and attempt bounded cleanup.
While the owner remains live, only positively observed cleanup releases run
admission and host executor charges.
Snapshots and Helm show an authored startup-stage summary and distinguish a
retryable failed run from unconfirmed cleanup; underlying diagnostics are excluded.
After owner death, older stranded runs retain unknown cleanup separately and can
accept new turns automatically without a cleanup attestation.

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

Helm uses the independently versioned public Vessel API over `/v1/vessel/socket`.
The explicit HTTP `/v1/vessel/command` and SSE `/v1/vessel/events` API remain for
compatibility and non-Helm callers. Its explicit session operations are distinct from private
runtime commands. Vessel selects owners under lifecycle arbitration, translates
requests and normalizes responses; live-resource actions retain exact incarnation
fences. Public events follow session owners and report observed incarnation changes.
The private runtime IPC remains protocol v1. See [process access](process-access.md#wire-and-retained-state)
for wire examples and the coordinated Helm/Vessel gateway upgrade requirement.

The connected TUI combines local WS and scoped WSS routes. It retains separate
drafts, prompt navigation, scroll, pending command identities and observation
cursors per voyage. Switching views does not redirect in-flight actions. Background
voyages show retained result status and pending decisions; slow remote observation runs
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
runs do not accept steering.

Helm exposes named interactive terminals through the persistent F3 action. The
browser lists observed state and host, selects with Up/Down, and attaches with
Enter using the inventory's exact owning run and observed incarnation. Ctrl+]
returns from private input to the saved conversation draft. Stale inventories
cannot attach. F1 opens a scrollable guide; operator panels and command receipts
render readable metadata. Chat shows compact public tool targets and outcomes directly for up to three
consecutive calls. Longer sequences keep the newest three calls visible and group
only the older calls into independently clickable accordions. Group counts and
outcomes describe those older calls; expanding shows them in their original order
without duplicating the newest three. Ctrl+T controls all groups; individual saved
call details still expand by double-click, and raw result payloads remain excluded
from compact summaries. Turn separators
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
F8 opens a keyboard chooser for typed tool/task/subagent actions, saved workflows,
Access and Model selectors, plus machine cleanup information. Tool forms use
executing-Voyage schemas, named task/agent references and a separate scrollable
review before dispatch through the normal durable command path. Drafts stay intact;
changed observations require reopening the form. Unsupported structured/private
fields are unavailable rather than raw-JSON fallbacks. Idle tool discovery shares
the runtime's built-in definitions without starting an agent, MCP transport or PTY
process. It is preflight metadata, not permission/readiness; configured MCP tools
remain unavailable there until an active runtime discovers them.

Saved workflows use named executing-host inventory, exact digest trust, typed
public inputs, isolated masked private inputs and a separate executing-host preview
and submit confirmation. Required and supplied optional private references must
match the preview; values never enter composer drafts or public pending envelopes.
Private entry is discarded on cancellation, focus loss, route/owner changes or a
five-minute deadline. Once submission starts, the original public command is
persisted before private handoff. Lost results reconcile that identity without
replaying private values. Preview of a cleanly suspended voyage uses a read-only
helper and does not start an executor merely to review a workflow.

F2 opens a searchable named voyage/draft picker at all supported viewport widths.
It searches the selected current/archive catalogue, preserves unsent composers,
and captures stable route/session identities rather than list positions. Search
paste never submits or selects. Opening it over a pending request does not answer
that request; returning to the voyage restores review. F5 changes to Archives.
 A quiet voyage rail, borderless conversation and compact composer adapt
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
Local workers, including nested workers, use the owning voyage's decision interface
when one is available. Helm identifies the requesting worker in the approval panel;
approving one action does not mark its execution complete. Delegated access, tools,
roots and administrator ceilings still constrain the action, and an explicit worker
approval-deny policy cannot be widened by a descendant. Workers without a decision
interface refuse actions requiring fresh approval; already permitted actions run.
Worker clarification questions remain unavailable.

Approval cancellation follows the executing tool/worker, not the Helm connection.
Dropped requests, cancellation, authority invalidation and expiry clear pending
requests durably and emit observation updates. A reconnect can observe the same
unexpired request while its execution is still waiting; it does not replay work.
Waits are bounded by the configured command timeout, capped at 120 seconds. New
worktrees are placed under `.voyage-worktrees/<voyage-scope-hash>` inside the
workspace, with self-ignored scope metadata, so ordinary workspace read/write
authority covers isolated coding. Allocation still checks parent roots, symlinks,
command policy and inherited ceilings; it never delegates private session storage.
Retained worktrees from older runtimes keep their paths and authority requirements.
Finished workers owning worktrees remain retained for commit, integration and cleanup, even when
all ancestors have finished. Successful cleanup clears ownership and permits
normal archival; dirty worktrees are not silently discarded.

Ordinary screens use names and plain-language summaries rather than runtime
identifiers and schemas. Console rendering preserves blank/wide cells and uses
the runtime's cursor; a supplied program title appears in its private header.

Tools, policy, todos, subagents, terminals, workflows and model metadata have runtime
controls. Operator tool calls use the real authorized registry and admitted run
resources, including approval and completion accounting, without inventing a model
request. Saved workflows retain digest-bound trust and typed public inputs; secret
shell bindings travel through an expiring private input channel and are excluded
from durable command/history payloads. `WorkflowPreview` accepts an optional
`optional_secret_names` array of declared optional secret names (not values), at
most 32 unique names of 1–64 ASCII identifier bytes. Omission or an empty array
retains required-only preview behavior; required names are always included
implicitly and must not be repeated in the optional selection. Unknown names,
public parameters and malformed or duplicate names are refused. Preview responses
include sorted `secret_names` for the complete rendered reference set. Clients
selecting optional secrets must require exact name-set parity before submission:
an older Vessel/runtime rejects the new request field, and a missing or mismatched
response set is not permission to execute a different prompt. Private submission
and its mutation identity are unchanged. GitHub operator commands run in voyage with
exact attended publication decisions and canonical session references.

## Human Vessel connections

Helm treats failed three-second catalogue probes as unavailable Vessels, pauses
observation and background recovery for those routes, and keeps cached conversations,
drafts and exact pending identities. It does not infer remote completion or cancel
in-flight work. Ctrl+G opens the Vessels modal for explicit Connect / Retry; success
resumes observation without replay. Saved connection autoconnect applies at startup,
not as an endless offline retry loop. Other routes and draft editing remain usable,
and ordinary interactive chat stays open if local startup fails. See
[unavailable Vessels](vessel-connections.md#unavailable-vessels).

Normal interactive Helm startup loads remembered connections without shell flags.
The visible **Vessels** control, Ctrl+G and `/vessels` open a private connection
modal for pairing/import, authenticated review, aliases, autoconnect, live
connect/disconnect, renewal/replacement and forgotten-access restoration. It stays
centered over the current view and scrollable in narrow layouts. Remote setup input
is intercepted before composer clipboard handling and never becomes a conversation
message. The sidebar entry is the primary Vessels control; a top-right fallback
button appears only when that entry is hidden. Ctrl+G remains available in either
layout.

Immutable connection UUIDs and activation generations replace positional routing.
Disconnection observes task cleanup, retains exact pending identities and stops
observation, not accepted remote work. Older-generation updates cannot redirect
input or change the active view. Private credentials, pairing recovery and the
versioned address book are separate from provider configuration and model-driven
routes. Concurrent-window updates use per-record revisions and atomic private
storage. New remote drafts select approved workspaces and retain host-owned
configuration; no runtime is created merely by connecting or choosing a workspace.

See [Vessel connections](vessel-connections.md) for the owner invitation flow,
rights, migration, expiry, recovery and tunnel requirements. This is human-client
pairing; model delegation uses separately accepted participant bindings.
The executing owner can list workspace grants with `vessel list-connections`,
including offline, revoked and expired access, without printing credentials.
Lost-installation recovery uses explicit old-grant revocation and new pairing;
it does not transfer an old installation's identity or replay lost commands.

## Model-driven Vessel coordination

Voyage registers a native `vessel` tool for configured local and remote routes,
with a bundled coordination skill included only in executors that have the tool.
The authenticated supervised session identity supplies introspection context.
Catalogue search, target inspection and conversation pages support discovery;
steering targets active runs, submission continues idle voyages, and creation
starts an independent voyage with an initial task. Steering admission retains one
stable message timestamp through canonical application; legacy records retain
absent times. Follow-up reads/waits and
cancel, rename, archive and restore use the existing public lifecycle protocol.
Vessel capabilities include software version and stable Vessel identity. New
model-originated coordination messages retain sender and exact tool-call provenance;
Helm shows **Sent by _session name_**, with an underlined name that opens the sending
call expanded through an authorized connected route. Legacy/human messages are not
re-attributed. Unavailable or ambiguous source history is reported, not restored or
replayed automatically.

There is no related-voyages-only restriction. Actual execution policy and route
rights still apply. Mutation intents and exact wire requests are retained before
effects; uncertain requests are not blindly replayed. Independent voyages are not
subagents and remain outside the originating run's resource/completion accounting.
The tool does not expose private terminal input, raw credentials, administrative
attestations or an arbitrary public-command escape hatch. See
[Vessel coordination](vessel-coordination.md) for the interface and limitations.

## Resources and execution policy

Native OpenAI Chat, OpenAI Responses, Anthropic and ChatGPT OAuth transports retain
streaming/tool behavior. Native API providers do not require Codex. OAuth and API
billing remain distinct. The external Codex compatibility provider has been removed;
see [configuration](configuration.md#providers-and-authentication) for migration.
MCP stdio and Streamable HTTP tools run in Voyage, with bounded transports, offline
dynamic input/output schema validation and snapshotted capability manifests.
Stdio retains observed child-session cleanup; HTTP retirement does not attest to
remote effect cleanup. Ordered typed results and private session-owned artifacts
survive history and local branches. Vessel authorizes artifact downloads, and Helm
shows metadata and verifies downloaded bytes. See [MCP tools and artifacts](configuration.md#mcp-tools-and-artifacts)
for concurrency, schema limits, supported content and transfer limitations.

The live registry supplies tools and runtime instructions. Local roots, command
access-mode restrictions, approvals, cancellation, administrator ceilings and resource limits apply
to root and subordinate work. Application policy alone is not an OS sandbox.
Optional `[sandbox].mode = "required"` adds Linux x86_64 bubblewrap isolation for
Voyage subprocesses; default off mode retains application policy alone. Required
mode fails closed, and live read-only dispatch narrows new subprocess mounts.
Existing processes retain their original mounts. Native HTTP model transports are
separate from tool isolation. See [configuration](configuration.md#optional-linux-process-isolation)
for readiness diagnostics, network grants and resource-limit scope. Execution
configuration is loaded and revalidated on the executing host; routing cannot
broaden it. External content remains untrusted.

At the model's final response, Voyage derives completion automatically from the
run's recorded task and agent outcomes. Completed items need no separate review,
fingerprint exchange or disposition call. Unfinished todos and unsuccessful agents
remain incomplete; missing records and active agents remain unresolved. When a normal
model final leaves owned work unfinished, the same active run gets one runtime-generated
system notice suggesting it finish what remains. It is not attributed to the user or
stored as a user/history message. Useful children are not shut down before this
continuation, and the writer lease is released so tools can finish work. The premature
answer stays in canonical history but is omitted from outgoing continuation projections
to avoid assistant-prefill semantics. User steering, cancellation and normal inference
limits still apply. If the model finishes again without resolving the work, the run
ends Incomplete without another reminder. There is no extra per-item sign-off or
bookkeeping deadline. Remaining owned agents then receive bounded shutdown and a fresh
observation before the final decision is saved under the existing writer lock. Resource
cleanup still requires actual observation before another turn can start.

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
stale-content and Git conflict checks. Host accounting records
cleanup obligations without an account-wide executor or PTY quota. Unconfirmed process
death retains evidence without blocking unrelated voyages. Guardian-backed recovery
resolves only that incarnation's scoped records after verified cleanup; legacy records
still need reconciliation/attestation. Completion evidence is
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

## Participants and transfer

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

Outbound worker mode and its enrollment relay are retired. There is no compatibility
API or automatic conversion of worker installations. Retired worker registrations,
bound journals and enrollment-bearing session grants are rejected; their private
data is not deleted. Remote human Helm
connections and participant execution use the scoped Vessel gateway. Shared local browser execution is explicitly opt-in through Helm's existing
full-duplex socket; it does not restore the retired worker relay. See
[local browser](local-browser.md) for setup, companion privacy, permissions,
provider limitations and recovery.

Declarative extensions, repository onboarding, configuration drafts and local
credential enrollment retain their explicit operator workflows. See
[operations](operations.md), [configuration](configuration.md) and
[security](security.md) for usage and authority boundaries.

Voyage Actions also includes **Access**: Read only, Ask first, or Unrestricted.
`/access` opens the same chooser; `/access read-only`, `/access approval` and
`/access unrestricted` open a confirmation for that mode. Slash completion offers
all three. The chooser shows effective access and preserves the composer draft.
Changes require executing-account owner authority and are allowed during an active
run. Conversation messages and tool-result checkpoints do not invalidate an open
Access confirmation. The owner checks the reviewed revision against intervening
access, inference, model or full-configuration changes, including changes arriving
before durable acceptance; those still require reopening Access.
New tool admissions use the updated mode; pending tool approvals are invalidated
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
