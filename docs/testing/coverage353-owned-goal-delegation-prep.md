# Prepared owned Goal delegation and accounting journeys (#353)

This cohort is **uncompiled and unrun** source preparation on
`coverage353-owned-goal-delegation`, based on
`036ea6e487114dd2ae34ed30fce0f53ab1dd6919`. Its
[objective was recorded before edits](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5953684217).
No Cargo/tests/Main/shared-target/profile/native operations occurred in preparation.

## Scope and selected input

Latest accessible audited e106/036 coverage maps51 unique zero candidate addresses
in `tools/vessel/goal_budget.rs`. These are inputs, not proven reachability,
coverage gains or exclusions. The complete100 target and deferred
Root/system/adoption/native/platform/provider obligations remain unchanged.
Only the module gains a test declaration, alongside new
`goal_budget/owned_delegation_journey_standard_tests.rs` and this runbook.

Seventeen semantic Tokio parents exercise production `ManagedSessionOwner` /
`UsageObserver` / actual private SQLite allocation and dispatch, whole live
ToolRegistry/VesselTool create or Submit, and explicit public receipt/Resolve
reconciliation. There are no child entry tests or new normal binaries/objects.
Existing tests use mock Accounting and direct `perform`, or durable helper-level
fixtures; these new cases join the actual owner/registry/public boundary.

The ordinary synthetic parent is a real SQLite admitted delegated run. Its
`Running`, interruption checkpoint and cleanup fields are fixture protocol state,
not a spawned executor, observed process death or native cleanup claim. There is
no provider request, execution agent, paid budget consumption, OS Root authority,
new credential or live production endpoint.

## Cross-boundary assertions

| Family | Meaningful effects and refusal oracle |
| --- | --- |
| Whole Submit/create | Production `begin_execution_meter` creates the real UsageObserver. The HTTP peer independently opens the actual private SQLite: allocation exists before StartSettings, frozen exact dispatch exists before Submit, wire budget and immutable command agree. One accepted RunID is shared with the stable later receipt. |
| Capability/idle/cleanup refusal | Unsupported execution-budget capability, active child, retained pending cleanup and missing idle proof refuse before any child allocation, creation or Submit. The tool's durable outcome remains conservative unknown, not a fabricated definite non-admission. |
| Lost accepted reply | A single scripted accepted Submit loses its response. Exact later usage can settle accounting, but the durable tool result remains unknown/no replay; it is not rewritten as a confirmed reply. |
| Unknown/refused/missing route | Fully awaited reconciliation leaves the entire private SQLite graph unchanged on unknown receipt, denied History, wrong destination, disabled/missing route, changed exact budget/session/run or malformed supplemental evidence. No Submit/creation/replacement ID is issued, no missing credential directory is recreated. |
| Exact late usage | Bound complete or incomplete usage updates only separately named observed counts. Original terminal execution usage stays immutable, repeated exact import changes nothing after owner reopen, and late data never restores parent continuation. Cleanup/complete fields are typed peer evidence, not native child retirement. |
| Non-admission/Resolve | Exact negative command receipt closes the allocation without fabricating a zero-use run. Wrong negative ID cannot refund it. Fencing sends the exact original recorded command through Resolve, never another Submit. Terminal gated-undispatched and already-closed allocations need no child receipt RPC. |

Refusal and idempotency comparisons enumerate **all** private SQLite tables and
columns as typed values, including BLOBs and side-table observations/receipts.
No side effects are excluded. Reopened fully awaited reconciliation drops its
owner and reacquires the actual execution lease. Private credential contents are
not read or emitted. Every route/file/token belongs to the synthetic fixture.

## Causality and owned observer limits

The owned loopback public HTTP peer is bounded, validates the fixed command
path/protocol/synthetic local Authorization, and positively joins socket tasks,
retires its listener and confirms connection refusal at shutdown. It never
launches a subprocess/provider/executor. Fresh readonly Receipt returns unknown
until the exact Submit passes the peer's durable dispatch check; it cannot settle
an allocated-but-undispatched child. Accepted reply and all later usage polls
share one stable run/receipt tuple.

Production `prepare` currently starts a background observer without exposing a
JoinHandle. `meter.wait_allocations` establishes settlement **not independently
observed observer-task retirement**. The fresh positive cases retain that limit;
no test hook, sleep-based cleanup proof or synthetic task-join claim is added.
The fully awaited explicit reconciliation cases and owned peer joins provide only
their own cleanup evidence. No comprehensive historical descendant/native proof
is inferred from fixture disposal or a protocol cleanup field.

## One coordinated gate

After independent review and integration with the complete public-coordination
cohort, freeze all relevant source. Use existing workspace target/instrumentation,
private bounded ordinary-user unit and one compiler job:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/owned-goal-delegation-focused-summary.json -j1 \
  -- tools::vessel::goal_budget::owned_delegation_journey_standard_tests
```

Focused summary remains ignored/unpublished; retained mixed objects are not a
new valid full measurement. Do not combine `--no-clean` and `--no-report`. Resolve
concrete failures, complete required format/strict Clippy and independent full
workspace coverage after the final relevant edit. Retain preceding and foreign
evidence, audit all current source/objects, publish compact coverage/main source
and own actual hosted archive follow-through. Source prep does not close353 or
any full milestone/native/browser/platform/provider acceptance.
