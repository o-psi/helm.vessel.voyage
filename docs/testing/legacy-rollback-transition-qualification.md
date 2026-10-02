# Ordinary legacy activation failure and exact rollback

Scope: issue401. The actual CT124 current-installer `install.sh upgrade` route
reached **inline local-owner legacy handover**, not a remote worker. Its monitored
candidate exits led to `Service is not active`, then rollback refused the generic
stable inspection gate with `Service is transitioning`. Operation
`1e63b5a2-720f-4c97-8d63-0f598566afb0` remained unconfirmed, candidate/schema2,
quarantine, snapshot and releases retained. Five exact owned candidate signal/reap
witnesses were preserved; this was not a successful old-reader restoration.
Do not retry/reset/reconcile that fixture or edit its receipt as part of this fix.

## Narrow rollback boundaries

General service `plan()` and normal quiescence remain strict about transitions.
Only a failed **already approved, published ordinary legacy candidate** uses the
new stop path. Before effects, the one attempt captures the original service's
account/credential namespace, complete manager and unit environment hashes and
credential-only override path/device/inode/owner/mode/content hashes. No credential
bytes are read/copied. The whole candidate declaration is pinned independently of
its release ID. These private in-memory pins are held through the same attempt;
existing receipt formats are not rewritten or retroactively supplemented.

The failed-candidate stop requires the exact canonical target unit/state and
unchanged enablement, environment and overrides. Fresh full manifest/file checks
must match the approved candidate. Any positive PID requires actual owner,
qualified executable path/device/inode/bytes, stable start identity, exact argv
and relevant HOME/data/key namespace. Mutable CPU/scheduling `/proc/stat` counters
are not identity. Growing/replaced images, changed declarations/context or unknown
states refuse before stop. Startup identity ENOENT/EACCES can remain bounded pending
for five seconds; it never authorizes stopping an unverified positive PID.

A matching owned unit also owns its queued auto-restart/start job. One stop cancels
that launch, without direct PID signalling, restarting or changing enablement.
Every stop observation rechecks the same context, and completion requires
inactive/failed, MainPID0 and authoritative no-job encoding empty or0 within
fifteen seconds. A pending job is not completion. Unknown/changed context or
unobserved cleanup retains quarantine, snapshot and uncertainty.

## Held restored snapshot before old pointer selection

After matched candidate quiescence, the existing helper reacquires supervisor
ownership and restores only the exact snapshot. Previously, generic pointer rollback
then invoked plain legacy eligibility while the caller still held guardian and
execution leases: that observer cannot borrow the held leases. Dropping ownership
to make it pass would introduce a competing-writer gap.

The new crate-private pointer entry instead accepts the actual typed legacy Guard,
exact current candidate and exact previous release. It rejects pending/changed
installation state, foreign namespace, forward-only guard or an unverified old
reader. The helper's existing `verify-restored` action must prove schema1, exact
restoration marker, backup hash, complete private/canonical state and every inherited
held lease before selecting v1.0.2. Generic unguarded rollback admission is unchanged.
The leases stay held through pointer publication and verified original identity;
only then are they released to restart the original service. Environment/override
pins are checked before and after original activation, before quarantine removal.
No old reader is selected over schema2 and no original apply is replayed.

## Prepared verification and remaining native gate

The source-only regressions cover13 private service-context families and two
real SQLite/held-pointer families (multiple refusal cases): matched PID0/live
transitions, both native no-job renderings, pending jobs, one-stop observation,
strict generic inspection, stable start despite changing counters, unit/env/key/
namespace/manifest/executable/argv refusals, equal-byte different inode, bounded
image size, stop refusal, context mutation after stop, original source capture,
ordinary eligibility still failing while leases are held, exact guard-backed
pointer, wrong identities, snapshot mutation, foreign state and forward-only proof.
Service/PID observations use the existing private fixture seam and do not claim
native process/service results. The SQLite, file locks, restored marker and pointer
publications are actual private fixture operations when executed.

Preparation performed Rust formatting, diff/path checks and embedded Python syntax
checks only. Compilation/tests are **not yet run**. The coordinated local gate must
run `failed_candidate::tests` and `legacy::tests`, strict Clippy and full workspace
coverage after the final relevant edit; publish its compact summary and follow an
actual hosted archive. Then use a **fresh** preserved baseline such as CT128 with
that qualified archive for the same exact candidate-start failure/rollback case,
and independently prove original image/unit/schema1/histories/account namespace,
receipt and observed cleanup. CT124 remains retained unconfirmed evidence.
