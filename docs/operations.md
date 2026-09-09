# Operating the current implementation

Helm interfaces reach independent voyage processes through Vessel. Ordinary chat,
one-shot runs and managed sessions share that process boundary. See [Current state](current-state.md) and the
[Architecture](architecture.md). Scoped grants, participants and signed owner moves
are documented in [process access](process-access.md).

Examples use installed executables. In a checkout, use `target/release/helm`,
`target/release/vessel` and `target/release/voyage` after building the workspace.
Replace UUID, revision, deadline and digest placeholders with actual observations;
workspace paths refer to the executing host. Configure that host's provider before
submitting work; see [Configuration](configuration.md).

## Paste images in the composer

Copy an image (including an OS screenshot), then press **Ctrl+V** or **Alt+V** in
Helm's normal input box. The image becomes an inline `[Image N]` element at the
caret. Backspace/Delete removes an adjacent owned element. Enter sends text and
images in order; image-only first turns work. **No modal or capture confirmation
is needed.** Exact local image paths and file URLs can also be pasted directly.

Clipboard reads happen on the Helm host, even for remote voyages. Linux needs
wl-clipboard (Wayland) or xclip (X11); missing helpers give guidance while retaining
the draft. Acquisition is asynchronous and Escape cancels it. Existing limits are
four images, 2 MiB combined, with immutable private draft and no-replay recovery.
See [Images and pasted screenshots](multimodal-implementation.md) for shortcuts,
platform caveats, path parsing, limits and verification. Plain/line-oriented commands
do not provide a native image clipboard UI.

## Independent local voyages

Keep all three executables beside one another. On Linux, connected mode can start
an absent local Vessel in the default private state directory and leave it running
when the interface closes:

```sh
helm connect
helm connect list
helm connect new --workspace /absolute/project
helm connect inspect SESSION_UUID
```

`--directory /absolute/private-vessel` selects another local installation;
`--no-start` requires an existing supervisor. For deliberate service activation,
use the [installer instructions](../installer/README.md). A foreground supervisor
can also be started explicitly:

```sh
vessel local-serve --directory /absolute/private-vessel \
  --voyage-binary /absolute/bin/voyage
helm connect --directory /absolute/private-vessel
```

Vessel does not cap concurrent voyage count. Existing `local-serve --capacity N`
arguments are accepted but ignored. New service units omit that legacy option;
the installer recognizes both templates during upgrades. Runtime resource limits
and supervisor record-retention safeguards still apply.

The connected TUI uses Ctrl+N to create, Tab/Shift+Tab to switch, and Ctrl+C/Ctrl+Q
to detach. `/new /absolute/workspace`, `/use SESSION_UUID`, `/rename NAME`,
`/model MODEL`, `/cancel`, `/receipt`, `/help` and `/quit` address the selected
voyage. Ordinary text submits a turn when idle and steers the exact observed run
when active. A refused or uncertain command retains its draft; Helm checks uncertain delivery
automatically before allowing another send. Switching views does not cancel or redirect outstanding work.

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

F1 opens a scrollable keyboard guide. PageUp/PageDown scroll the current view;
Esc returns from details to the preserved conversation draft. The footer always
shows **F3 Console**, including on layouts without the voyage sidebar. Operator
overviews use task-specific summaries; runtime identifiers, schemas and wire
receipts are kept out of the ordinary screens. Helm checks the status of an
unconfirmed action automatically without editing or resending the conversation draft. Authored
code remains readable in conversation; stored operator results receive plain-language
summaries without changing the saved history.

Conversation messages keep their authored text, including JSON examples. Helm
only summarizes explicitly identified operator actions. User messages, updates,
answers and recorded run outcomes have distinct labels. Up to three consecutive
tool calls show compact action details and outcomes directly: commands, paths,
search terms and edit sizes. Four or more calls form one accordion; click its
header to expand that group, or use **Ctrl+T** to expand/collapse all groups.
Narrative updates and turn boundaries separate groups. Raw result payloads,
program input and environment values stay out of the activity view.

Completed, interrupted and incomplete turns end with a separator. New runtimes
record start and finish timestamps and show elapsed time there. Older histories
without recorded timing show the outcome without inventing a duration.

**Ctrl+F** finds text in the loaded conversation; Enter advances to the next match
and Esc closes search without changing your draft. PageUp/PageDown and the mouse
wheel keep an earlier reading position anchored while output arrives. **Ctrl+End**
returns to the latest output. **Ctrl+Home** loads 128 earlier messages and retries
incomplete loading. Oversized messages load automatically from the same saved
revision. Reading is bounded to 16,384 messages and 64 MiB of public history; use
`/export PATH` for a complete larger transcript. Search covers the loaded section.

