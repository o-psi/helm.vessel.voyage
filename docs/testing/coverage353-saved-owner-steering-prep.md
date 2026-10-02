# Prepared saved-owner and steering journeys (#353)

All ten saved-owner and eight steering parents passed in the corrected
coordinated full workspace measurement of clean
`1d49636e47bd42a7aef3dc4c54741209f5790335`. Source preparation was on
`coverage353-saved-owner-steering`, based on
`fede3e8d25ecb3e0063d1d3ea323f6392a9123be`; it performed no Cargo command,
shared target/profile mutation, provider call, production host operation or
native browser journey. The scope
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

The following are maintained optional focused entrypoints, not commands claimed
executed for this delivery. The coordinator instead ran the combined full
workspace gate after all source was integrated and frozen, using the existing
target, private umask, bounded independent ordinary-user unit, one compiler job
and coordinated instrumentation:

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

## Verified measurement and remaining delivery

The corrected full workspace gate measured clean
`1d49636e47bd42a7aef3dc4c54741209f5790335`: **2,860 passed, zero failed and
eight ignored** across fifteen Cargo parent test targets. All **56** new parents
passed: eighteen Helm observation, ten saved-owner, eight steering and twenty
public-gateway cases. Child processes are not extra test parents. Formatting
and strict workspace Clippy passed.

The terminal export/audit selected **18 distinct current objects**: fifteen test
objects and three actual workspace entrypoints. It retained 499 owned profiles;
all 235 foreign profiles remained unchanged. All prior mappings were retained
across 542 workspace files plus one standard-library source. Summary, detailed
JSON and HTML totals agree, and the exact objects were archived. Lines are
**107,223 / 134,975 (79.439155%)**, functions **9,710 / 12,300**, and regions
**169,252 / 223,661**. Compared with clean BEB, covered lines increased by 606,
the denominator by 24 and coverage by 0.434924 percentage points. Accounting
reconstructed all 543 files and 12,300 function groups: 27,752 aggregate uncovered
line units versus 25,629 unique zero addresses, a difference of 2,123 = 116
maximum/union + 2,034 overlap − 27 shadowing. These are accounting categories,
not a production reachability count or exclusions.

The [compact coverage record](../../coverage/latest.json) belongs to this measured
source. Full evidence remains ignored and retained by the coordinator. Publication
and the actual hosted build/archive are pending at this checkpoint; no native
installation of this newly measured source is claimed. Existing native acceptance
remains bound to its separately recorded qualified source, including `613`.
The full measured denominator and 100% reachable-production objective stay
unchanged; [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353) and
[release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375) remain open.
