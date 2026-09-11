# Vessel notifications and owner handoff

Status: **integration draft for #72; executable verification and coverage are
pending the coordinated build slot**. Source and test definitions are not a
passing deployment claim. Browser/mobile and #257 condition subscriptions or
automatic voyage wake-up are not part of this service.

See [client commands](notifications-client.md), [architecture](architecture.md),
[process authorization](process-access.md) and [owner decision evidence](approval-semantics-verification.md).

## Ownership and consent

An independent Voyage commits a metadata-only intent in the same transaction as
its final classified run outcome or new durable decision. The final outcome is
used **after** cancellation and incomplete-work overrides. Completed, incomplete,
failed, cancelled and interrupted remain distinct. Every exported result says
nothing about effect/resource cleanup; cleanup remains with the canonical owner.
No model output, prompt, tool argument, approval parameters, provider state,
credential, pathname, title or private-terminal bytes enters a notification.

Vessel is a bounded metadata courier and recipient store, not an agent or second
canonical history writer. It reads the explicitly enabled source through its
existing authenticated private owner IPC. A producer cannot select a recipient,
write arbitrary content or invoke the public inbox API as a publisher. There is
no external webhook, URL opener, browser handoff or network credential forwarding.
The public `notifications` operation is carried on the same authenticated Vessel
HTTP/duplex command API as other operations.

Two distinct actions enable a destination:

1. The **source's local owner** configures an immutable source session/Vessel and
   recipient grant/principal/revision, event preferences, finite lifetime and TTL.
2. That **exact recipient** explicitly accepts subscription and bounded retention.

Pairing, catalogue membership, an existing Observe right or the source owner's
configuration alone does not accept recipient delivery. Changing any destination
field requires a new ID and acceptance; a revoked ID never becomes active again.
A local recipient is represented by the actual Vessel UUID for both recipient
IDs and revision 1, and is accessible only using local account authority.

