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
Vessel resumes only that state automatically, with a new incarnation; unavailable
or explicitly stopped owners still require explicit recovery/restart. A command
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
target. Switching views preserves drafts and unread state. Notifications carry
minimal metadata and require fresh authorization when opened. When no eligible
interface is available, the runtime follows bounded pending/refusal behavior under
local policy; it never waits forever or treats silence as approval.

PTY attachment is a separate exact-target channel owned by the voyage. Human input
is private terminal data, not model-visible chat or replay history. Detach before
switching its target. Sanitize untrusted terminal text in all ordinary UI paths;
only the controlled terminal-screen renderer interprets terminal output. Keep
provider/subprocess diagnostics out of the TUI's display stream.

## Completion, cancellation and recovery

Root terminals close before turn suspension; they have no implied lifetime between
turns. Terminal metadata and canonical history remain durable.

Register subagents, managed commands, terminals and completion obligations before
they become independently active. Finishing a model turn is not proof that all
owned work finished. Completion must reconcile the run's obligations with bounded
waiting and actual evidence, retaining incomplete and unresolved outcomes.

Cancellation names the session and exact run. Persist cancellation intent, close
new work admission and cancel owned work. Keep the owner fence until cleanup is
observed or explicitly reported unconfirmed. A late model result cannot overwrite
a committed cancellation. Cancel one voyage without stopping unrelated voyages.
Helm detach is never a cancellation request.

After process failure, recover exclusive ownership before admitting another run.
Mark abandoned execution interrupted; preserve partial output, unresolved effects
and pending cleanup. Never replay an uncertain tool effect automatically. A stored
PID, heartbeat timeout or terminal metadata is insufficient proof that descendants
are gone. Recovery may require explicit, attributable operator confirmation.

Vessel restart must reconcile runtime registrations against live authenticated
incarnations. It cannot steal a session fence or clear cleanup blockers. Runtime
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
[process access](process-access.md) for authenticated local HTTP, scoped HTTPS and
the SSH compatibility adapter.

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

Outbound enrollment-relay commands retain their separate remote-owner receipt
and transport-lease authority; local `Resolve` refuses remote-bound sessions.
