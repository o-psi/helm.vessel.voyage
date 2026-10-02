# Cold browser runtime and capacity preparation (#353)

This separate source-only cohort extends the corrected #353 production address
map. Its input `db3d358` inventory has 135 zero-count addresses in
`voyage/src/host_browser.rs` and 127 in `host_browser_capacity.rs`. Most existing
worker contract tests seed an already owned worker and bypass cold allocation;
their privacy/control/receipt assertions and the separate 24-case Helm
Handle/App cohort are preserved. No coverage gain is claimed before the next
coordinated measurement, and the full denominator stays intact.

## Recorded production defects

Both defects were recorded before edits:

- [Missing guardian allocation leak](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5946752499): `Worker::spawn` retained a private profile
  directory before checking `guardian.py`. A missing guardian left no child or
  cleanup owner. The shared private `spawn_in` path now validates the guardian
  first; production still calls it with literal `/tmp`. The regression invokes
  the real failure with a private fixture profile parent and observes that it
  remains empty. There is no new configuration or public execution API.
- [Incorrect sibling recovery catalogue](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5946823937): the production server supplies `SESSION/journal`,
  while `acquire_for_session` requires `SESSION` and derives its sibling
  catalogue from the parent. Cold startup now requires the `journal` leaf and
  valid session/catalogue parents, then supplies the current session directory.
  It refuses an unexpected layout before removing prior guardian evidence.
  Existing worker locks, registration/UID/private-file/PID checks, session
  fences and complete guardian-evidence requirements remain unchanged.

The sole production caller remains `server.rs`, which passes
`directory.join("journal")`. Seeded worker and unavailable/prior-lock test
paths keep their earlier refusal behavior. No broader parent lookup, system
identity scope or deferred v1.1 work is introduced.

## Prepared assertions

`host_browser/cold_lifecycle_tests.rs` prepares 11 scenarios: missing guardian
without a retained profile; invalid distribution/profile-parent refusal;
actual cold Start/init/open/reservation/receipt/close; one fallback status read;
pre-spawn reservation release while preserving the original unknown receipt;
blocked resource-store refusal; init/open/malformed-reply failures that retain
cleanup obligations and refuse restart; unexpected journal layout; unresolved
cleanup evidence; actual sibling-slot cold recovery; and busy/unverified sibling
refusal without deleting foreign slots.

`host_browser/capacity_recovery_tests.rs` prepares 10 scenarios: exact owned
slots/audit/retained-lock recovery; explicit intent/observation/reason guards;
registration/stopped-state matrices; actual guardian/startup lock refusal;
invalid PID and the fixture's live PID; private-file link/mode/size refusals;
foreign/unsafe capacity; symlink session refusal; all complete guardian fields;
and actual execution-lock exclusion before recycling one observed slot.

Every cold/global-root case runs the fixed exact child test binary with private
HOME/XDG roots, cleared environment, ordinary UID and explicit parent/process
group checks. The parent environment and production hosts are untouched. Child
stdout/stderr are private bounded files, retained on failure; a finite deadline
stops/reaps only the owned test child. The existing LLVM profile location is
preserved for the coordinated measurement. No independent Cargo process runs.

The Python pipe endpoint is a fixed fixture. Production `Worker::spawn`, framed
exchange, actual owned Python PID/start/group, local profile cleanup and
reservation code run against it. It never launches Chromium or descendants.
Its `fixture_only` cleanup reports and the capacity registration/guardian
records are synthetic contract inputs. They prove neither an independent
native subreaper nor native browser/descendant cleanup. A deliberately false
cleanup report retains resource obligations; no successful release or external
effect reconciliation is manufactured. A dead PID alone never admits recovery.

## Gate and remaining scope

Source formatting, Python fixture AST and diff checks precede independent review.
No test/build/Cargo/native-host/provider/credential operation has been executed
for this cohort. The final source gate and full workspace coverage are pending;
that separate gate must include both production fixes and these final scenarios.
The prior ready Helm/saved-authority/history/composer cohort can finish on its
own unchanged source.

Packaged discovery and cgroup configurations not exercised by these fixtures
remain gaps to reassess after measurement, not exclusions or unreachable
classifications. Real #333 public WSS/TLS/sites/media/cost qualification and
its legitimate Native credential remain separate. macOS/Windows and parked
Root/system/cross-UID qualifications remain v1.1 obligations.