Recipients need current source Observe authority. Budget-enabled destinations
also require History (the accounting API's current disclosure boundary). Opening
an attention reference additionally requires Decide; neither Observe, History,
Execute, local source configuration nor coordination substitutes for it. The
source session must still belong to the same Vessel and workspace scope; departed
ownership and participant-private registrations are refused. Human workspace
connections are checked as connections, not treated as their derived session
credentials. Participant/parent-derived grants are not human inbox identities.

Grant/principal/revision/source checks occur at configuration, publication,
observation and opening, including after an asynchronous owner request. The
existing registration lock orders local grant revocation with inbox admission.
Destination expiry cannot exceed its recipient grant expiry. The service samples
its clock after obtaining SQLite's write lock, rather than using a pre-wait expiry
sample. This is a local admission boundary, not an atomic distributed
revocation/effect transaction; already disclosed bytes cannot be recalled.

## Durable identities and bounds

The owner journal reserves a **256-slot × 1,024-byte** metadata ring on the
owner-fenced schema-12 upgrade. A full ring replaces the oldest intent without
blocking terminal settlement. Its monotonically increasing durable sequence
exposes a gap to readers behind retention; evicted events are not reconstructed
or advertised as delivered. Missing original incarnation or unavailable source
clock likewise produces a gap rather than fabricated attribution. Recovery uses
the original run incarnation, never the recovery process's identity.

Recipient storage is separate, SQLite-backed, in the private Vessel notification
directory. Source-event IDs and payload fingerprints are retained after payload
expiry so an old ID cannot be admitted again. Exact publication retries return
original receipts; changed reuse conflicts. Publication commits before cursor
advance; a crash between them rereads the same source page and recovers the same
receipt, not source execution. No automatic retry sends a run, approval decision,
cancellation or other execution command.

| Bound | Value |
| --- | --- |
| Destination lifetime | At most 30 days, also no later than recipient grant expiry |
| Notification TTL | Positive, at most 7 days; attention capped by owner decision expiry |
| Destinations | 256 lifetime records; at most 32 per recipient grant and per source |
| Recipient event receipts | 16,384 lifetime total; at most 1,024 per destination |
| Encoded destination/event record | 1 KiB |
| Inbox page | 1–100 entries |
| Owner outbox read | 1–128; courier uses 64 |
| Budget read | 1–100; courier uses 64 |
| SQLite main-file ceiling | 64 MiB; logical admission quotas reserve headroom for settlement |
| SQLite busy wait | 2 seconds |
| Courier batch | Up to 8 destinations, then a 1-second pause; separate source/budget cursors |
| Owner read deadline | 5 seconds per source stream |
| Failure retry schedule | Persisted attempts, at most 5; 2/4/8/16/32-second earliest retry times |
| CLI watch | 1–3,600 seconds; at most one poll per second; stops on errors/Ctrl+C |
| Passive TUI attention probe | At most once per 15 seconds per connected route, 2-second deadline |

These are resource bounds, not a delivery-latency guarantee. Offline sources,
queue pressure and owner deadlines can make attention expire before delivery.
After five failed reads/publications, the retained attempt remains stopped; it
does not busy-loop or silently renew authority. The current workflow requires a
new explicit destination to try a new delivery lifetime. Source/recipient
operations remain available independently of a full publication quota.

Capacity refusal, unavailable sources and retained gaps are not successful
notification delivery. Inbox producer state reports incomplete observation;
empty pages never prove completed work. A drained, positively stopped owner
incarnation is not restarted repeatedly just to check an unchanged outbox; a new
incarnation is eligible for new observation. Recipient disconnect never cancels
the source run. Vessel restart restores metadata cursors and bounded attempts,
not a claim that a dead process or external effect survived.

The lifetime quotas intentionally do not recycle receipt identity. No deletion or
reset command retires the deduplication evidence. An exhausted installation needs
an explicit future generation/retirement design before identities can safely be
reused; deleting queue rows is not supported recovery. Payload expiry/revocation
clears live payload columns with secure-delete enabled. Filesystem snapshots,
backups and already delivered copies remain outside recall guarantees. Neither
notification retention nor receipts delete source cleanup/approval obligations.

## Preferences, clocks and receipts

A destination selects an exact voyage and nonempty event-kind subset. Projects
are optional: budget events may refer to the source session's project through an
opaque project UUID, not a project name/path or implicit whole-project subscription.
The source's accounting API remains the authority for budget provenance.

Quiet hours are **UTC minutes since midnight**, start inclusive and end exclusive,
with explicit midnight wrapping. Equal endpoints are invalid. There is no local
time-zone or daylight-saving interpretation: UTC means DST cannot lengthen a
window. Quiet hours suppress passive attention notices, not explicit inbox reads,
source settlement, expiry, default-deny or decision deadlines. A durable clock
high-water fences rollback from resurrecting an expired payload. Unavailable or
out-of-range clocks refuse publication/observation; revocation and receipt
settlement retain their bounded independent paths.

Each destination has independent Available → Seen → Dismissed state. Stale
interfaces cannot reverse a later receipt. Listing/opening does not implicitly
mark Seen. A receipt means only that authenticated metadata observation was
recorded, not that a human read it, completed work, approved an action or observed
cleanup. Multiple interfaces for the same destination see the same monotonic
receipt; another recipient's destination is not acknowledged for them.

## Approval separation

Open retrieves only the current authorized owner reference. For an attention
reference it uses the recipient's own Decide authority, filters the owner's
current requests by the exact decision/run/incarnation, then rechecks current
scope and lifetime. Missing, resolved, expired, revoked, stale or unavailable
references return non-actionable status without private content. There are no
bearer decision links.

Even a current response has `actionable: false` and `approval_granted: false`:
opening is not authorization consumption. The operator must separately select
and review the owner request and use the existing explicit Respond operation.
That operation retains its exact request identity, principal binding, deadline,
single-response semantics and uncertainty receipts. A changed request requires
new review. Questions remain a distinct owner workflow, not security approval.
No notification failure, silence, quiet hour, dismissal, replay or reconnect
extends an approval deadline or triggers a tool.

## Budget provenance and compatibility

The courier reads #71's `BudgetEvents` owner operation and maps only immutable
source-event UUID, ledger sequence, session/run, scope UUID, revision, dimension,
warning/denied threshold and source time. It never infers warnings from totals.
Monetary estimates are not invoices; numerical amounts and labels are not copied.
Absent original incarnation is explicitly null, not filled with the current
observer's identity. An older owner without that API is unavailable for budget
delivery; no guessed or model-generated fallback is used.

## Verification plan and current limits

Focused Rust test drafts cover strict protocol fields, UTC boundaries, independent
consent, identity/dedup conflicts, restart, monotonic receipts, retention/clock
fencing, capacity, private-file refusal and atomic owner transaction rollback.
`voyage/tests/notifications.py` drafts an offline real Voyage/Vessel/independent
Helm CLI/TUI journey using existing controlled-loopback fixtures: actual completion,
wrong recipient, duplicate/conflicting commands, receipt race, restart/reconnect,
revocation/stale restoration, passive narrow/resized inbox and preserved Unicode
composer, plus exact decision opening/dismissal with zero tool effects before a
separate explicit owner denial. These checks have **not run yet**.

Required next evidence: compile/tests on final merged source, adversarial fixture
execution and fixes, workspace coverage with current-artifact selection, committed
measurement, normal main integration/publication and issue results. A Linux
synthetic endpoint is not installed HTTPS, live-provider, real-user-message or
native macOS/Windows certification. No such external activity is authorized by
this implementation task.
