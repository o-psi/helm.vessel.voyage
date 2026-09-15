# Evidence-backed working context: design and evaluation

Implementation update: [runtime slice and remaining scope](evidence-backed-context-runtime.md). The user subsequently authorized code changes; the implemented subset is documented there. The broader interfaces and evaluation gates below remain a design, not a claim of current support.

Status: research/design deliverable for [issue #282](https://github.com/o-psi/helm.vessel.voyage/issues/282), incorporating its [research review](https://github.com/o-psi/helm.vessel.voyage/issues/282#issuecomment-5685740013). **Proposed interfaces below are not available tools.** This document does not enable result offload, change storage limits, or authorize broad runtime rollout. Agreement on the stages and gates below is required before that work. Source inspection is not an end-to-end runtime test.

## Decision

Extend the existing session-owned artifact infrastructure if lifecycle and capacity experiments pass; do not introduce a second execution plane or a parallel result database by default. Keep canonical evidence distinct from model projections. Start with request accounting and equivalent-output existing-tool controls, then selective retrieval, reference-preserving context reduction, and small evidence-linked state. Learned compression and progressive schemas are separate experiments, not prerequisites.

Success is lower **total task** model usage with equally correct, safely evidenced outcomes. A smaller first response, fewer tool calls, or fewer stored bytes is insufficient.

## Evidence and scope

Inspection baseline: local source based on `091fe1f3900382cdd446af9e6fec128a60b35d89` (2026-09-15). The checkout contains unrelated UI edits and missing quality/packaging scripts; these are not part of this proposal. Paths below describe inspected source, not installed binary behavior. Historical ignored shell `max_output_bytes` arguments require matching the historical binary/schema before drawing conclusions.

The issue's seven-voyage audit reports 7,626,325 result-text bytes counted once per result, **not deduplicated unique content**. Shell accounts for about 95%, but shell is not a content type. Mean result and shell result sizes are approximately 4.75 and 6.29 KiB. The 308 results over 8 KiB contain 5,338,201 bytes; only 2,815,065 bytes are above the cutoff. This is not a safe truncation policy or savings forecast. Exact repeated same-tool output is only about 0.23%. Two voyages dominate the sample. Cumulative provider counters cannot establish replay amplification, component attribution, or uncached price. No new benchmark results are claimed here.

## 1. Current implementation and boundaries

| Stage | Source and observed responsibility | Consequence |
| --- | --- | --- |
| Execution | `voyage/src/tools/shell.rs`: `ShellArgs` accepts command and explicit workflow-secret references; execution uses the runtime output cap and policy checks. `voyage/src/tools/filesystem.rs` supplies file reads/searches. | Model-supplied shell output-limit keys are not an implemented paging contract. Switching tool names alone does not reduce output. Capture limits can already lose bytes before context preparation. |
| Structured ingestion | `voyage/src/tools/output.rs`: `scrub`, `reject_split_text`, `redact`, `ingest` apply redaction/refusal rules; ingestion validates size and authority, then calls artifact storage for MCP results. | This is useful infrastructure, not general shell-result offload. Refused secret-bearing binary content must not become a readable artifact. |
| Artifact evidence | `voyage/src/artifacts.rs`: `Store::put/load/get/chunk/copy_to`, `Scope`. Immutable IDs and SHA-256 metadata, session-scoped loading, bounded byte chunks and branch copying. | Reuse these integrity/ownership primitives. Callers still authorize access. Current 4 MiB/artifact, 256-artifact, 64 MiB database limits and 8 MiB MCP result limit are not a policy for arbitrary large shell captures. |
| Canonical and working history | `voyage/src/context/working.rs` prepares extractive working context; `voyage/src/context.rs` estimates request sizes conservatively from bytes. | Projection is distinct from source evidence; byte estimates are not provider tokenization. A new reference must survive reduction and remain resolvable without replaying a tool. |
| Provider encoding | `voyage/src/provider/openai_responses.rs` encodes replay/tool output and parses aggregate usage; other adapters require separate measurement. | Measure the actual encoded request, not just canonical text. Cache counters and omitted usage differ by transport. |

Additional execution detail: `ToolRegistry::execute_report_with_workflow_secrets`
in `voyage/src/tools/mod.rs` validates/redacts typed output and enforces fallback
budgets. `agent.rs::run_inner` records fallback `Message.content`, typed
`Message.tool_output` and `tool_outcome`; counting both text representations
would double-count the logical payload. `session/checkpoint.rs` keeps canonical
history and working context separate; `SessionStore::save_with_lease` in
`session.rs` enforces lease/revision checks and projection fingerprints.

`WorkingContext::prepare` currently triggers above 192 KiB serialized projection
size with 4 KiB excerpts. Recovery tries 4 KiB, 1 KiB and 256-byte excerpts then
whole tool-group reduction. It protects task/steering/user messages and retains
outcomes/artifact references; reductions clear provider continuation state.
`agent.rs` then applies completion/visual projections, regenerates guidance,
includes tool definitions, resolves inference, redacts outgoing copies and
reconciles interrupted calls before encoding. Inspect `provider/openai.rs`,
`provider/openai_responses.rs`, `provider/anthropic.rs` and
`provider/chatgpt_oauth.rs` independently: OAuth modifies the shared Responses
body, so measurement before final transport adaptation is insufficient.

Canonical history is authoritative for what was recorded, not proof that an external command captured every byte or that every assertion in its output is true. Existing artifact chunk APIs and routed Vessel history inspection are distinct from the proposed model-facing saved-result query contract. Vessel inspection is already model-accessible where routes authorize it; do not describe it as unavailable or replace it with a new supervisor plane. Voyage remains the owner of execution, context and persistence; Helm only presents controls/status.

## 2. Proposed saved-result contract

### Identity and completeness

Introduce a versioned result manifest associated with the canonical tool result. It references existing artifacts rather than making artifact IDs bearer capabilities. Illustrative fields (not a current schema):

```json
{
  "version": 1,
  "result_id": "opaque immutable identity",
  "owner_session": "session UUID",
  "run_id": "run UUID",
  "tool_call_id": "canonical call identity",
  "canonical_message": "stable storage locator, not a mutable array offset",
  "tool": "shell",
  "captured_at": "RFC3339 timestamp",
  "outcome": {"kind": "exited", "exit_code": 1},
  "capture": {"status": "complete", "observed_bytes": 18520, "lost_bytes": 0},
  "representation": {"kind": "redacted", "version": 1},
  "streams": [{"name": "stdout", "artifact_id": "UUID", "sha256": "hex", "bytes": 18520}],
  "projection": {"included_ranges": [[0, 2048]], "omitted_ranges": [[2048, 18520]]},
  "availability": "available"
}
```

Required distinctions:

- `capture.status`: complete, truncated, interrupted, or unknown. A known truncation is never complete merely because every retained byte is stored. Unknown original length/lost count is null, not zero. Separate stdout/stderr and their actual ordering guarantees; do not fabricate a combined chronological stream.
- `representation`: redacted source evidence versus derived projection; version and hash describe the **stored redacted bytes**. Do not publish secret hashes or retain a raw secret copy as a shortcut to completeness. “Complete” means complete under the declared capture/redaction policy, not original unredacted bytes.
- `outcome`: success/failure/refusal/timeout/cancelled/unknown independently of capture and retrieval. A known exit code remains visible even when the body is offloaded; exit zero is not proof that the intended tests ran.
- `projection`: exact half-open byte ranges and separate derived counts/summaries. Completeness of a preview and of capture are independent. Include restrictions, unresolved errors, uncertain effects and where to find details; do not infer relevance from head/tail alone.
- Store command provenance as a canonical redacted reference, not a second copy of credentials or arguments. Derived results record parent hash, transformation/version, parameters and covered ranges. Counts over incomplete input explicitly describe only the captured subset.

### Query operations

Propose one discoverable result-query surface, initially `describe`, `read`, `search`; later allowlisted structured `filter` and `group`. Retrieval never invokes the original tool.

| Operation | Request | Required response |
| --- | --- | --- |
| describe | Authorized owner/result identity | Manifest, available streams, capture/outcome, current availability, supported operations and limits |
| read | Stream, half-open byte offset and bounded limit | Bytes/text, actual range, hash, representation, explicit continuation and end marker |
| search | Stream, bounded literal or Rust-regex pattern, cursor, match/output limits | Matching ranges/snippets, ranges scanned, has-more/continuation, timeout or budget exhaustion; zero hits is not “no matches” until scan complete |
| filter/group | Registered parser/version and allowlisted fields/operators | Derived rows/counts, parse failures, source coverage, stable pagination; no arbitrary code or shell expression |

Byte offsets refer to immutable stored representation, never model tokens. Text reads return UTF-8-safe boundaries and actual offsets; binary reads require an appropriate existing disclosure channel, not blind text decoding. Line ranges may be a convenience index with explicitly defined newline handling. A stable cursor binds result hash, query, order and position; changing parameters invalidates it. Return bounded errors for invalid ranges, unsupported encoding, revoked access, hash failure, resource limits and unavailable evidence. Expired cursors may restart the same **read** but cannot trigger execution. Define and publish concrete per-call scan/time/output limits before implementing the schema.

Initial working projection: compact manifest, outcome, capture warning, useful index and a bounded excerpt. Keep small results inline unless evaluation shows benefit. The 8 KiB audit cutoff is a benchmark arm, not the selected shipping threshold. Retrieval follow-ups count toward total cost. Deduplicate identical read requests within a task when authority and immutable identity still match; after two identical unproductive expansions, require a different query or report inability to recover evidence rather than loop.

### Durability and lifecycle

Persist redacted bytes before atomically publishing a canonical manifest; a crash must not produce a resolvable-looking dangling reference. Design reconciliation for artifacts written before a failed canonical commit: unreachable artifacts can be collected only after confirming no canonical/branch reference. Never re-execute to repair capture. If offload fails, retain the existing bounded inline outcome plus explicit storage/capture failure; do not suppress a command's already-observed effect or claim full retention.

Retain referenced evidence for the canonical session's retention lifetime by default. Context reduction does not delete it. Session deletion/explicit retention policy may remove evidence; retain an authorized tombstone with reason where policy permits, not fabricated empty content. Capacity exhaustion must refuse further capture/offload explicitly, not silently evict referenced artifacts. Prototype chunked large text within existing storage first; measure overhead and transaction limits before approving a quota increase or a new backing store. A 64 MiB database cannot be assumed suitable for long sessions merely because a 4 MiB object fits.

Branching uses verified `copy_to` semantics and records source provenance plus destination ownership. Decide and test atomic branch admission when referenced copies exceed destination quota. Do not quietly claim the branch has complete evidence. Restart must preserve IDs, hashes, manifests and unavailable status. Migration/version incompatibility is an explicit inability to read, never permission to rerun.

### Authority and privacy

Every describe/read/search/filter/group checks current route/session access and local runtime policy, including revocation and expiry, before disclosure. Cross-session access is permitted only through explicit existing authorization or branch-copy semantics; references themselves convey no authority. Querying must not broaden filesystem roots, browser disclosure/upload grants, private-terminal capture, secrets, or execution rights. Redact before storage and before derived output; test secrets split across blocks and stream/chunk boundaries. Hidden human terminal input is not a capture source. Treat retrieved text as untrusted evidence, not runtime instructions. Resource limits and cancellation apply to scans and parsers as well as tools. Preserve authorization failures in canonical history, but retain only relevant unresolved restrictions in active context.

## 3. Evidence-linked task state

A versioned working record complements/replaces redundant transcript projection; it is not appended as an ever-growing second transcript. Build on existing durable todo/evidence semantics rather than creating a competing completion authority. Example proposed record:

```yaml
version: 1
revision: 7
objective: Verify the packaged executable includes the fix
constraints:
  - No redeploy or command replay without authorization
observations:
  - id: O1
    claim: Installed version differs from the inspected source revision
    status: observed
    evidence:
      - result: R12
        stream: stdout
        range: [0, 80]
        sha256: stored-representation-hash
    limits: Version output alone does not identify packaging inputs
assumptions: []
unknowns:
  - Which checkout supplied packaging inputs?
next_actions:
  - Inspect packaging input path
unresolved:
  - kind: failed_verification
    evidence: R18
pending_effects: []
supersedes: []
```

Claims can be proposed, observed, contradicted, stale, or withdrawn. Evidence links attest provenance, **not truth**. An intended action cannot transition to an observation merely by completion status; require a recorded outcome and preserve its limitations. Verification of the intended suite requires test identity and actual execution/result evidence, not just exit zero. Human attestations are separately attributed. Conflicting observations coexist with a conflict/unknown entry; never select the most convenient claim. A derived observation points to its derivation and original range.

Use append-only canonical state revisions plus a bounded current projection, with optimistic revision checks and atomic updates. Resolve pending effects only from observed receipts/outcomes; a timeout or disconnect remains uncertain and cannot authorize replay. Retire resolved failures from active state only with linked resolution evidence; canonical failures remain accessible. Preserve user constraints and relevant refusals until explicitly superseded. Compression may shorten prose but not silently change these transitions.

Before committing a reduction, validate that referenced evidence is addressable, tool-call/result groups remain valid, and unresolved obligations/constraints survive. Missing evidence marks the claim unavailable/unverified and preserves recovery instructions. If no safe reduction fits, return an actionable limit rather than fabricate a summary. Restart/branch must carry state and references consistently. Store enough revision provenance to recover or reject a bad state update without destructive rewriting.

## 4. Benchmark and accounting plan

### Fixed corpus and experiment protocol

Create safe deterministic fixtures, not replays of mutating historical tools. Use six strata with at least five task instances each, three repetitions per configuration, paired on the same fixture, task, model/version, limits and tool availability. Freeze fixture hashes and graders before comparing results. Report every stratum and failures, not only a pooled mean dominated by long coding work.

1. Short exact lookup and small edit: inline output is expected to win frequently.
2. Long coding diagnosis: many search hits/build diagnostics, one actionable failed test buried in the middle; apply edits only in disposable fixture workspaces.
3. Prose/issue research: conflicting statements, negation, numeric exceptions and stale claims.
4. Structured analysis: JSON/CSV aggregation, malformed records, joins and explicit completeness requirements.
5. Operations/recovery simulation: refusal, timeout, uncertain effect, cancellation, restart and revoked route. Use a fake effect ledger; never execute production mutations.
6. Long multi-step task/branch: quota exhaustion, missing artifact, copied evidence and resumed unresolved state.

Arms: A current baseline; B matched-output bounded shell versus filesystem controls; C saved results/retrieval; D C plus reference-preserving working reduction; E D plus typed state; F E plus optional prose compression. Separate schema-loading and deterministic processing ablations to avoid attributing their benefit to retrieval. Keep output content comparable for B; `read_file` and `search_files` are not automatically smaller. Run cache-cold and cache-warm sequences separately with provider-supported controls where possible; record uncontrolled cache state as unknown. Live-provider runs require approved provider/budget. Offline encoding measurements establish sizes, not task quality or cache price.

### Per-request record

Assign task, run, request and attempt IDs; record provider/model/transport, fixture/source/config versions, timestamps, projection generation, outcome (including retries/rejections/cancellation), and usage availability. Instrument each provider's final serialized request boundary, after projection and encoding, without logging secrets or raw private payloads.

Use disjoint tagged byte buckets: system/runtime instructions, tool definitions, user content, assistant prose, tool-call arguments, tool-result bodies, working-state content, and serialization/other envelope. Record total encoded bytes and assert bucket sum equals total. Also retain **orthogonal** origin tags (canonical replay/new/retrieval/derived) and canonical-versus-projected sizes; do not sum origin views with component buckets. Capture duplicated runtime-inventory text separately as an attribution within instructions, not another additive category. Character bytes and estimated tokenization are not actual provider tokens; tokenizer estimates must name tokenizer/version and are not assumed additive across boundaries.

Record actual input/output, cached input/write/read and reasoning details only where exposed; absent counters are null, not zero. Provider accounting may include cached input inside input totals: preserve raw field semantics and normalize with documented adapter/version mapping. Sum actual attempt totals once for task totals; reconcile against run/session rollups without adding them. Include failed/retried calls and compressor usage. Report unknown-usage attempts and known lower bounds rather than manufacture complete totals. Price only with dated model-specific rates and supported non-overlapping cache fields; otherwise report tokens/latency without a cost claim.

Existing accounting integration: `model.rs::Usage` supplies run/session input
and output totals, but absent response usage can become zero and failed attempts
are not fully represented there. `inference/mod.rs` records optional attempt
usage and attribution; `inference/history.rs` exposes missing/reported counts
and may omit detail at capacity. Extend that ledger rather than summing it with
run totals. `crates/voyage-protocol/src/provider_attempt.rs` and
`agent/provider_attempts.rs` provide retry identities/timing and an inference
permit join key, not encoded sizes or usage. Account quota windows in
`accounts/usage.rs` are not request billing. Record omitted ledger records,
admitted versus actually dispatched requests, and partial versus final usage.
Include title generation, delegated work and all compressor requests in task
rollups, not only the main agent's completed calls.

Track retrieval calls/bytes/scan work, empty/repeated queries, state-update/compression tokens, full task wall time, TTFT when observed, peak active context, stored bytes/artifact counts, and missing/corrupt/revoked evidence. Separate harness overhead from model task latency. Canonical bytes counted once per recorded result and content-deduplicated bytes are separate metrics.

### Fidelity and outcome grading

Require exact SHA and failed-test-name recovery from omitted middle ranges; preserve a refusal, negation, numbers, uncertainty and user restrictions; recover a requested omitted range; distinguish complete capture from truncated/unknown capture. Search zero-match before end must not be graded as absence. Inject quota failure, UTF-8 boundaries, split secrets, hash corruption, cursor expiry, cancellation, restart, branch-copy failure and revoked/cross-session authority. Assert no unauthorized disclosure and no additional ledger effects during retrieval/recovery. Test all enabled provider encodings for valid tool groups and preserved references; unsupported/native platforms remain explicitly unverified.

Use deterministic exact-value/effect-ledger graders plus blinded rubric review for qualitative answers. Grade successful justified completion, not confident wording or todo status. Publish per-instance outcomes, paired token/latency differences and confidence intervals. Retain redacted evidence for adjudication. Treat excluded/failed trials explicitly; never remove difficult tasks to meet a gate.

## 5. Alternatives and trade-offs

| Approach | Benefit hypothesis | Costs and failure modes | Recommendation |
| --- | --- | --- | --- |
| Existing bounded shell/Python and filesystem tools | Reduce input at source using precise operations | Extra calls, inconsistent bounds; heuristic filters lose relevant evidence; source can change | First control; compare matched outputs and preserve source identity |
| Saved-result selective retrieval | Avoid repeatedly encoding irrelevant bulk; recover exact evidence | Storage/quota/lifecycle complexity, extra turns, bad indexes, scans and cache-prefix invalidation | Primary staged prototype using existing artifacts |
| Deterministic structured processing | Count/filter/join without model ingesting every record | Parser errors, incomplete inputs, authority expansion if arbitrary execution is added | Allowlisted typed transformations with provenance; use existing authorized shell for general code |
| Evidence-linked state | Preserve constraints and unresolved work across long tasks | Incorrect/stale claims, extra update tokens, duplicate context, invalid references | Small incremental state integrated with existing context/todo semantics |
| Learned prose compression | Smaller research/documentation context | Lost negation/exceptions; compressor compute/tokens, latency and cache churn | Optional only after earlier gains; exclude code, commands, IDs, permissions, constraints and critical outcome evidence |
| Progressive schemas / deduplicated guidance | Smaller schema/instruction prefix | Discovery turns, missing restrictions, changing tools harms caching | Separate measured ablation; retain authoritative discoverable inventory and essential policy |
| Exact-result deduplication | Cheap elimination of identical repeats | Very small opportunity in this audit; changing source identities may look identical | Secondary optimization, not project rationale |

Stable references help subsequent reuse but the **first replacement still changes the prefix**. Frequent rewriting of state or prior results may destroy cache reuse. Compare stable appended state updates, periodic projection checkpoints and current reduction behavior; do not assume cache neutrality. No learned transform touches canonical evidence. Test learned transforms on negation, exceptions and uncertainty and revert to extractive retrieval on fidelity failures.

## 6. Incremental sequence and decision gates

These are proposed acceptance thresholds to agree before runtime implementation, not measured achievements. Each stage needs a separately scoped change and recorded outcomes; failing a gate means keep the previous behavior and investigate, not expand rollout.

| Stage | Deliverable | Go/no-go |
| --- | --- | --- |
| 0: baseline | Reproducible fixtures, request accounting across enabled adapters, audit of installed/historical shell schemas | Disjoint encoded-byte reconciliation passes; all attempts represented, unknown usage explicit; graders and provider budget approved before live comparison |
| 1: controls | Matched-output existing-tool and deterministic-processing experiments | No lost requested evidence or authority change; report total turns and task cost, including no benefit |
| 2: opt-in retrieval | Manifest/storage/read/search prototype plus failure/lifecycle coverage | Zero fidelity/security/duplicate-effect failures in fixed suite; every published reference resolves or explicitly reports unavailable; durable publication/branch/quota behavior verified |
| 3: context integration | Reference-preserving reduction and bounded expansion | Same safety/fidelity gate; invalid tool groups zero; exact-value recovery 100%; at least 15% median total-token reduction on long-task strata, paired 95% interval excludes zero; no more than 5% median short-task token regression or 10% p95 latency regression |
| 4: state | Incremental evidence-linked state and recovery | No lost constraints/pending effects; no intentions promoted to facts; outcome success non-inferior within 5 percentage points with stated uncertainty; incremental total cost improves over stage 3 or state remains off |
| 5: optional experiments | Prose compression/schema-loading ablations | Critical fidelity errors zero; statistically supported incremental task-cost benefit including compressor/discovery overhead; no unsupported cache/pricing claims |
| rollout | Agreed supported providers/platforms, operator controls and rollback | Review actual results and resource/privacy behavior; widen only after agreement. Keep opt-out and old projection fallback, never delete canonical evidence to roll back |

Small pilot samples may not establish the non-inferiority margin; expand trials rather than assert success. Token savings and cost can disagree: if measured cache-aware price regresses, do not claim financial improvement or ship as a cost optimization without explicit review. Long-task gains cannot conceal short-task regressions. Thresholds may be changed only with recorded rationale before a new comparison, not retrospectively to pass.

Open implementation decisions for stage 0/2 review: manifest atomicity with session persistence, streaming redaction boundaries, artifact quota sizing/chunk layout, exact query budgets, retention policy integration, and provider-specific usage mapping. These are not grounds to promise current runtime support. Broad implementation is gated on agreement with this plan; the planning issue can close once its deliverables are accepted, with later runtime work tracked separately.

## Research attribution and acceptance map

External evidence motivates experiments; no reported percentage is a Helm forecast:

- [Context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents), [code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp), and [advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use): just-in-time retrieval, deterministic processing and schema discovery.
- [LLMLingua](https://arxiv.org/abs/2310.05736), [LLMLingua-2](https://arxiv.org/abs/2403.12968), [LongLLMLingua](https://arxiv.org/abs/2310.06839): optional compression research, not coding-evidence fidelity guarantees.
- [Don't Break the Cache](https://arxiv.org/abs/2601.06007): the issue review attributes 41–80% API cost and 13–31% TTFT improvements to the study's overall caching experiments, not exclusion of tool results specifically.
- [ACE](https://arxiv.org/abs/2510.04618): motivation for incremental updates, not proof that typed state validates factual claims.

The posted review verified selected papers/source and corrected audit arithmetic. This delivery inspects local source and adopts those corrections; it does not independently replicate the papers or rely on the full unpublished external addendum. Related [#64](https://github.com/o-psi/helm.vessel.voyage/issues/64) covers context reduction/recovery, [#71](https://github.com/o-psi/helm.vessel.voyage/issues/71) multimodal structured results, and [#79](https://github.com/o-psi/helm.vessel.voyage/issues/79) sharing/retention boundaries. Their lifecycle labels are not present-runtime verification.

Issue planning criteria map directly to sections: current flow (1), result contract (2), evidence-backed state (3), benchmark/accounting (4), alternatives (5), incremental sequence and agreement gate (6). Verification of this documentation consists of source/path/link/manifest/diff checks; runtime performance, live providers and native platform behavior remain unmeasured.
