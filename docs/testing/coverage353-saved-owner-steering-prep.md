# Prepared saved-owner and steering journeys (#353)

This cohort is source preparation on `coverage353-saved-owner-steering`, based on
`fede3e8d25ecb3e0063d1d3ea323f6392a9123be`. It has not been compiled or executed.
No Cargo command, shared target/profile mutation, provider call, production host
operation or native browser journey was performed while preparing it. The scope
was recorded before implementation in [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5949762192).

## Selection and boundaries

The retained `beb` LCOV has 166 zero-execution candidate input addresses across
`voyage/src/server/suspended.rs` (78), `attachment/journal/steering.rs` (71) and
`attachment/journal/observations.rs` (17). They are selection inputs, not a proven
reachable count or promised coverage gain. Adjacent journal and process-command
code is exercised through its production interfaces. No exclusions, denominator
changes or deferred Root/system/adoption/platform work are introduced.

Only two production files gain test declarations. Ten Unix saved-owner parent
cases call the complete production `observe` function against a private, real
SQLite canonical history and actual startup/execution leases. Eight steering
parents operate real SQLite transactions, immutable image storage and journal
reopen. Failed/duplicate/collision operations compare every private SQLite table
and column, including typed BLOB storage values, indexed provenance/revisions,
durable events, process observations, goal scheduling and command receipts. No
observable side-table mutation is excluded. Both use synthetic local input and
existing workspace objects. They do
not launch an agent, provider, executor, service manager or OS helper process.

The fixture writes `stopped.json` explicitly to test protocol evidence. Its
`cleanup_observed` field is not proof that a real process died. Lease release is
observed by reacquiring actual locks after every observer call; the two held-lease
journeys also await the spawned observer task. Temporary state is owned by the
fixture and removed on drop. This does not establish native updater/browser or
historical descendant cleanup.

## Assertions

| Family | Production behavior and oracle |
| --- | --- |
| Saved canonical observation | Whole Snapshot, paged History and bounded UTF-8 MessageChunk preserve all private SQLite table rows, canonical Unicode text and private provider-state sentinel; public replies omit provider state. No runtime endpoint is created. |
| Range and authority refusal | Invalid bounds, stale revisions, unsupported projections, wrong protocol/session/incarnation/token, relinquished registration and missing/invalid existing actor refuse without initializing authority or mutating private rows. |
| Retirement evidence and locks | Saved read access remains distinct from exact Stop evidence. Startup and execution locks delay observation until released; final reacquisition proves the observer did not retain ownership. |
| Receipt and exact resolution | Metadata receipts survive owner drop. Unknown receipt lookup reserves nothing. Resolve settles the exact never-admitted command ID; changed original input and later admission cannot replay it. |
| Events and missing workspace | Whole saved public/live event projection retains session boundaries and redacts private evidence. Missing workspace and stale endpoint do not hide canonical history or cause recreation; exact negative delivery resolution remains distinct from execution. |
| Steering order and reopen | Two admitted inputs retain exact actor, payload, receipt IDs and FIFO ordinals through atomic canonical application and reopen. Exact duplicates return the original receipt without another append. |
| Immutable refusals | Changed receipt context/payload, corrupted SQLite provenance, command-ID aliases, reordered/tampered/expired canonical messages and repeated application refuse without partial receipt/message mutation. |
| Queue and terminal obligations | The pending-input limit refuses an unreserved new ID. A completed result cannot hide queued obligations. Explicit durable rejection permits completion and terminal admission remains refused. |
| Image and rejection lifecycle | Mixed text/image consumes exact session-owned PNG bytes and checkpoints original parts. Changed image hashes refuse. Rejection is durable, reason-immutable and never adds rejected input to canonical history. |

## One coordinated gate

Run only after the complete source cohort is integrated and frozen by the gate
owner. Use the existing target, private umask, bounded independent ordinary-user
unit, one compiler job and coordinated instrumentation:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/saved-owner-focused-summary.json -j1 \
  -- saved_owner_journey_tests

CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/steering-owned-focused-summary.json -j1 \
  -- steering_owned_journey_tests
```

Focused summaries can contain retained historical objects and remain ignored and
unpublished. Do not combine `--no-clean` and `--no-report`. Resolve concrete
failures, then complete required formatting/strict Clippy and fresh full workspace
coverage after the final relevant edit. Preserve prior evidence and foreign raw
profiles, audit all current workspace objects and publish the compact summary
with main delivery and actual hosted archive follow-through. Source preparation
and focused passing assertions alone do not close #353 or the milestone.
