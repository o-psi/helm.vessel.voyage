# Notification delivery verification (#72)

## Scope and status

The notification/inbox/owner-reference slice has passed focused Rust and offline
Linux real-component journeys. **This is not closure of all retained #72 scope.**
The #71 accounting backend is not yet in the measured main base; its notification
adapter has typed-envelope coverage, not a genuine budget-transition delivery
journey. Pending-outbox process-crash barriers beyond the journal transaction tests
also remain unverified. No human acceptance gate substitutes for those checks.

See [service contract and limits](notifications.md), [client workflow](notifications-client.md)
and [existing approval evidence](approval-semantics-verification.md). Receipt,
opening, dismissal and recipient acceptance never dispatch or approve an action.

The implementation is based on published main `b990ecd` (#64/#10 and Android
changes), merged into `26e0e00`, plus the notification changes. Exact final measured
source, tool versions, fingerprint and workspace coverage are recorded separately
in [coverage/latest.json](../coverage/latest.json) when measurement completes.
Native Python/PTTY checks are not part of Rust coverage percentages.

## Actual commands and outcomes

Using the existing shared target under the coordinator-admitted build lease:

- `cargo check -p helm -p vessel -p voyage --locked -j 8`: passed. An unused local
  variable was corrected; the subsequent check passed, followed by warning cleanup.
- `cargo test -p voyage-protocol -p vessel -p voyage -p helm --locked -j 8 notification -- --nocapture`:
  31 passed, no failures. This includes five pre-existing Helm settlement tests,
  13 Vessel notification tests, nine owner outbox tests and four protocol tests.
- After the storage-admission authority fix, `cargo test -p vessel --locked -j 8 notification -- --nocapture`:
  14 passed, no failures, including the new blocked-SQLite/current-revocation test.
- `cargo test -p helm --locked -j 8 inbox -- --nocapture`: eight passed, no failures.
- `cargo build -p helm -p vessel -p voyage --locked -j 8`: passed.
- `python3 voyage/tests/notifications.py --bin-dir TARGET/debug`: final attempt
  passed all five journey groups and observed cleanup.
- `python3 voyage/tests/notification_adversarial.py --bin-dir TARGET/debug`: final
  attempt passed all eight adverse groups and observed cleanup.

The native fixtures used these development binaries (SHA-256):

| Binary | SHA-256 |
| --- | --- |
| Helm | `41e04f0ad397eea1bbc07c194643f624cfad076e722e18f47dd7a974904cfe56` |
| Vessel | `e198a642637738c09dd7fe4aaa94b43c6e1a6e1fb835d14ae37e328a75ece43d` |
| Voyage | `95cb297849e056429a218e83fcdfe77377435b9984f70659292dc00fe251714e` |

Later Rust test-only additions do not claim a new native binary execution.

## Verified behavior

The primary journey exercised separate source configuration and recipient Accept,
wrong-recipient refusals, exact configure/test retries and changed-payload conflicts,
monotonic seen/dismiss races, synthetic test labeling, actual owner completion
references, and an independent authenticated Helm CLI and TUI. Its passive inbox
survived 120×32 → 40×18 → 120×32 presentation changes, preserved a Unicode composer
draft, restored the alternate screen and made no inference call. The fixture waits
for actual catalogue arrival and selects the canonical displayed voyage title.

Vessel restart and recipient reconnect retained an already accepted notification
without another provider call. Destination and grant revocation prevented payload
observation; restoring the revoked destination ID was refused. An independent
viewer could receive bounded attention metadata but could not open an action
preview without Decide. The exact authorized decision/run/incarnation was exposed
only to the approver. Opening twice and dismissing left the owner request pending
and the requested file absent. A separate explicit owner denial then settled the
request with its exact retry receipt; no file effect occurred.

The adversarial journey covered foreign grant/principal/session/Vessel/revision,
missing source, destination lifetime exceeding grant expiry, Observe without the
accounting History right, rejection of five free-form payload fields, and absence
of synthetic transcript canaries from real terminal-event metadata. It also checked
destination/grant revocation and expiry, and that recipient Accept grants neither
Decide nor an approval response.

A held SQLite writer lock crossed notification destination expiry within the
unchanged two-second store busy limit. The released reader returned a successful
empty page, not stale content or a timeout relabeled as safety. A separate fixture
atomically revoked only its synthetic executing-host grant record while an HTTP
reader waited behind SQLite; the reader disclosed no payload. This HTTP fixture
cannot distinguish waiting during store opening from waiting at the later
post-routing transaction. Its evidence explicitly retains that limitation.

The focused Rust test supplies a store with previously valid bound authority,
holds a second connection's write lock, revokes the synthetic host grant before
releasing it, and asserts the reader refuses. The production store now rereads
current authority inside `BEGIN IMMEDIATE` after lock acquisition, alongside its
post-lock clock sample. This is a local admission sample, not a distributed atomic
revocation/effect transaction.

Owner tests cover final cancellation/incomplete overrides, journal transaction
rollback on injected intent failure, stable IDs across reread/reopen, bounded-ring
gaps, original incarnation on interrupted recovery and legacy schema fencing.
Store tests cover immutable consent/dedup, exact synthetic retries across source
incarnation changes, independent receipts, retained fingerprints after expiry,
capacity behavior, UTC quiet hours, rollback fencing and unsafe private-file refusal.

## Preserved failed attempts

The first full workspace coverage run failed the existing canonical-journal legacy
fixture: it changed a freshly initialized schema-12 database's version to 10 while
retaining the new notification tables. The runtime correctly rejected duplicate
future-layout tables. The fixture now removes those two tables when constructing
synthetic v10 storage; the runtime's migration refusal was not weakened. That
failed run and profiles are retained separately from the subsequent measurement.


The first native attempt opened F2 before catalogue arrival and sent the inbox
command into an empty picker. Two following selector attempts expected a generated
`session-*` name instead of Helm's first-message display title. Those fixture
failures, terminal traces and observed cleanup remain retained. The fix waits for
the actual source listing and selects the known canonical displayed title; no
runtime timeout was increased.

The first adversarial attempt passed authority/metadata and lock-expiry checks,
then its nonblocking **test setup** failed to acquire SQLite while the normal
courier briefly held it. The fixture now arranges that writer barrier with a
bounded one-second acquisition loop before timing the experiment. Runtime busy
limits and the held-lock experiment deadline are unchanged. Original primary and
cleanup exceptions are retained separately rather than overwritten by `finally`.

Detailed logs, synthetic databases, terminal traces and reports remain in private
local/ignored evidence, not this repository. No test failure was erased or
reclassified as a pass merely because a later attempt succeeded.

## Limits and remaining work

- #71 actual budget-source transition delivery remains pending integration and
  verification. The adapter recognizes both strict and estimated token labels and
  unsettled-attempt labels without interpreting amounts or approving limits.
- Process-level fault injection at every producer commit/send, Vessel acceptance
  and recipient acknowledgement boundary remains broader than the executed
  restart and transaction-rollback checks. No full crash-matrix certification.
- Full private-terminal/modal concurrent-input matrix and installed HTTPS
  deployment remain separate evidence; this fixture checks a passive inbox,
  Unicode draft and narrow/resize behavior, not every native terminal journey.
- Linux loopback/synthetic-provider evidence is not real-user messaging, live
  provider spend, native macOS/Windows or production TLS certification. None of
  those activities was performed implicitly.
- Receipt retention does not recycle identity or erase owner cleanup obligations.
  Exhausted lifetime quotas require a future explicit generation-retirement
  design, not manual queue deletion or an automatic replay.
