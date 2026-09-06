# Voyage runtime contract

Status: component invariants for the [architecture](architecture.md). The Linux
process implementation follows these contracts; [current state](current-state.md)
and the [delivery ledger](implementation.md) distinguish source from observed
verification and unsupported platforms.

## Ownership and admission

A Vessel supervises one independent voyage process for each executing session.
The voyage process holds an exclusive session execution fence through idle time,
run callbacks and owned cleanup. All clients, including plain/one-shot
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
| Runtime | Live authenticated incarnation, stopped, unavailable, ownership conflict |

An unavailable connection cannot establish a stopped runtime. A final answer cannot
establish resource cleanup. Operator attestation must remain distinct from observed
cleanup and retain its provenance.

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

Protocol version 1 uses length-prefixed JSON with a maximum 4 MiB frame on private
Unix sockets. Vessel accepts at most 64 concurrent local connections and 1–256
configured process slots (default 16). Request tokens and process-registration
files are private account data; public health alone never proves authority. An
unsupported protocol version is refused before dispatch.

`VesselRequest` routes catalogue, exact process lifecycle, scoped grants and
`Forward {session_id, incarnation, command}`. A runtime request adds the host-private
runtime token and optional validated grant binding. Both layers return separate
`result`, `error` and `outcome_unknown` fields. Definite pre-admission rejection may
be durably retained; unknown delivery must not be converted into a fresh mutation.
See [process access](process-access.md) for authenticated HTTP and SSH routing.

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
