# Voyage runtime contract

Status: component invariants for the [architecture](architecture.md). The Linux
process implementation follows these contracts; [current state](current-state.md)
and the [delivery ledger](implementation.md) distinguish source from observed
verification and unsupported platforms.

## Ownership and admission

A Vessel supervises one independent voyage process for each executing session.
The voyage process holds an exclusive session execution fence through run callbacks
and owned cleanup. A terminal turn may suspend only after cleanup is observed,
private preparation is resolved, and all session resources have been closed.
Suspension releases the process and fence, preserving the session and journal. All clients, including plain/one-shot
frontends, reach that same owner through Vessel. The selected Helm view has no
write authority over the canonical store.

Each consequential command binds the authenticated principal, session identity,
runtime incarnation, immutable command ID, expected revision, current authority
and expiry. Run-specific operations also name the exact run. Persist the accepted
command and its obligations before effects. A changed payload under an existing ID
is a conflict. An exact retry observes the original receipt; it does not dispatch
again. Loss of a response is not permission to allocate another command ID.

One mutating run may be active per voyage. Input during a run is explicit steering,
with bounded, durable queued/applied/not-applied receipts, rather than a second
simultaneous root run. Preserve a refused or uncertain composer draft. Multiple
interfaces racing a command must see a winner and a conflict or the same receipt.

Runtime construction registers cleanup obligations before starting resources.
Shared workspace writer arbitration and shared budget accounting must work across
voyage processes. When capacity is exhausted, fail or queue within documented
bounds; do not wait indefinitely or silently switch workspaces.

Persistent subagent ownership follows the session's agent tree and completion
store. Binding two voyages to the same workspace must not create a run-lifetime
exclusive lock between their independent stores. Shared-file operations and host
accounting retain their own coordination; session concurrency does not make
conflicting workspace edits safe.

## Persistence and observation

The voyage owns canonical conversation text, checkpoints, tool-effect intent,
run results and delivery receipts. A checkpoint is acknowledged only after durable
persistence. UI formatting, activity summaries and remote projections must not
rewrite canonical text. Runtime system instructions are not conversation messages.
Provider continuation remains private to the execution host.

Maintain one authoritative store per session. Existing JSON and managed-journal
stores must never admit simultaneous execution of the same identity. Any explicit
storage unification needs source fencing, durable provenance, retained originals,
bounded publication and truthful pending states. Failure after partial publication
must not create two writable histories or silently fall back to an old snapshot.

Events have ordered, projection-scoped cursors. Persist canonical changes before
publishing their committed observations. Bound frame size, queues, replay retention
and subscriber count. A slow or disconnected observer cannot block persistence or
consume unbounded memory. A replay gap requires an authorized snapshot plus a new
cursor, not a fabricated complete history. Reconnect retrieves state and receipts;
it never automatically resends turns, decisions or terminal keystrokes.

Report separate dimensions:

| Dimension | Examples of distinct observations |
| --- | --- |
| Connection | Connected, reconnecting, disconnected; last observation time |
| Run | Accepted, running, awaiting decision, completed, incomplete, cancelled, failed, interrupted |
| Command delivery | Accepted, applied, rejected, unknown |
| Cleanup | Pending, observed, operator-attested, unconfirmed |
| Runtime | Live authenticated incarnation, cleanly suspended, stopped, unavailable, ownership conflict |

An unavailable connection cannot establish a stopped runtime. A final answer cannot
establish resource cleanup. Operator attestation must remain distinct from observed
cleanup and retain its provenance.

Clean suspension has exact-incarnation durable evidence and no runtime endpoint.
Vessel resumes that state automatically with a new incarnation. An unavailable
owner can also recover automatically under exclusive fences after verified cleanup;
an explicitly stopped owner still requires restart. A command
that was positively refused before dispatch during suspension can cross that
transition; an uncertain external effect is never automatically retried. Bounded
one-shot observers read suspended history and receipts under the session fence
without an agent loop. Runtime policy and scoped grants are checked on the
executing machine for resumed work and again before releasing observations.

## Decisions and terminal interaction

Approval and question requests are durable and bounded. They bind session,
incarnation, run, request identity, exact action or question, authority and deadline.
The voyage authorizes each response against current policy. A stale response,
response to another voyage or second response cannot authorize an effect.

Background voyages expose pending decisions without redirecting the current input
target. Switching views preserves drafts and result-retention timers. Notifications carry
minimal metadata and require fresh authorization when opened. When no eligible
interface is available, the runtime follows bounded pending/refusal behavior under
local policy; it never waits forever or treats silence as approval.

