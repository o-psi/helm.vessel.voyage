# Operating the current implementation

Helm interfaces reach independent voyage processes through Vessel. Ordinary chat,
one-shot runs, managed sessions and the outbound compatibility adapter share that
process boundary. See [Current state](current-state.md) and the
[Architecture](architecture.md). Scoped grants, participants and signed owner moves
are documented in [process access](process-access.md).

Examples use installed executables. In a checkout, use `target/release/helm`,
`target/release/vessel` and `target/release/voyage` after building the workspace.
Replace UUID, revision, deadline and digest placeholders with actual observations;
workspace paths refer to the executing host. Configure that host's provider before
submitting work; see [Configuration](configuration.md).

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
cancellation.
`/tools`, `/policy`, `/todos`, `/subagents`, `/workflows` and `/models`
inspect runtime controls; Esc returns to history. `/tool NAME JSON`
executes a real authorized tool, using an operator run when idle. `/terminal UUID`
opens a separate private attachment (Ctrl+] detaches). `/configure /absolute/host/file`
applies trusted host settings while idle. `/branch [name]`, `/archive`, `/restore`,
`/clear SESSION_UUID`, `/compact RETAIN_COUNT`, `/delete SESSION_UUID` and
`/export local-new-file.md` provide lifecycle controls. Clear retains old run/receipt
evidence; delete purges text after cleanup and leaves a tombstone.

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

## SSH-account compatibility route

Start a Linux Vessel on the remote machine and put `vessel` on that account's PATH.
Configure SSH authentication and known-host verification independently. The SSH
helper is noninteractive, does not forward the agent, and does not provision or
start a remote supervisor:

```sh
helm connect --ssh USER@HOST --remote-directory /absolute/private-vessel
helm connect --ssh USER@HOST --remote-directory /absolute/private-vessel --include-local
helm connect --ssh USER@HOST --remote-directory /absolute/private-vessel list
helm connect --ssh USER@HOST_A --remote-directory /absolute/vessel-a --ssh USER@HOST_B --remote-directory /absolute/vessel-b --include-local
```

Pair each repeated `--ssh` with one `--remote-directory` in the same order, up to
16 remote Vessels. Plain commands select one Vessel, so do not combine multiple
routes or `--include-local` with a CLI subcommand. Remote `/new` needs an explicit
absolute path on the remote machine.
SSH-account authority is broad local-account authority; this is not enrolled
per-session grant delegation or participant-Vessel execution. The provider and its
credentials remain on the remote executing machine. Snapshot polling/reconnect does
not replay work, and SSH loss does not request runtime cancellation. This adapter
does not carry SSE; use a scoped HTTPS credential for streamed remote updates.

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

## Supervised outbound compatibility relay

The compatibility relay exposes one dedicated supervised voyage through an outgoing
enrollment connection. `helm remote-worker` starts it through local Vessel and exits. Enrollment
and presence alone grant no execution or access to existing private sessions.
Configure a private Vessel authority directory, operator token and canonical HTTPS
origin behind a TLS proxy on the Vessel host. Supply the token through the
`VESSEL_OPERATOR_TOKEN` environment variable rather than putting it in shell history.

```sh
vessel --bind 127.0.0.1:9480 --database /absolute/vessel.db \
  --attachment-directory /absolute/vessel-authority \
  --public-origin https://vessel.example --remote-execution
helm attachment --directory /absolute/enrollment --origin https://vessel.example \
  enroll --invitation-id INVITATION_UUID
helm attachment --directory /absolute/enrollment status
helm --config /absolute/provider.toml --workspace /absolute/project remote-worker \
  --directory /absolute/remote-installation --enrollment-directory /absolute/enrollment \
  --origin https://vessel.example
```

