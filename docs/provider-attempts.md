# Durable provider attempt observations

Issue [#261](https://github.com/o-psi/voyage/issues/261) adds provider-attempt
metadata to each run summary, separate from canonical messages and tool outcomes.
The shared contract is
[`ProviderAttempt`](../crates/voyage-protocol/src/provider_attempt.rs).
The agent owns dispatch/retry decisions; this persistence and presentation layer
never dispatches effects or grants permission to retry.

## Persistence contract

The runtime calls `RunCheckpoint::provider_attempt` before dispatch and updates
the same attempt ID with the observed outcome before scheduling another attempt.
A request ID groups attempts; distinct attempt IDs preserve every attempt in order.
Upserts reject changed request, ordinal, limit, start time, provider or model under
an existing attempt ID. Provider/model identifiers must be redacted and bounded by
the producer; categories must be allowlisted static labels, never error strings,
request/response bodies, credentials or private tool input.

`SessionCheckpoint` saves a candidate copy before replacing its in-memory state.
The attachment journal writes under its existing execution guard, transaction,
revision and commit fences. Only the current running run can accept metadata.
Cancellation intent does not make outcome recording unauthorized: the journal
keeps the run `Running` while cancellation is pending, and the managed callback
deliberately does not require still-valid effect authority to record a stop after
revocation. Terminal or foreign runs cannot be modified through this callback.
None of these exceptions authorizes starting another request.

Older summaries load with an empty `provider_attempts` vector. Restart preserves
an `InFlight` observation as **outcome not yet recorded**, not a successful,
failed, or still-running provider request. Recovery does not replay it.

## Inspection and Helm

The public run and turns projections expose `provider_attempts` and an authored
`provider_attempt_summary`. Turn projection no longer silently drops summaries
past the former 1,024-turn limit. All durable attempts remain in the JSON; no
per-attempt eviction or truncation is performed. Existing storage/transport limits
still apply: capacity failure is not evidence that metadata was persisted or
retrieved, and this change does not add an unlimited snapshot transport.

Helm deserializes typed metadata and generates its own authored summary from the
phase, decision and numeric fields. Raw category/provider/model strings are not
rendered as diagnostic text. Saved attempts render at their turn boundary, live
attempts beside live output, and history without a loaded message anchor is
explicitly labelled. Provider failure is not a tool failure. Partial-response
stops advise reviewing saved output before explicitly submitting continuation;
this is not automatic replay or a claim that tool effects are safe to repeat.

## Verification boundary

Focused source tests cover protocol round-trip and secret-safe summaries,
revision-fenced persistence/reload and copy-on-failure, cancellation-time outcome
recording, terminal/wrong-run refusal, complete attempt projection and Helm text.
This isolated source-only delivery does **not** execute those tests, Cargo,
coverage, live providers or project binaries because the build/target lease is
held by other work (#72). The integrating parent must combine the agent trait and
retry implementation with this slice, run the required focused checks and workspace
coverage, and publish the measured result. No native-platform or live-provider
behavior is certified by source inspection or formatting checks.
