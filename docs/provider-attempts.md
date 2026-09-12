# Provider failures and recovery

Voyage records provider attempts separately from canonical conversation messages
and tool outcomes. Vessel exposes those observations; Helm does not grant retry
authority. See [#261](https://github.com/o-psi/voyage/issues/261).

## Limits and classification

The defaults are eight **total attempts** per logical inference request, an initial
1-second retry ceiling, a 30-second maximum wait, and a 120-second elapsed retry
admission window. Exponential waits use equal jitter (half to the full ceiling).
Every actual retry has a new attempt ID and inference admission; no adapter hides
additional inference retries. The logical request ID groups attempts.

`provider_response_timeout_ms` defaults to 60000. It bounds connection establishment
and waiting for the adapter to return a response stream, including preparation.
`provider_stream_idle_ms` defaults to 300000. It bounds waiting for the next decoded
provider event. Decoded text, tool, usage or response metadata events reset that
idle timer; raw byte trickles, SSE comments/heartbeats and unrecognized events do
not. A healthy stream may last longer than the retry admission window. That window
only prevents another attempt, and is neither a turn deadline nor a stream timer.
All three time limits are positive milliseconds; shell `command_timeout_secs` is
separate. Policy and authority are rechecked at least every 100ms while awaiting
provider progress or backoff. Cancellation stops waiting and enters normal cleanup;
it is not proof the provider stopped computing or that no usage was billed.

Timeouts, transient rate limits and service unavailability are eligible for bounded
retry before output. Native connection refusal/network-unreachable errors identified
through the transport's connection and I/O error chain are also eligible. Other
transport errors remain uncertain and nonretryable. Request construction errors,
authentication, account quota, malformed responses and clean premature EOF are not
transient retry permission. Explicit context rejections use the existing bounded
working-context recovery path, without replaying completed tools.

`Retry-After` supports seconds and HTTP dates. The runtime never shortens a valid
server delay. A delay beyond the maximum wait or remaining admission window stops
with a distinct recorded reason. New defaults do not override explicit saved
configuration in existing voyages.

## Durable observations and inspection

The shared [ProviderAttempt contract](../crates/voyage-protocol/src/provider_attempt.rs)
records local logical-request and attempt IDs, provider/model, ordinal/limit,
start time/duration, phase, output progress, safe category/status/code, effective
timeouts/retry limits, server delay, selected/measured wait and the stop decision.
The current provider-code allowlist covers explicit account quota and context-length
codes; other provider-controlled code strings are omitted.
An upstream request ID, when available, is a separate bounded validated field;
it is never interchangeable with a local request ID. Received HTTP status survives
later stream failure. Credentials, arbitrary headers, response bodies and private
error strings are excluded. Known secrets are removed or the identifier is omitted.

A durable intent is required before dispatch. The same attempt is updated at
response start, first output, outcome and backoff completion. Checkpoint failure
never authorizes dispatch or another retry. Persistence uses the current owner's
journal transaction and revision fences. Cancellation or revoked effect authority
still permits the owning running run to record why it stopped; terminal/foreign
runs cannot be changed. A record left in flight after process death means **outcome
not recorded**, never that a request survived restart. Recovery does not replay it.
Legacy runs have no fabricated attempt history.

Helm's `/attempts` reads history without starting a run. Use `/attempts RUN_UUID`
to inspect one run, or follow the displayed `/attempts all OFFSET` or
`/attempts RUN_UUID OFFSET` continuation. Each view reads at most 16 records.
The model-facing Vessel tool also exposes a `provider_attempts` action with
revision-bound `next_read` pagination. The Vessel `provider_attempts` operation accepts `run_id`, `offset`, `limit` (1–32),
and an optional `expected_revision` fence, under history-read authority, including
while the voyage is suspended. A stale revision is refused; restart inspection
from offset zero. Helm page reads are fresh observations, so a running attempt
may have changed between pages.

Routine snapshots retain the latest 128 turn summaries and the latest attempt per
turn, with explicit counts/truncation. Full durable attempt history remains available
through paging. Successful attempts stay quiet in the transcript; failed attempts
show authored summaries and a history hint. The inspection view does not render raw
provider-controlled error strings. Capacity or persistence failure is not evidence
that a diagnostic record was saved.

## Safe continuation

Once any text or tool-call fragment appears, this runtime stops automatic retry of
an interrupted response. It does not execute partial tool arguments. It retains
available public output and completed canonical history. A withheld possible secret
suffix is not flushed on stream failure. Review the saved output, completed tool
results and unresolved cleanup, then send a **new message** to continue the same
voyage. That creates a new run/command and retains earlier history; repeating the
original command ID returns its original receipt rather than dispatching again.

Automatic post-output continuation would require a distinct request built from a
verified retained-history boundary, reconciled effects and explicit treatment of
partial output. This implementation does not claim those conditions from a text
or tool fragment alone. Unknown effects require reconciliation; changing retry
limits does not make them safe to repeat. Codex's history-based recovery is not
permission to remove this guard or a proof of exactly-once external effects.

## Verification

Focused Rust tests cover error classification, retry exhaustion and bounds,
cancellation/revocation, partial-output refusal, checkpoint failures, durable
identity/reload and bounded protocol/UI projection. The offline native fixture uses
actual Vessel-supervised processes and scripted Responses, Chat Completions and
Anthropic endpoints:

```sh
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/provider_attempts.py --bin-dir target/debug
```

It checks transient recovery, exhaustion, rejection/quota, server delay limits,
truncated and idle streams, silent response headers, heartbeat-only streams,
cancellation, metadata paging, exact duplicate receipts, explicit continuation and
observed process cleanup. It also opens Helm in a real PTY and checks `/attempts`
without dispatching inference. Logs and synthetic request evidence remain in its printed
private temporary directory. This is Linux/offline evidence, separate from workspace
Rust coverage. No live-provider or native macOS/Windows result is implied. See
[quality](quality.md) for the coverage and other applicable checks.