The authenticated operator creates an invitation with
`POST /v2/enrollment/invitations`, JSON body `{"ttl_ms":300000}`,
`Content-Type: application/json` and `x-voyage-request: 2`. Use an authenticated
HTTP client that supplies the operator credential privately; do not place it in
URL parameters or command arguments. Invitation TTL must be 1–900,000 ms. The
response contains `id`, the secret `key`, and `expires_at_ms`. Distribute the ID and
one-use key through a private channel, not chat or logs. Invitation creation does
not itself enroll a machine. Current enrollment administration also supports
`POST /v2/enrollment/revoke` with `machine_id`, `expected_epoch` and a stable
`transaction_id`; revocation is an explicit exact-epoch action.

Obtain an invitation and its one-use secret from the authenticated Vessel operator.
Enter the secret only at the hidden prompt; automation uses `--invitation-key-stdin`
with private piped input. HTTPS is required outside explicitly allowed literal
loopback development origins. The voyage connects outbound and needs no inbound task port.
The runtime creates a new dedicated session and later reopens only its exact binding;
it cannot relabel an existing private journal. Workspace/model bindings stay fixed.
Providers, credentials, execution policy and cleanup remain on the executing host.

Authenticated operator routes are `POST /v1/remote/MACHINE_UUID/command` and
`GET /v1/remote/MACHINE_UUID/events?session_id=SESSION_UUID&after=0&limit=32`.
POST requires `x-voyage-request: 2`; supplied browser origin/site headers must pass
origin checks. The command envelope carries a UUID, expiry and operation: `list`,
`inspect`, `submit` against an expected revision, or `cancel` for an exact run.
For example, the following request body inspects an existing session; substitute
actual UUIDs and an expiry in Unix milliseconds within five minutes:

```json
{
  "command_id": "COMMAND_UUID",
  "expires_at_ms": 1234567890000,
  "operation": {"type": "inspect", "session_id": "SESSION_UUID"}
}
```

For submission, use `{"type":"submit","session_id":"SESSION_UUID",`
`"expected_revision":0,"prompt":"Describe the workspace"}` as the operation.
The numeric revision must come from the current inspection. A cancel operation
contains `type`, `session_id` and `run_id`; a list contains `type`, `after` and
`limit`. These are operation payloads, not additional enabled CLI commands.
Preserve the entire envelope on uncertain retries. Inspection returns metadata,
usage and cleanup, not canonical history or provider continuation. Replay uses its
own contiguous public cursor; `snapshot_required` means inspect and resume there.
These compatibility routes do not provide create/history/model/root editing or
approval delegation. The separate scoped process gateway provides explicitly
granted history, lifecycle/model and decision operations; host/root configuration
requires local or SSH account-owner access. Disconnecting an HTTP observer does
not cancel execution; losing the
worker's Vessel link conservatively cancels it and stops dispatch.

## Withdraw and recover a remote grant

```sh
helm remote-consent --directory /absolute/remote-installation inspect
helm remote-consent --directory /absolute/remote-installation preview \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision GRANT_REVISION
helm remote-consent --directory /absolute/remote-installation withdraw \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision GRANT_REVISION \
  --confirm CONFIRMATION_DIGEST
helm remote-worker --directory /absolute/remote-installation --recover
```

Review the preview's destination and identity before withdrawal. Grant revision is
separate from session revision. Withdrawal permanently fences this grant's later
remote admission, reads and event publication, and requests cancellation. It cannot
recall already delivered/queued bytes. Preserve the exact withdrawal for receipt
recovery after uncertain acknowledgment. Restart cannot restore the retired grant;
future authorization needs a deliberate new dedicated installation and empty session.

Remote recovery is local and never reconnects. Its `--acknowledge-cleanup RUN_UUID`
and `--reconcile-tools RUN_UUID --expected-revision REVISION` options have the same
attestation/uncertainty meaning as managed recovery. Check pending cleanup locally
because withdrawal prevents a final public event. See [Security](security.md) for
trust boundaries and [Quality](quality.md) for the current verification status.