PTY attachment is a separate exact-target channel owned by the voyage. Human input
is private terminal data, not model-visible chat or replay history. Detach before
switching its target. Sanitize untrusted terminal text in all ordinary UI paths;
only the controlled terminal-screen renderer interprets terminal output. Keep
provider/subprocess diagnostics out of the TUI's display stream.

## Provider outcomes, retries and accounting

Native provider completion requires a supported terminal signal. OpenAI Chat
output limits/content filtering, Anthropic output limits/pause/refusal, and an
OpenAI Responses incomplete event produce an **incomplete** run, not successful
completion. Malformed or unsupported terminal signals fail the run. Streamed text
remains provisional, and tools from truncated output are not dispatched. Native
Chat and Anthropic compatible endpoints must supply their finish/stop reason.

Transient provider failures retry only before any text or tool fragment arrives,
within `provider_retry_attempts`. Exponential delays use equal jitter between half
and all of the configured delay ceiling. An explicit `Retry-After` uses the server's
delay without jitter, including HTTP-date values and service-unavailable responses.
A delay above `provider_retry_max_ms` stops with the original failure category;
it is never shortened to retry early. The positive `provider_retry_elapsed_ms`
window includes request and wait time: no further retry starts after it expires.
This is a retry-admission bound, not a new deadline on an in-flight response.
Cancellation interrupts retry waits; policy freshness and execution authority are
rechecked while waiting and again before dispatch. Account
exhaustion and authentication failures do not retry. These bounds apply to retry
attempts, not ordinary successful model/tool cycles. Generic transport failures
are conservatively non-retryable because remote completion is uncertain; a timeout
before any response delta remains eligible under the configured attempt/window
bounds. Missing usage never means free or refunded work. Each admitted retry has
a fresh accounting attempt identity under the same logical request identity.

[Provider attempt observations](provider-attempts.md) persist content-free intent
before dispatch and the classified outcome before retry, separately from tool
outcomes and canonical messages. A partial text or tool-call delta prevents retry,
including context recovery. After terminal state and observed cleanup, the operator
may submit a new continuation turn after reviewing saved output/effects. It has a
new command identity; resending the old command identity only retrieves the old
receipt. No recovery automatically repeats tools or appends synthetic provider
errors to canonical history. Unknown tool effects retain the existing explicit
reconciliation requirements below.

Tool identities are checked across the complete response before dispatch. Exact
ID/name/semantic-JSON duplicates execute once; conflicting identities or malformed
names reject the batch without effects. IDs reused in a later response or run are
fresh calls. Durable command deduplication is a separate contract: replaying an
accepted command observes its original run, including after suspension. Neither
contract provides global exactly-once external effects.

Interrupted calls without durable results block another run until fenced
reconciliation after cleanup or durable retention of unresolved obligations.
Automatic recovery records its own provenance,
distinct from operator-selected reconciliation. Reconciliation appends unknown failed results and preserves the
original terminal outcome; it never repeats the effects. Its identity checks are
also response-local, so reuse in a previously completed response cannot prevent
recovery. Conflicts within the unresolved batch still refuse reconciliation.

The inference ledger attributes every admitted attempt and keeps missing usage
unknown. Reported usage from failed or incomplete streams remains there; completed
responses rejected by tool preflight also retain their totals in the canonical run
record. Usage is not a pricing or billing ledger. Public failure summaries identify
the authored failure category without exposing provider bodies, credentials or
subprocess diagnostics. Failure of a checkpoint retains recovery obligations and
cannot authorize a provider/tool replay.

## Completion, cancellation and recovery

Root terminals close before turn suspension; they have no implied lifetime between
turns. Terminal metadata and canonical history remain durable.

Register subagents, managed commands, terminals and completion obligations before
they become independently active. Finishing a model turn is not proof that all
owned work finished. Completion observes current run-owned outcomes automatically,
without a second model-authored sign-off. Completed records count directly;
unfinished tasks and unsuccessful agents retain incomplete outcomes, and missing
records or active agents remain unresolved.

If a normally completed model response has no tool calls but owned work remains,
Voyage offers one automatic continuation opportunity in the same active run before
sealing the final decision or shutting down useful descendants. Fixed runtime-authored
system instructions ask the model to inspect bounded unfinished-item IDs and finish
what remains feasible within the existing user request. This is **not a user message**,
steering receipt, tool result, or new authorization. Task titles/results are not promoted
into system instructions. The readiness snapshot is explicitly potentially stale.
The writer lease is released before inference so tools and children can make progress;
new user steering is still applied at the normal boundary.

