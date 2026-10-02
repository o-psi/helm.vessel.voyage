# Prepared ordinary forward transaction coverage (#353)

This is **source preparation**, not a passing test or new coverage record. The
cohort is isolated on `coverage353-forward-owned`, based on qualified79507fe.
No Cargo command, test, build, native update, provider call or shared target/profile
mutation was performed during preparation. Native CT106 recovery evidence remains
separate in [the qualified runbook](ordinary-forward-recovery.md).

## Selection and scope

The retained audited address map exposes506 zero-execution source addresses in
`installer/src/remote/forward_recovery.rs` and397 in `remote/linux.rs`; these are
candidates needing assertions, not a proved reachable count. The LLVM aggregate
gap count and LCOV address count remain distinct. Nothing is excluded or removed
from the denominator by this cohort.

The new test module exercises the public `recover-user` dispatcher and whole
prepare/apply/status flows, preserving the original unknown receipt. Its23 private
transaction scenarios are grouped into eight test families; a ninth test checks
ordinary system-protocol discovery and three non-root refusals. Real owned SQLite,
filesystem publication, manifest/hash verification, kernel file locks, inherited
helper leases and bounded Python/curl helpers run inside the existing private
fixture. One owned loopback health observer has a45second lifetime and bounded
read/write; its socket/thread are retired and joined before fixture removal.

Service-manager responses and process identity use the existing thread-local test
seam; shell catalogue programs are synthetic. An unexpected manager call fails
instead of invoking a real manager. This does **not** establish native systemd,
real supervisor PID, authenticated public Helm transport or release qualification.
No additional instrumented normal binary or dependency is introduced. Full Cargo
measurement must retain all current workspace objects and owned profiles normally.

`remote/system.rs` is not universally ordinary: its namespace admission requires
real/effective UID0. This cohort checks its public protocol and ordinary caller
refusal before that namespace, not privileged publication/adoption. The remaining
Root/system responsibilities are retained; no module-wide reachability exclusion
or native Root success is inferred.

## Whole transaction assertions

| Family | Assertions |
| --- | --- |
| Two forward completions | Exact target pointer/publication, separate complete recovery, original receipt still unknown and byte-identical, retained canonical/account/session/formats, snapshot, no quarantine, no duplicate apply. One path tolerates only semantic-equal idle notification layout drift before quiescence. |
| Fourteen pre-effect context refusals | Original receipt/metadata/pointer/running manifest/target manifest/target binary/account/other private state/notification clock/cleanup/other unknown operation/unit/execution lease/new journal schema changes refuse before quarantine/snapshot/publication. Independent changes are preserved. |
| Gateway stop refusal | Unknown separate recovery and quarantine are retained; no snapshot, old apply replay or automatic rollback. |
| Target start refusal after publication | Target pointer and raw snapshot remain retained, not silently downgraded; status cannot start a stopped target or infer readiness. |
| Target account writer | A real private fixture write during target readiness breaks held proof; changed bytes, snapshot, quarantine and unknown recovery remain observable, with no restore/replay. |
| Target idle notification writer | Semantic equality cannot override the held raw snapshot; even an idempotent notification header write prevents completion and status reconciliation. |
| Observation reconciliation | A published target that initially fails exact health/version observation remains unknown; later exact readiness/proof observation can finish without installation, start/stop/enable or old apply replay. |
| Complete-proof tamper | Replacing snapshot bytes or target executable cannot use a complete label to supersede the original uncertainty. |

Fixture release versions are synthetic values, not release planning. The old
pointer is materialized while the fixture has no migrated database, then its
synthetic already-migrated state is seeded. Setup never rolls an old binary over
schema2 and never fabricates a rollback reader/backup for the production host.
Declared compatible running/target fixtures exercise the general transactional
path; the real undeclared-source case remains covered by its earlier focused
regressions and distinct native checkpoint.

## Coordinated gate after integration

Compile and run this exact family only after the gate owner integrates the source.
Keep private umask, one compiler job, no swap and bounded independent unit ownership:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/forward-owned-focused-summary.json -j1 \
  -- forward_recovery::owned_transaction_tests
```

The focused JSON can contain retained mixed objects; keep it ignored/unpublished.
Do not combine `--no-clean` with `--no-report` on the installed coverage version.
Use the existing instrumentation/target and a bounded systemd user unit under an
ordinary account. The system-refusal test intentionally refuses a root test runner;
it must not inspect or change a real system installation. Resolve concrete compile,
queue/lifecycle or assertion failures before the final relevant edit is frozen.
Then run formatting/strict Clippy and an independent complete workspace coverage
measurement, archive preceding profiles/objects, audit the complete current object
set and publish only the compact summary. Own the resulting main publication and
actual hosted archive follow-through. No focused/source-prep outcome closes353 or
replaces remaining native/browser/platform/provider obligations.
