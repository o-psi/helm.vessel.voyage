# Local managed catalogue and cancellation

These Journal catalogue and cancellation APIs support the local managed executor tracked
in [issue 78](https://github.com/o-psi/voyage/issues/78). They expose no remote route,
provider execution entrypoint, or sharing grant. Installation and principal UUIDs
are attribution. A caller must establish local installation authority before using
these APIs; accepting these identifiers from a remote request is insufficient.

`Journal::create_session` is explicit UUID, create-only storage. A collision is an
error. The local frontend must not automatically generate another UUID and retry
an uncertain create. The caller can inspect the bounded catalogue to recover a
lost response. Dedicated remote sessions have separate exact-run cancellation
receipts; general remote session creation remains planned. These local catalogue
entries are not a voyage-wide inventory of participant Helms.

`list_session_summaries(after, limit)` returns UUID-ordered metadata pages, with
limits 1–100 and an exclusive UUID continuation. Each page has a consistent SQLite
read snapshot; concurrent insertions can appear on subsequent pages. Returned
metadata is session UUID, authoritative revision, optional name, model, workspace,
and active run UUID/state. SQL projects these fields without decoding or returning
conversation or provider history. Metadata per session is bounded to 64 KiB before
SQLite returns it to Rust. Names and paths are untrusted display data; frontends
must escape terminal control sequences and must not infer execution authority from
a displayed workspace.

`request_cancel_local(&LocalCancelRequest)` binds the exact session/run and
canonical installation/principal. It does not acquire execution ownership. The
first intent commits under `BEGIN IMMEDIATE`, sampling the production clock after
transaction acquisition and requiring an unexpired deadline within five minutes.
An existing identical run/actor intent is immutable observation: a changed or
expired retry deadline neither refreshes the record nor samples the clock. Once
committed, cancellation intent does not expire. Wrong scope fails before observing
an outcome. A terminal run returns its actual immutable terminal state.

`Requested { duplicate }` means the request is durable, not that execution stopped.
The owning coordinator polls `local_cancel_requested(session_id, run_id)` through
its existing Journal connection, cancels its token, and awaits provider/tool/child
cleanup while retaining its execution guard. Polling does not bootstrap storage or
read partial provider text. SQLite Busy remains a typed error; bounded retries
must not claim a receipt when no transaction committed.

The Journal checks intent in the transactions that start a run, accept a canonical
checkpoint, and terminalize it. Committed intent prevents start and acceptance.
If it precedes terminalization, stale success, incomplete, or failure outcomes
become Cancelled, with no new successful final text and no stale successful
classification. Existing canonical text and usage remain intact, and pending
steering is settled by the existing terminal transaction. The runtime must inspect
the returned actual `RunRecord` instead of reporting a stale in-memory success.
Terminalization first makes later cancel an observation without revision change.
Recovery remains Interrupted and never replays effects, even with pending intent.
The mailbox targets a run UUID; it cannot cancel a later run in the same session.

Schema 5 adds an immutable cancellation table without changing canonical session
or steering records. Schema 2–4 remain readable under their existing contracts;
new cancellation requires an explicit quiescent upgrade. Upgrade holds the SQLite
writer transaction and every session execution fence, refuses active runs, and
invalidates older open connections. Schema 4 steering remains active before the
upgrade. Stop old processes before upgrading; old binaries do not understand 5.
Rollback requires retaining the new binary to finish/recover runs and inspecting
state rather than silently downgrading or deleting cancellation evidence.

Backend tests cover metadata projection, pagination, malformed bindings, exact
actor/run targeting, deadline/clock errors, immutable retries, SQLite Busy and
failed writes, both terminal/cancel orderings, accepted-checkpoint cancellation,
reopen recovery, quiescent schema migration, stale connections, and an independent
process committing intent while the execution owner retains its fence. Full local
CLI/provider cleanup integration is implemented by [private managed sessions](local-managed-sessions.md)
and covered separately in `tests/system/local_managed.py`.

## Durable cleanup obligations

The foreground coordinator opts in with
`register_local_cleanup(&guard, run_id)` while the run is Accepted, before
constructing execution resources. Registration is transactional and idempotent
only for the same pending obligation. A failed or aborted registration cannot
permit dispatch. Once registered, the obligation remains after terminalization,
recovery, timeout, cancellation, or process death. New admissions for that session
fail while it remains outstanding; exact command retries remain observations.
The catalogue exposes `pending_cleanup_run`, including when no run is active, so
lost output cannot hide a blocked session.

After terminalization and actual resource cleanup, the owner records
`confirm_local_cleanup_observed(&guard, run_id)`. This stores the owner's observed
cleanup claim; the database cannot inspect operating-system effects. Confirmation
before terminalization is refused. Failed or uncertain cleanup must not call this
method. The owner retains the execution guard through cleanup and confirmation.

Recovery only marks Interrupted; it never clears an obligation. A human who has
independently stopped prior effects can explicitly call
`attest_local_cleanup(&guard, run_id, installation_id, principal_id)` through the
local coordinator. This verifies the canonical actor binding and stores
`operator_attested`, distinctly from `observed`. Neither provenance can later be
relabeled, and evidence rows are retained rather than deleted. The execution fence
is required for both acknowledgement paths. These APIs are local trust boundaries,
not mechanisms to prove that arbitrary descendants are dead or to expand policy.