Guidance stays in outgoing system instructions for the continuation's tool cycles,
not in canonical conversation history. The premature final answer remains in the
canonical transcript but is omitted from outgoing continuation projections. This
avoids treating a trailing assistant final as provider prefill without fabricating a
user turn. Native Responses/ChatGPT use `instructions`, Anthropic uses `system`, and
Chat Completions uses its leading system message. No provider replay metadata on
other messages is changed. Local request/encoding checks do not establish live
acceptance for every compatible endpoint.

A continuation can use ordinary model/tool cycles; it has no separate bookkeeping
deadline. It receives only one automatic reminder per active run invocation, so a
second final response with unfinished work ends honestly as Incomplete rather than
creating an endless prompt loop. This does not reopen terminal runs, retry provider
failures/truncated responses, resume cancelled execution, bypass context/account
limits, or create mandatory per-item review work. User requests to stop/pause and
real blockers remain controlling. No notice survives into a later run.

After the continuation, or immediately for clean work, bound any remaining owned-agent
shutdown and recheck under the writer lease before recording the final decision. Preserve
historical decisions and review evidence. Resource cleanup still requires observed
results; a completed task status does not prove that its processes have stopped.

Cancellation names the session and exact run. Persist cancellation intent, close
new work admission and cancel owned work. Keep the owner fence until cleanup is
observed or explicitly reported unconfirmed. A late model result cannot overwrite
a committed cancellation. Cancel one voyage without stopping unrelated voyages.
Helm detach is never a cancellation request.

A monitoring failure and resource cleanup are separate outcomes. Retain the original
in-flight observation and cleanup handles after a timeout. A live owner retries
failed observations in bounded batches without creating concurrent cleanup workers.
Observed callback termination, terminal/resource teardown and reservation release
permit continuation; a successful monitor result is not an extra prerequisite.
Do not lose ownership just because a run has reached terminal state.

After process failure, recover exclusive ownership before admitting another run.
Mark abandoned execution interrupted; preserve partial output, unresolved effects
and pending cleanup. Never replay an uncertain tool effect automatically. A stored
PID, heartbeat timeout or terminal metadata is insufficient proof that descendants
are gone. Fenced recovery moves unresolved cleanup into durable historical records
without confirming it, and permits the next explicit turn in the same conversation.
Original command identities remain terminal; unfinished tools receive unknown
outcomes and are not replayed. Snapshots expose `retained_cleanup` independently of
active `pending_cleanup_run` and resources. Later guardian cleanup applies only to
its own incarnation, never these retained unknown effects. Clear, compact and delete
remain blocked while they would discard context for retained unknown effects.

On Linux, a separate guardian can supply positive local cleanup evidence after
the owner exits. It becomes a child subreaper before launch, records boot and
incarnation identity before effects, signals only kernel-owned children through
pidfds, and requires the kernel to report no remaining children before recording
cleanup. It never reads or owns canonical session state. Recovery checks its
private evidence while holding startup and execution fences; remote assignments
remain separate obligations. Missing legacy evidence is never fabricated.
Read-only saved conversation access needs those fences and normal authorization,
but does not require successful execution recovery or valid execution configuration.

Vessel restart must reconcile runtime registrations against live authenticated
incarnations. It cannot steal a session fence or fabricate cleanup evidence. Runtime
restart may restore history but must not resume an unknown external effect.
Credential revocation and authority loss fence new dispatch and initiate the
specified bounded cleanup; disconnected enforcement remains explicitly unknown.

## Session lifecycle and participation

Create/open, rename, model change, branch, archive and confirmed deletion go through
the same owner and immutable command mechanism. Model changes affect subsequent
turns. A branch receives a new session ID and runtime process; it cannot inherit
live resources, credentials or execution ownership. Archive/delete require an
explicit disposition for active work. Keep enough receipt/tombstone evidence to
avoid resurrecting deleted sessions through stale commands.

Vessel membership is not an execution grant. Proposed participants must accept
bounded local bindings and disclosure policy. Before delegation, the voyage records
an assignment obligation; the participant records deduplicated acceptance before
effects. Unknown acceptance blocks reassignment to another participant. Results
retain source attribution and are disclosed only under current grants.

