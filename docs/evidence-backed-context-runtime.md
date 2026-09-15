# Saved tool evidence: implemented runtime slice (#282)

This is the implemented slice of the [design](evidence-backed-context.md), not a claim that its entire rollout/benchmark plan has passed.

## Behavior

The standard Voyage registry now exposes `result` with `describe`, `read` and literal `search` actions. It reads immutable session-owned evidence, not the source command or current filesystem. It is available in read-only mode, still checks current execution authority, and cannot select another session through its arguments. Existing separately authorized Vessel/branch access remains unchanged.

After ordinary tool output validation, confidentiality processing and output-limit classification, the runtime attempts to retain fallback text between 16 KiB and 2 MiB as a versioned JSON artifact. The manifest contains tool/call/execution identity, the recorded outcome, capture status and retained text. Existing artifact storage enforces hashes, private ownership, 4 MiB objects, 256 objects and the 64 MiB store ceiling. No new database or quota increase is introduced. Serialization expansion can exceed the artifact limit; retention then falls back to existing canonical output without a reference. Results of `result` itself are not recursively retained.

The canonical result remains intact, with an additive artifact reference only if storage succeeds and the reference fits the output budget. A failed save does not change an already-observed command outcome. Existing capture loss is not repaired. Capture is `incomplete` when the recorded outcome indicates incompleteness, otherwise `unknown`: older adapters do not attest source completeness. Retained text is the redacted fallback representation, not necessarily original stdout or separate streams.

Context preparation projects large results with a saved evidence reference immediately, even below the general 192 KiB preparation threshold. It keeps a bounded extractive preview, retrieval instructions, artifact metadata and existing tool outcome/identity. Canonical text is unchanged. Saved projections retain fingerprint validation and clear provider continuation state; subsequent group reductions retain artifact references too. Restart uses the existing persisted working-context and artifact lifecycle. Unavailable evidence produces an error instructing against command replay, not an empty success.

## Query contract

```json
{"action":"read","id":"<artifact UUID from result>","offset":0,"limit":4096}
```

`limit` is 1–4096 UTF-8 bytes of retained text. A read can advance up to three additional bytes to avoid splitting a code point; the actual `end` and `next_offset` are authoritative. Offsets inside a code point are rejected. Responses include text hash, total bytes, capture status and recorded outcome. The whole JSON envelope must fit the runtime output budget; request a smaller limit if refused.

```json
{"action":"search","id":"<artifact UUID>","offset":0,"limit":4096,"query":"failed:test_name"}
```

Search is literal, case-sensitive, with a 1–256 byte query. It scans bounded pages and returns at most 32 match ranges, not unbounded snippets. It extends inspection across page boundaries to detect crossing matches. Follow `next_offset` until null; an empty page before scan completion does not prove absence. Read returned ranges to recover exact evidence. `describe` returns metadata without text. No cursors, regex evaluation, arbitrary code, filter/group or implicit execution are introduced.

Every retrieval loads and verifies the existing artifact, checks current authority, and refuses disclosure if newly known secrets occur anywhere in retained text—even outside the requested page. Normal registry output confidentiality checks also apply. The backing artifact API loads at most the fixed object limit; this is not streaming multi-gigabyte storage. Cancellation is checked before retrieval; bounded synchronous local loading/search remains subject to those fixed limits.

## Request byte instrumentation

All four model transports (OpenAI Chat, Responses, Anthropic, ChatGPT OAuth), in complete and streaming paths, serialize the final request once and send those exact bytes. `voyage::request_accounting` tracing events contain only counts:

- `total_bytes`
- `instructions_bytes`: top-level instructions/system value encodings
- `schemas_bytes`: tools/functions value encodings
- `history_bytes`: messages/input value encodings
- `envelope_bytes`: all remaining keys, syntax and values

The components sum to total bytes. Chat-format system messages remain inside history; this coarse breakdown is not semantic per-role attribution. Events indicate request construction, **not dispatch, provider usage or billing**. Conversation attempts now persist these observations in `retry.request_bytes` on the existing durable provider-attempt record, available through existing inspection. Failed/cancelled dispatch paths retain the observation when their final attempt checkpoint succeeds. Legacy/unobserved records remain null, not zero. An abrupt process death before that checkpoint can still lose the byte observation. Multiple body encodings within an attempt are accumulated. Title requests currently have tracing only. This does not add inference-ledger fields, cache/reasoning token counters or task-wide cost aggregation. OAuth measurements occur after its request-body adaptations. Auth/device/token requests are excluded.

## Verification and remaining scope

Offline regressions cover UTF-8 pages, cross-page matches, bounded match pagination, incomplete/unknown capture, exact middle SHA/test/refusal recovery, missing/foreign-session evidence, current-secret refusal, registry redaction before storage, no tool re-execution during repeated reads, cancellation, stable projection/restart and preserved canonical outcomes. Request tests reconcile escaped Unicode/envelope bytes and actual RequestBuilder body bytes. Existing provider tests exercise the adapters.

The implementation intentionally does **not** add learned compression, a competing todo/completion state machine, schema loading, structured filter/group, new sharing permissions or unlimited retention. Still required for the full design: finer-grained accounting and title/usage ledger joins, representative behavioral benchmarks/cache measurements, evidence-linked task-state integration, broader quota/crash/branch/revocation tests and approved rollout decisions. No live-provider token/cost improvement or native Windows/macOS result is claimed. Keep #282 open for this remaining scope.