Pending questions and approvals open automatically in the conversation area,
with the first choice highlighted and the conversation draft preserved.
Archived conversations retain already loaded text;
a stopped archive reopened without cached text must be restored before history
can be read. Restore does not resume unfinished work automatically.

A failed startup displays its stage (policy, accounting, subagents, tools or
provider configuration). When cleanup completes, fix the indicated configuration
or resource problem and submit a new message. When cleanup is unconfirmed, further
work remains blocked: preserve the session and use the explicit runtime recovery
procedure after stopping and inspecting its exact owner. Restarting Helm or
resending the original command does not clear this obligation.

Separate voyages may run and delegate simultaneously in the same workspace.
Their histories, tasks and subagent ownership are separate; each voyage still
allows only one active root run. Workspace files remain shared, so coordinate edits
to the same files or use separate workspaces for independent changes. Host resource
limits and shared inference accounting apply across all voyages.

### Interactive terminals and passwords

Press **F8 Explore** to choose a readable overview of tools, permissions, tasks,
delegated work, workflows, models or machine capacity. Up/Down chooses, Enter
opens, and Esc returns without changing the message draft. These are read-only
overviews; they do not change configuration. The wide layout keeps voyages in a
full-height rail, with the conversation and composer together on the right.

Press **F3** (or type `/terminals`) to open the selected voyage's terminal browser.
Use Up/Down to select a named running terminal and **Enter** to attach. No UUID
copying is needed. The browser shows the executing host and observed program state;
"running" does not imply that a program needs input or that an update completed.
Empty inventories and unavailable or stale observations have explicit messages.
Attachment is disabled until a fresh observation identifies the terminal's owner.

The **PRIVATE TERMINAL** screen sends keyboard input directly to that program.
Enter passwords only there; programs commonly hide password characters. **Ctrl+]**
returns to Helm and its saved draft. In that screen Ctrl+C interrupts the program,
not Helm. Detaching leaves the terminal running. Human attachment permanently
switches that terminal to private capture; subsequent terminal output and human
input are excluded from model history. Inspect progress in the private terminal.

Helm observes terminal metadata without attaching or executing a tool. It uses the
inventory's owning run, including retained terminals after a run completes, and
rejects stale runtime incarnations. Console rows preserve blank cells, wide text
and combining characters, resize on attachment, and show the program cursor. Helm
uses standard terminal text and borders rather than requiring an icon font; actual
font coverage still depends on the user's terminal. Very small windows show a
resize message rather than permitting an unseen terminal selection.

### Approvals and questions

Questions and approvals for the selected voyage open ready for keyboard input.
Use Up/Down to choose and Enter to confirm. Questions start on the first offered
answer; select “Write a custom answer…” and press Enter to open the answer editor.
Enter sends your written answer; Esc returns from the editor to the choices.
Esc from the choices skips a question or denies an approval. Approvals offer
“Deny” (selected initially) and “Allow once.” No focus shortcut is required.

Left/Right switches between simultaneous requests. PageUp/PageDown scrolls long
request details; changing choices brings the selection back into view. Background
decisions appear in the voyage list. Each question retains its answer draft while
navigating requests in the current UI.
Question responses are model input, never execution approval. Pasting text only
edits a draft; it does not send an answer or authorize an action. The main composer
remains separate and responses preserve its draft. Uncertain response delivery
retains its command identity; Helm automatically checks an unconfirmed response. Unknown future interaction kinds remain visible without response controls.

`/approve DECISION_UUID` and `/deny DECISION_UUID` answer
an exact approval; `/answer DECISION_UUID text` answers a model question. There is
no approve-all operation. Responses are tied to the observed decision, run,
incarnation and revision, and an expired request cannot authorize an action.
Decision waits are bounded; leaving an unanswered prompt is not approval. Expiry
is reported as an unanswered approval, separately from an explicit refusal or
cancellation. Local worker approvals appear here too, labelled with the worker's
name, including requests from nested workers. Allow once authorizes only the
pending action within its existing execution policy; it does not approve the
worker's whole task. Cancelling a worker clears its pending request. Leaving Helm
does not cancel work: reconnect before the deadline to answer the same request.
The wait uses `command_timeout_secs`, capped at 120 seconds. Expired requests are
closed; a later reconnect does not retry the action. Workers with no approval
interface refuse actions needing fresh consent. Worker questions are not routed.
`/tools`, `/policy`, `/todos`, `/subagents`, `/workflows` and `/models`
inspect runtime controls; Esc returns to history. `/tool NAME JSON`
executes a real authorized tool, using an operator run when idle. `/terminal UUID`
opens a separate private attachment (Ctrl+] detaches). `/configure /absolute/host/file`
applies trusted host settings while idle. `/branch [name]`, `/archive`, `/restore`,
`/clear SESSION_UUID`, `/compact RETAIN_COUNT`, `/delete SESSION_UUID` and
`/export local-new-file.md` provide lifecycle controls. Clear retains old run/receipt
evidence; delete purges text after cleanup and leaves a tombstone.