Removing membership requires visible drain-versus-cancel handling and stops new
routing. It cannot claim immediate remote cleanup. Moving the canonical voyage
process between Vessels requires proven source quiescence, durable relinquishment,
verified destination readiness and a committed ownership generation before
activation. There is no automatic failover based on timeout. These distributed
operations remain additional work beyond multiplexing independent voyages.

## Implemented process transport

Protocol version 1 uses bounded JSON over HTTP(S) between Helm and Vessel, with a
maximum 4 MiB request, response or SSE event. The local Vessel binds ephemeral
literal-loopback HTTP and accepts at most 64 concurrent authenticated commands plus
16 bounded multiplexed SSE clients. Scoped remote grants use HTTPS except an
explicitly enabled literal-loopback development origin. Request tokens and
process-registration files are private account data; public health alone never
proves authority. An unsupported protocol version is refused before dispatch.

`VesselRequest` routes catalogue, exact process lifecycle, scoped grants and
`Forward {session_id, incarnation, command}`. A runtime request adds the host-private
runtime token and optional validated grant binding. Both layers return separate
`result`, `error` and `outcome_unknown` fields. Definite pre-admission rejection may
be durably retained; unknown delivery must not be converted into a fresh mutation.
SSE streams versioned, session/incarnation-bound invalidation batches for up to 256
subscriptions. They send keepalives, preserve runtime event cursors and terminate on
authorization or routing failure. Canonical text remains in snapshots/history/output;
stream reconnection never dispatches or retries a command. See
[process access](process-access.md) for authenticated local HTTP and scoped HTTPS.

`Events {after, limit, wait_ms}` returns metadata-only `public-v1` invalidations,
ordered cursors, `replay_gap`, `has_more` and `latest_cursor`. Limits are 1–128 events,
0–10,000 ms wait and 2,048 retained events. SQLite triggers commit invalidations
with canonical writes. Snapshot/history reads still require history authority;
an observe-only event does not disclose transcript text. The client advances its
cursor only with a verified response and refreshes a snapshot after a gap.

Snapshot is a bounded recent projection. `History` pages contain at most 128
messages. `MessageChunk` requires an exact revision; `RunOutput` requires an exact
run. Chunk limits are 1–65,536 bytes and offsets must be UTF-8 boundaries. Partial
output remains distinct from committed assistant messages. Retained transcripts
are not reconstructed by replaying provider requests.

Recover runs under the same startup and session OS fences without constructing an
executor. Unresolved run/session resources remain explicit obligations. Operator
attestation carries exact identifiers and is reported separately from observed
cleanup. Only a positive, exact-incarnation disposition enables explicit restart.
Owner transfer requires signed preparation, positive permanent source fencing and
verified checkpoint publication; no timeout or failed health probe grants takeover.

## Delivery resolution

`Resolve { command_id, original }` never dispatches `original`. A live runtime
excludes mutation dispatch while resolving; a suspended observer holds the same
exclusive execution fence as the owner. Existing receipts retain their meaning.
Absent admission is converted to a durable `not_admitted` tombstone and caller
reservation before responding. Later requests with that ID cannot be admitted,
including after restart. Payload and principal conflicts fail closed. Scoped
resolution requires the original command's right and matching identity; payload-free
legacy resolution requires local host authority. `Receipt` remains a read-only lookup.

Helm persists exact public mutation envelopes before transmission and resolves
uncertain ordinary submissions without automatic replay. Definitive non-admission
restores editable text, allowing an explicit new command. An unavailable status
check never becomes evidence that the original send was refused. Branch and stopped
archive restart remain separate Vessel lifecycle workflows. Private terminal input
and workflow secrets are excluded from persistent public command envelopes.

Long-lived event waits do not hold admission/dispatch locks or stall the listener.
Suspension and mutation dispatch are mutually exclusive. Accepted request handlers
are drained on retirement, and queued sockets receive explicit non-dispatch where
possible; unavoidable connection loss is handled by durable resolution.

## Tool validation and outcomes

Tool execution belongs to Voyage; Helm displays observations and Vessel routes
commands without broadening execution authority.

### Shell policy

Shell policy analyzes the original script with a bounded tree-sitter Bash grammar
and an explicit supported-node subset. It does not execute parsing probes or
rewrite source. Syntax recovery/missing nodes, unsupported constructs and exhausted
budgets refuse execution in every access mode. Limits are 1 MiB source, 100,000
visited nodes, 128 AST levels, eight nested shell-script analyses and 250 ms total
analysis time. Parser recognition is not a claim that all Bash syntax is supported
by the executing `sh`.

