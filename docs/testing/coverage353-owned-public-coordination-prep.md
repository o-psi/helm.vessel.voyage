# Prepared whole public Vessel tool journeys (#353)

Uncompiled/unrun source on `coverage353-public-coordination`, based on
`036ea6e487114dd2ae34ed30fce0f53ab1dd6919`. The
[objective was recorded before edits](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5953417481).
No Cargo/test/Main/shared-target/profile/native operation occurred in preparation.

## Selected input and current boundary

Latest audited `v103-courier-extension-production/current.lcov` supplies207 unique
zero-execution candidate addresses: `tools/vessel.rs`133, `pagination.rs`47,
`transport.rs`18, `journal.rs`9. These are selection inputs, not proven reachability
or promised gain. The51 Goal-budget observer addresses remain a separate retained
obligation. No denominator or source exclusion changes;100/full scope persists.

The production route is ordinary live `ToolRegistry` → `VesselTool::execute_report`
→ current local policy/trusted routes → private intent/wire/result → public HTTP.
Helm and Vessel remain presentation/supervision surfaces; this model-facing tool
coordinates independent voyages rather than executing their agent loops.
Existing direct `perform` matrix and pure output/page tests do not exercise this
whole route; the prior whole mutation test covers rename success/dedupe only.

Only `tools/vessel.rs` gains a test declaration. New
`tools/vessel/owned_public_journey_tests.rs` has18 semantic Tokio parents; no child
entrypoint or additional Cargo object is introduced. All call actual
`ToolRegistry::execute_report_with_workflow_secrets`, using a registered production
VesselTool and private fixture owner directories.

## Behavior and oracles

| Journey | Assertions |
| --- | --- |
| Whole mutation success | Exact original request intent and transmitted command are already private/durable at the peer's first effect boundary. Configured secret is redacted before durable result/output. Rebuilding the registry returns the same retained result without HTTP; changed payload cannot reuse its ID. |
| Lost/cancelled/timed-out reply | A single actual HTTP mutation has its exact wire retained and publishes durable unknown, not non-admission. Rebuilt registry cannot resend or generate replacement IDs; cancellation does not send a remote Cancel. The synthetic late reply is not claimed as remote process cleanup. |
| Receipt and operations | Unknown command receipt and operation enumeration are local retained observations, make no HTTP request and cannot create an executor or another intent. |
| Local current policy | Readonly, cancelled invocation and withdrawn execution authority refuse before a new private mutation journal/effect. Cached continuation is withheld when its current local authority is withdrawn. Missing owner context refuses mutation but route metadata remains no-start. |
| Action and public refusal | Unknown configured route, malformed action/bounds/relative creation input refuse before persistence/contact. Server definite/unknown scope refusals redact diagnostics and remain explicit refusal/unknown. Wrong public session envelope withholds payload. |
| Inspect/history/events/catalogue | Whole readonly registry calls preserve unavailable snapshot/registration distinction, explicit event-gap fresh-inspection continuation, canonical Unicode and no fake cleanup; no mutation journal appears. |
| Run output and complete public message | Actual bounded HTTP chunks and model output pages reassemble exact UTF-8/redacted content, including a configured secret split across the64KiB source boundary. Live run state remains live and text is not completion evidence. The message fixture is already the upstream public projection: absence of provider state is a fixture boundary, not a new backend private-field sanitizer claim. |
| Cached continuation | Offset/run/session/redactor mismatch releases no cached bytes and makes no RPC. Matching continuation probes current remote authority before another page; a refusal returns no cached payload. Malformed run/offset/next/total/data bounds fail before a partial page. |

## Owned peer and evidence limits

The peer is a real owned loopback HTTP listener. It validates the fixed public
command path/protocol and synthetic local Authorization, captures typed requests,
and holds each reply for the exact parent oracle. Each socket read/reply and
parent call has a four-second bound; only the tested mutation deadline is shorter.
Shutdown stops admission, joins all socket tasks, retires the listener and confirms
a fresh connection cannot reach it. No detached process/executor/provider/native
service is launched. Task/socket retirement is not historical descendant cleanup.

Temporary owner/resources/journal trees are private; model output and persisted
results are checked for synthetic credential disclosure. Fixture signatures,
backend grants, physical Vessel/cryptographic identity and actual accepted runtime
receipts are not invented: public replies here are typed controlled peer evidence.
No production connection credential, URL rewriting, extra authority or remote
scope is created. Backend/native acceptance remains separate.

## Coordinated single gate

After independent source review and integration, freeze all relevant source under
the gate owner. Use existing target/instrumentation, private bounded ordinary-user
unit and one compiler job:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/public-coordination-focused-summary.json -j1 \
  -- tools::vessel::owned_public_journey_tests
```

Focused summary remains ignored/unpublished and can contain mixed retained
objects. Do not combine `--no-clean` and `--no-report`. Resolve concrete failures,
then required formatting/strict Clippy and an independent full workspace coverage
measurement after the final relevant edit. Retain prior/foreign evidence, audit
all current objects/source attribution and publish compact coverage with main
changes and actual hosted artifact follow-through. Source prep does not close353,
native/browser/platform/provider obligations or the full milestone.
