# Durable managed steering foundation

This is the local receipt backend for #78. Stable local actor identity and explicit
managed CLI execution are implemented. Managed TUI projection, the remote steering
frontend, and complete guarded frontend lifecycle operations remain planned. This
module does not itself enable network steering or automatic session transfer.

Planned [cross-Helm steering](voyages.md) must route messages from an authorized interface
Helm to the relevant coordinator or participant, preserving current authorization
and honest delivery receipts. The interface Helm need not coordinate the voyage.
These local receipts do not establish that cross-machine route or its handoff semantics.

Schema 4 stores steering receipts separately from canonical session messages.
The receipt UUID is also the request's idempotency key; it cannot collide with any
turn-command UUID, including commands in another session. Historical receipt IDs
in imported/copied canonical transcripts are also reserved against new admissions;
copies preserve their original IDs without fabricating steering rows. New imports
and session creation reject historical IDs that collide with existing commands. The request binds text,
session, run, submitting machine/principal, expected revision and deadline. Exact
retries return the durable outcome without sending again, even after the original
deadline; changed bindings conflict and current caller authorization still applies.
Production handles sample a trusted system clock after acquiring the storage
transaction and again before applying pending input; callers cannot supply a
stale timestamp. A new deadline must be within five minutes. Newly expired receipts cannot reach provider dispatch.
Pending text is not inserted into the canonical transcript or provider context.

A managed steering handle commits Queued before sending to its bounded channel.
The session owner mutex covers queue commit, send and any durable rejection after
Full/Closed. A failed rejection write poisons the turn: further provider dispatch,
partial publication and completion acceptance fail closed while ownership remains
held. A failed queue write sends nothing. A dropped caller future does not stop
an already-running storage worker or release that worker's ownership token.

At a model boundary the canonical checkpoint validates exact known user receipts,
FIFO order and unchanged prior history. It commits the appended messages, Applied
receipts, usage/revision and replay events in one transaction. Applied means the
input is durably included at a model boundary, not proof that a remote provider
consumed it. Unknown IDs, altered text/roles/provider metadata, duplicate receipts,
reordered pending receipts and already rejected receipts cannot enter history.
Pending receipts prevent final acceptance. Cancel, failure or explicit interrupted
recovery settles remaining Queued receipts as NotApplied without re-enqueuing them.
Historical Applied receipts and canonical text are not rewritten on restart.

There is no default steering authorization grant. The managed handle requires a
trusted current-authorization callback. It checks the submitting actor at queue
admission and checks current actor authority again at canonical/dispatch and final
acceptance boundaries. Callbacks must be bounded and must not reenter the owner;
frontends and transports remain responsible for authenticating the caller and
freshly authorizing every other action. Stored receipt identities are evidence,
not reusable positive authorization. Managed execution rejects externally filled
raw steering receivers; standalone Agent channels remain a separate low-level API.

Limits are UTF-8 bytes: each input has the existing Agent limit of 64 KiB; a run
can retain at most 64 pending receipts and 1,024 total receipts. The journal retains
at most 100,000 steering receipts globally, independently of replay eviction, and
its existing 256 MiB database and 16 MiB canonical snapshot bounds still apply.
Receipt projection pages contain at most 64 records. Encoded receipt rows are
bounded before parsing (six times the text byte limit plus 4 KiB for metadata). Exhaustion fails without
dispatch or discarding deduplication evidence.

Schema 2/3 databases require explicit quiescent upgrade before steering. Existing
import provenance, canonical messages, runs and replay are preserved; old schema
writers are fenced after upgrade. A legacy command/historical-receipt collision
blocks upgrade without changing the prior schema or canonical history. Native Windows journal storage and process
fences remain in force. Source JSON transfer stays unsupported outside Unix.