Quoted heredoc bodies are literal data: unmatched quotes and command-looking text
inside them do not invalidate the script or become executable commands. Supported
substitutions in unquoted bodies and commands after the heredoc are inspected.
Sequential heredoc commands, escaped delimiters and tab-stripping heredocs are
supported. Multiple heredoc redirects queued on one command are currently refused
by the grammar rather than partially analyzed.
Command-name deny-listing has been removed: neither executable names nor inert
arguments are blocked by a name list. Literal
nested shell `-c` scripts are inspected; dynamic executable names, dynamic command
forwarding, `eval`/`source`, shell script/stdin entry points and unsupported expansion
forms are refused with specific guidance. A valid shell script may therefore still
receive an explicit unsupported-syntax refusal. Use simpler explicit commands;
never reinterpret a refusal as authorization to run the same effect another way.

Only confidently read-only single commands avoid prompts in approval mode.
Compound scripts, heredocs, redirects and substitutions require approval there.
Unrestricted mode skips ordinary prompts, not syntax checks,
roots or administrator ceilings. Application policy cannot interpret arbitrary
Python/program semantics and is not an OS sandbox. The POSIX analyzer does not
authorize Windows `cmd.exe` PTY commands; that path explicitly refuses until a
separate dialect-aware implementation is verified. Linux evidence does not
establish native Windows/macOS behavior.

### Retired command-name deny lists

Command-name deny-list enforcement is removed from shell/PTY policy, internal Git
worktree helpers, clipboard helpers, administrator-ceiling intersection and child
or participant delegation. There is no default list and no `deny_commands` CLI,
UI or effective-policy setting. Read-only mode, approval mode, syntax inspection,
workspace authority, environment filtering, resource limits and cleanup remain.
This does not introduce Pi's entire execution model or disable every other guardrail.

Old `deny_commands` values in saved configuration are inert. Historical profile,
defaults and participant documents retain the serialized field for canonical-byte
checks, hashes and exact command/assignment identity; it is compatibility metadata,
not an execution restriction. New compatible documents carry empty lists and new
assignments send an empty list. Do not strip or rewrite the field in old journals.
Other unknown fields remain rejected in strict policy documents.

Existing named-profile/default references and confirmed transitions can become
stale when their effective policy or builtin snapshot changes. Reinspect/reselect
and explicitly reconfirm those settings using the supported policy controls;
old confirmation hashes are not silently accepted. Historical journals are not
rewritten and uncertain operations are not replayed. Older running voyages and
remote runtimes retain their old behavior until they run an updated executable;
an empty delegated list cannot disable an older host's own command policy.

### Action schemas

Vessel, process and subagent tool schemas declare the complete allowed key set
per action. Irrelevant fields are rejected, not ignored. For example, `create`
does not accept `name`, and `inspect` does not accept `limit`. Todo actions retain
separate create/status/evidence operations. Optional nullable fields follow the
runtime's actual argument types.

Vessel validates pure text/path/page bounds before transport or intent admission.
Nonblank text limits are **UTF-8 bytes**: query 4,096; task/prompt 65,536; rename 256.
JSON Schema character limits alone cannot enforce byte limits; runtime validation
remains authoritative. UUID formats also require runtime parsing.

Inspection returns a compact observed overview with bounded progress text and
explicit detail-read requests. Large cleanup/resource observations are marked
omitted rather than presented as empty. `details` pages public snapshot fields;
revision-bound `history` and `message` expand canonical conversation, while
`run_output` reads accumulated run text. Message/run reads incrementally decode and redact bounded chunks using private,
expiring cursors; continuations recheck authority and message revision. Run reads
pin the initially observed prefix. Source changes and missing cursors are explicit
incomplete observations. Regex expansion retains its 4 MiB per-message cap. History and regex-search pages preserve message indices and exact continuations
when reduced to fit. Regex matches redacted content; a truncated projection is
expanded before evaluation or explicitly reported unsearched. A zero-match page
with more records or any unsearched gaps is not a complete negative search.
No state read grants execution authority.
See [Vessel coordination](vessel-coordination.md) for offsets and continuations.

If even the overview's identity/state cannot fit, no partial overview is returned;
a one-field detail read is suggested. Detail reads that cannot fit a minimal page
report that the configured budget must increase. Limited mutation presentation
retains a receipt-read hint, never a replacement mutation. Full redacted mutation
results remain in the durable operation journal.