Idle operator tool runs report setup, tool and finalization failures separately.
These summaries exclude underlying diagnostics. A failed finalization may follow
a tool effect that already happened; it does not roll back or replay that effect.
Cancellation remains a cancelled outcome, and another turn requires observed
cleanup. If terminal persistence itself fails, the retained cleanup/recovery
obligation remains authoritative; an accepted command is not proof of success.

For plain automation, capture identity before mutation and preserve exact commands
when delivery is uncertain:

```sh
helm connect new --id SESSION_UUID --command-id CREATE_UUID --workspace /absolute/project
helm connect submit SESSION_UUID --expected-revision REVISION \
  --command-id COMMAND_UUID --expires-at-ms DEADLINE_MS "Describe the workspace"
helm connect receipt SESSION_UUID COMMAND_UUID
helm connect inspect SESSION_UUID
helm connect cancel SESSION_UUID --run RUN_UUID --expected-revision REVISION \
  --command-id CANCEL_UUID --expires-at-ms DEADLINE_MS
helm connect stop SESSION_UUID --incarnation INCARNATION_UUID
helm connect restart SESSION_UUID --incarnation INCARNATION_UUID --command-id RESTART_UUID
```

Deadlines are Unix milliseconds, not durations. Revisions and run/incarnation IDs
come from current inspection. A receipt records admission, not successful execution
or observed cleanup. An unknown receipt does not authorize a different resubmission.
In Helm, automatic recovery resolves an ordinary pending send without repeating it
(`/receipt` is also available as an optional diagnostic):
a saved receipt confirms delivery, or the runtime durably prevents that command ID
from being admitted and returns your editable draft. Send the draft again only after
that definitive result. Status checks are bounded and non-overlapping, with a five-second pause before
checking unresolved outcomes again; an unavailable or older owner
leaves the original identity saved. The CLI `connect receipt` remains a read-only
lookup and does not close an unadmitted command ID.
A stop response can precede observed cleanup; inspect again. Never infer survival
from a stored PID or force restart by removing registration/cleanup records.

Snapshots bound message and partial-output size and mark truncation. Helm loads
oversized messages and offers earlier history in place. The typed request command
also provides paginated history and full message/output chunks:

```sh
helm connect request SESSION_UUID '{"op":"history","offset":0,"limit":32,"expected_revision":null}'
helm connect request SESSION_UUID '{"op":"decisions"}'
```

History replies include revision, message offsets and continuation fields. Pass the
observed revision on later pages when consistency is required. `message_chunk`
requires `index`, UTF-8 byte `offset`, `limit` (1–65536) and `expected_revision`;
its data chunks form a public message JSON projection. `run_output` requires an
exact `run_id`, byte `offset` and `limit`. Follow returned `next_offset` values;
never split a UTF-8 character or treat partial assistant output as accepted history.

## Remote HTTPS routes