### Structured outcomes and compatibility

New tool messages carry optional `tool_outcome` alongside unchanged canonical text
and `tool_output`. Execution outcome, observed command exit/signal and output
incompleteness are independent. A shell that successfully observes exit 7 has a
successful *execution observation* and a failed *command*. Capture limits can
coexist with either exit result. Policy refusal is distinct from an execution
error; a timeout is unconfirmed, not proof of rollback or cleanup.

The compatibility success/error flag is false for nonzero exits, signals,
refusals, execution errors, cancellation, unknown outcomes and incomplete output.
Helm uses typed labels/counts first and conservative legacy text fallbacks. Raw
subprocess diagnostics are not injected into activity labels. A successful Vessel
call still does not mean independent work has completed.

Outcome metadata survives canonical message serialization and full/bounded public
history projection. Absent metadata means legacy/unclassified, not verified
complete. Managed journal schema 11 fences older writers before new outcomes are persisted,
using the existing owner-fenced content upgrade path from schemas 8/9/10.
Existing strict `ToolOutput` wire objects are unchanged; old public message readers
that ignore optional message fields can decode the added sibling, but older JSON export/import writers
may discard it. This is not a lossless JSON downgrade guarantee. No automatic retry or
uncertain-effect reconciliation policy is changed.

Malformed unified diffs, header-only input and non-diff text are refused before
target writes. Both file headers and at least one hunk are required. Diagnostics explain
headers, body prefixes, hunk counts and newline requirements without echoing file
contents or repairing patches. Stale hashes and valid-but-nonapplying patches
remain distinct failures. A corrected existing-file patch needs a fresh
`read_file` SHA-256.

See [configuration](configuration.md#execution-policy-and-limits) for policy and
[Git workflow](local-git.md) for wrapper-free repository access. Focused offline
checks accompanying these code changes do not recreate the removed broad suites
or certify live providers or native platforms.

## Deleted working directories

A voyage's conversation lives in its private runtime journal, separately from its
working directory. Deleting a completed voyage's working directory must not erase
its history or prevent delivery resolution.

On Linux, the next explicitly submitted message or runtime restart can recreate a missing directory
at its original path. This applies to an existing voyage with retained host
configuration and a positively fenced restart. The parent directory must still
exist at its canonical path and permit creation. Recovery does not recursively
create missing parents, follow a replacement parent symlink, overwrite an existing
path, relocate the voyage, or restore deleted files. Ordinary startup then resolves
the retained configuration against current execution policy. Session identity,
history, access settings and command deduplication remain in force.

The new directory is private (0700). A `.git` pointer deliberately refers to a
missing `.voyage-missing-checkout` directory. This makes Git fail instead of
discovering a repository in a parent directory. To use Git again, explicitly
restore the original checkout, or remove that placeholder `.git` file and
initialize the intended repository. Recovery does not guess a branch or commit,
and it cannot recover deleted uncommitted files. No Git recovery provenance is
currently retained for automatically reconstructing a removed worktree.

Helm and the executing model receive a notice that the directory was recreated
without its files. The model notice is runtime context, not a synthetic message
inserted into saved conversation history. The private recovery record identifies
the recreated directory's inode and Git placeholder; replacing the directory or explicitly
removing the placeholder ends the notice.

Saved reads and delivery resolution do not recreate directories or run an agent.
Resolution uses retained configuration for public-payload validation without
requiring execution-policy resolution against a live workspace. Existing caller,
grant, cleanup and execution fences still apply. An unadmitted command ID is closed
durably; a delayed copy cannot execute. After a failed startup, a new message may
retry once the filesystem problem is corrected. Uncertain work is never replayed.

Automatic recreation is implemented for Linux. Native macOS and Windows behavior
is not established. Missing parents, denied filesystem permissions and current
policy refusals can still require operator action; history and local delivery
resolution remain independent of directory recreation.

### Deleted-directory verification

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 voyage/tests/missing_workspace.py --bin-dir target/debug
```

The focused offline Linux fixture verifies preserved history, negative delivery
resolution without directory creation, a subsequent completed turn under the same
session and access mode, no duplicate inference, a runtime-only model notice,
blocked parent Git discovery, and continuation after a failed startup with a
missing parent. It uses synthetic credentials and a local provider, with bounded
waits and observed cleanup of fixture-owned processes. It does not establish live
provider or full-screen Helm behavior. Evidence stays under the printed `/tmp/vdr-*`
directory. This process check is separate from workspace Rust coverage.