Start Helm normally and choose **Vessels** (also Ctrl+G or `/vessels`) to add or
import a remote connection without restarting. Pairing, saved connections, remote
workspace selection and access recovery are covered in
[Vessel connections](vessel-connections.md). Provider credentials stay on the
executing host. See [process access](process-access.md#scoped-remote-access) for
the gateway and TLS setup. Explicit CLI routes remain available for scripts:

```sh
helm connect --access-file /absolute/private/session-access.json
helm connect --access-file /absolute/private/session-access.json --include-local
helm connect --access-file /absolute/private/session-access.json list
```

Repeat `--access-file` for up to 16 grant routes. `--include-local` adds the local
Vessel to the TUI. CLI subcommands require a single route, so do not combine
multiple credentials or `--include-local` with a CLI subcommand.

Legacy session grants remain restricted to one voyage. New workspace pairing
grants permit creating additional voyages only in owner-approved workspaces and
only with the `create` right; neither kind grants account-owner administration.
HTTPS routes receive streamed SSE invalidations. Disconnecting does not cancel
accepted work and reconnecting does not replay commands. Saved interactive
connections do not change the target of an explicit CLI subcommand.

The legacy `--ssh`, `--remote-directory`, `admin move --destination-ssh` and
`vessel local-request` options have been removed. There is no automatic SSH-to-HTTPS
migration. Existing saved SSH drafts and journals are preserved, not rerouted or
replayed; reconcile uncertain operations on their original executing host.

## Ordinary local work and migration

```sh
helm --workspace /absolute/project chat
helm --workspace /absolute/project chat --plain
helm --workspace /absolute/project run "Describe this repository"
helm sessions
helm chat --resume SESSION_UUID
helm run --resume SESSION_UUID "Continue the previous work"
helm connect export SESSION_UUID /absolute/new-transcript.md
```

Chat uses the multiplexer when attached to a terminal; `--plain` selects the line
interface. One-shot runs stream durable output. Ctrl+C and EOF detach rather than
cancel accepted work. Use explicit cancellation when intended. A resumed session
keeps its saved workspace/model; explicit configuration changes require idle state.
Ordinary JSON sessions import on explicit resume with their original identity and
fingerprint, after exclusive source fencing. An already-retired source cannot be
reopened as a competing owner.

`run --no-save` still uses a durable temporary runtime journal so interruption cannot
lose cleanup obligations. It deletes application records after observed completion
and cleanup. Resuming with `--no-save` first branches; the original remains intact.
An uncertain command or cleanup leaves recoverable records and reports the reason.
This option does not promise that content never reaches disk or forensic erasure.

## Private managed sessions

A managed installation is an absolute directory, separate from a workspace. Its
supervisor lives under `vessel/`, with one dedicated journal per session. Existing
legacy `journal/` sessions migrate on submit only after exclusive fencing and
cleanup; use recovery first for abandoned legacy work.

```sh
STORE="$HOME/.local/share/helm-managed"
helm --workspace /absolute/project managed --directory "$STORE" create --name "Project work"
helm managed --directory "$STORE" list
helm managed --directory "$STORE" --json submit SESSION_UUID --expected-revision REVISION \
  "Inspect the project and explain its build setup"
```

Create cannot replace an existing identity. Recover a lost response with list;
`--limit` and `--after` bound metadata pages. Listings exclude transcript/provider
state. Submit prints an admission receipt before output; JSON mode emits bounded
output, decision and completion records. Decisions can be answered from another
connected interface. Plain attended submission presents approval/question prompts.
Persistent root terminals remain owned by the independent voyage between turns.

## Exact retries, cancellation and recovery

Helm observations and new turns automatically ask Vessel to recover and respawn an
unavailable owner when exclusive recovery can do so without an attestation. This
preserves the session and history, records interrupted work, and never repeats a
turn or tool. If cleanup, a retained resource or a tool outcome remains uncertain,
automatic recovery stops. Inspect the recovery disposition and use the explicit
commands below only after checking the named effect or resource. Vessel's catalogue
retains the last observed canonical name while the voyage process is absent.

When a submit response is uncertain, preserve its original command UUID, expected
revision, deadline and prompt. Observe it by repeating the exact request:

```sh
helm managed --directory "$STORE" --json submit SESSION_UUID \
  --expected-revision REVISION --command-id COMMAND_UUID --expires-at-ms EXPIRY_MS \
  "The original prompt"
```

An identical retry observes the original run, even after the original deadline;
it never repeats tools. Changing input under the same ID is refused. Receipt
observation can return exit status zero for an active or failed run: inspect its
`existing_run` state and cleanup metadata rather than treating that exit as completion.

```sh
helm managed --directory "$STORE" cancel SESSION_UUID --run RUN_UUID
helm managed --directory "$STORE" recover SESSION_UUID
```

Cancel records intent for that exact run. Its owner cooperatively stops work and
cleans up; acknowledgment is not proof that effects stopped. Ctrl+C detaches the interface; use cancel to request cleanup. Recovery requires exclusive ownership, marks abandoned
execution interrupted and saved PTYs stale, and preserves conversation/evidence.
It neither replays tools nor claims terminated processes survived.

Pending cleanup blocks new submissions and survives ordinary recovery. Independently
stop the old effects before recording an operator attestation:

```sh
helm managed --directory "$STORE" recover SESSION_UUID --acknowledge-cleanup RUN_UUID
```

An attestation differs from observed cleanup. If interrupted tool calls have no
recorded result, inspect the workspace and current revision, then reconcile:

```sh
helm managed --directory "$STORE" recover SESSION_UUID \
  --reconcile-tools RUN_UUID --expected-revision REVISION
```

Reconciliation records unknown/interrupted outcomes without repeating effects or
inventing success. A `run_unconfirmed` result preserves the actual saved state;
dropping an execution future does not prove its work stopped. Journal schema is
currently 8. Stop owners and resolve interrupted work before explicitly running
`helm managed --directory "$STORE" upgrade`. Keep private backups; never remove
cleanup or ownership records to force admission.
