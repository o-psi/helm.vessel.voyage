# Provider failures and recovery

Voyage records provider attempts separately from canonical conversation messages
and tool outcomes. Vessel exposes those observations; Helm does not grant retry
authority. See [#261](https://github.com/o-psi/voyage/issues/261) and
[#269](https://github.com/o-psi/voyage/issues/269).

## Limits and classification

The defaults are eight **total attempts** per recovery group, an initial 1-second
retry ceiling, a 30-second maximum wait, and a 120-second retry admission window
starting at the first recoverable failure. The attempt budget includes the initial
request and any history-based continuation requests. A completed response ends the
group; a subsequent tool-result inference starts its own group. Exponential waits
use equal jitter (half to the full ceiling). Every actual retry has a new attempt ID
and inference admission; no adapter hides additional inference retries. Retries of
unchanged input share a logical request ID. Continuation from retained partial
history has a new request ID and links to the interrupted attempt.

`provider_response_timeout_ms` defaults to 60000. It bounds connection establishment
and waiting for the adapter to return a response stream, including preparation.
`provider_stream_idle_ms` defaults to 300000. It bounds waiting for the next decoded
provider event. Decoded activity resets that timer even when it produces no public
text: reasoning, provider ping and otherwise unrecognized valid JSON object events count.
Raw byte trickles and SSE comments do not count; malformed or non-object JSON is rejected.
Activity does not add reasoning or other private content to the conversation.
A healthy stream may last longer than 120 seconds before its first failure and
still recover. Once started, the recovery window only prevents another attempt;
it is neither a turn deadline nor a stream timer.
All three time limits are positive milliseconds; shell `command_timeout_secs` is
separate. On Linux, Android and Fuchsia, native clients do not install reqwest's
implicit 30-second `TCP_USER_TIMEOUT`; the operating system retains its normal TCP
failure handling. This avoids preempting configured application waits during
unacknowledged traffic. Connection/response and decoded-event waits remain bounded
by Voyage and cancellable. This is not a guarantee that the network or upstream
will keep a connection alive until those limits.

New native transport timeouts are recorded as `transport_timeout`, separately from
Voyage's `timeout` deadline category. Legacy `timeout` observations cannot establish
which timer fired. Helm labels response-start, stream-idle, retry-delay and retry
admission limits separately; retry delay is not an active stream deadline.

Policy and authority are rechecked at least every 100ms while awaiting
provider progress or backoff. Cancellation stops waiting and enters normal cleanup;
it is not proof the provider stopped computing or that no usage was billed.

Timeouts, transient rate limits and service unavailability are eligible for bounded
recovery. Typed native connection-establishment failures (including DNS, TCP, proxy
negotiation and TLS handshake failures) are also eligible: the provider HTTP request
has not been sent through that failed connector. Permanent DNS/certificate/proxy
errors may still exhaust the bounded budget; TLS verification is never weakened.
OAuth token exchange retains its stricter uncertain-rotation handling. After a response
stream starts, typed connection reset/abort, broken pipe, unexpected EOF, incomplete
HTTP messages and premature clean EOF are recoverable stream interruptions. Generic
transport errors remain uncertain and nonretryable; dispatch failures are not made
retryable merely because they contain an arbitrary I/O error. A quick failure or
absence of response text/tool fragments does not prove the POST was unsent. An
uncertain transport stop advises reviewing saved tool outcomes before continuing,
not changing provider configuration blindly. Request construction
errors, authentication, account quota and malformed responses do not authorize
retry. Explicit context rejections use bounded working-context recovery without
replaying completed tools; a rejection after partial output cannot use that path
to replay the interrupted request.

Recovery uses native HTTP/SSE adapters. There is no provider WebSocket transport or
WebSocket-to-SSE fallback. Helm's control connection to Vessel is separate from
Voyage's provider connection. The independent Voyage process owns recovery and
credentials; disconnecting Helm does not cancel it.

`Retry-After` supports seconds and HTTP dates. The runtime never shortens a valid
server delay. A delay beyond the maximum wait or remaining admission window stops
with a distinct recorded reason. New defaults do not override explicit saved
configuration in existing voyages.

## Durable observations and inspection

The shared [ProviderAttempt contract](../crates/voyage-protocol/src/provider_attempt.rs)
records local logical-request and attempt IDs, provider/model, ordinal/limit,
start time/duration, phase, output progress, safe category/status/code, effective
timeouts/retry limits, server delay, selected/measured wait and the stop decision.
Defaulted recovery fields record the previous interrupted attempt (`recovery_of`)
and the absolute admission deadline (`recovery_deadline_at_ms`). The deadline is
absent until the first recoverable failure and cannot then move. A continuation
link must name an earlier scheduled interruption in the same run, with a different
request ID and the same provider/model. Legacy absent fields remain unknown.
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
runs cannot be changed. Restart reconciliation marks the latest pending in-flight,
retry or continuation observation `recovery_interrupted`; earlier scheduled attempts
remain history. This records lost execution ownership, not a surviving request or
permission to dispatch replacement inference. Resuming execution after process
interruption requires an explicit later message and normal admission/reconciliation.
Legacy runs have no fabricated attempt history.

Managed checkpoint workers tolerate brief SQLite lock contention, including
Vessel catalogue reads of the same journal. A shared two-second deadline bounds
lock waits within each blocking storage callback, beginning after the owner mutex
is acquired. SQLite retries the pending statement; the callback, provider request
and tool execution are not replayed. Cancellation interrupts lock waits. Canonical
history, working-context and acceptance waits also recheck foreground grant
authority; outcome metadata can still be recorded after revocation. Other journal
callers retain their nonblocking behavior. This is not a two-second limit on the
entire callback or on mutex acquisition.

Persistent contention or other errors still stop the run. When terminal persistence
succeeds, the run retains an authored checkpoint operation and category (`database
busy`, `storage error`, or `state or authority validation failed`), shown by Helm
and Vessel inspection. Raw SQLite messages, paths and grant diagnostics are never
included. An inability to save the terminal record can still prevent durable
diagnostics; older failures retain their original generic label. These labels do
not retroactively establish the cause of an earlier failure.

Helm's `/attempts` reads history without starting a run. Use `/attempts RUN_UUID`
to inspect one run, or follow the displayed `/attempts all OFFSET` or
`/attempts RUN_UUID OFFSET` continuation. Each view reads at most 16 records.
The model-facing Vessel tool also exposes a `provider_attempts` action with
revision-bound `next_read` pagination. The Vessel `provider_attempts` operation accepts `run_id`, `offset`, `limit` (1–32),
and an optional `expected_revision` fence, under history-read authority, including
while the voyage is suspended. A stale revision is refused; restart inspection
from offset zero. Helm page reads are fresh observations, so a running attempt
may have changed between pages. Scheduled reconnection and continuation summaries
show the selected delay and the recovery budget at that observation. The live
heading distinguishes reconnecting to the provider from continuing an interrupted
response; terminal states clear those active labels. They do not
claim a live countdown. `/attempts` exposes continuation lineage and the recorded
admission deadline; saved scheduling observations do not establish liveness after
restart.

Helm acceptance notices say “Request received.” They acknowledge submission and do
not claim the run is still executing. Terminal inventory that is absent, stale or
unavailable is labelled as such, including after suspension; it does not claim an
active “Checking programs” operation.

Routine snapshots retain the latest 128 turn summaries and the latest attempt per
turn, with explicit counts/truncation. Full durable attempt history remains available
through paging. Successful attempts stay quiet in the transcript; failed attempts
show authored summaries and a history hint. The inspection view does not render raw
provider-controlled error strings. Capacity or persistence failure is not evidence
that a diagnostic record was saved.

## Safe continuation

On a recoverable failure after text or a tool-call fragment, Voyage schedules a
bounded continuation within the same run and original user command. Before the
next request, it checkpoints the safely retained text as a distinct assistant
segment carrying `interrupted_attempt`, displayed by Helm as “Interrupted response”. A tool-only interruption retains an empty
segment with that identity. The next request is built from current canonical
history and working context, with runtime guidance to continue the original task
without repeating retained text or completed operations. That guidance is not a
new canonical conversation message. Generated continuation text is a separate
segment; the runtime does not join it to the interrupted answer or deduplicate
prose by string matching.

No tool-call fragment from the interrupted response is admitted for execution.
Completed tool results from earlier requests remain in history, and runtime
recovery does not replay those calls. A withheld possible secret suffix is not
flushed on stream failure. Checkpoint or accounting failure stops recovery before
another dispatch. Policy, cancellation and inference budget are checked again for
each attempt; changing retry limits does not authorize uncertain external effects.
Unknown tool effects still require reconciliation, and model instructions are not
a guarantee of exactly-once effects.

Attempt exhaustion, elapsed admission budget, nonretryable errors, cancellation or
revocation stop automatic recovery and retain available output. The user can review
saved output, tool results and unresolved cleanup, then send a **new message** to
continue the same voyage. That creates a new run/command with earlier history;
repeating the original command ID returns its original receipt without dispatch.

## Verification

Focused Rust tests cover error classification, retry exhaustion and bounds,
cancellation/revocation, bounded partial continuation, stream activity, checkpoint
failures, durable identity/reload and bounded protocol/UI projection. The offline
native fixture uses actual Vessel-supervised processes and scripted Responses, Chat
Completions and
Anthropic endpoints:

```sh
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/provider_attempts.py --bin-dir target/debug
python3 voyage/tests/provider_failure_ui.py --bin-dir target/debug
python3 voyage/tests/provider_restart_recovery.py --bin-dir target/debug
```

The fixture provisions named synthetic accounts and private launch settings; it never
uses the operator's account. The failure-screen PTY check submits through Helm, waits
for bounded partial-output recovery exhaustion and suspension, and checks truthful
receipt and inventory notices. Its three provider requests remain within one run.
The separate owner-death fixture kills its exact owned process during backoff,
uses ordinary recovery without cleanup attestations, then restarts it. It requires
an interrupted diagnostic and no automatic inference after the old delay, followed
by successful explicit continuation in a new run.

The 66-case matrix covers all three adapters: refused connections restored at the
same endpoint, transient recovery, exhaustion,
rejection/quota, server delay limits, truncated and idle streams, silent response
headers, comment-only streams, cancellation, metadata paging, exact duplicate
receipts, explicit later continuation and observed cleanup. Activity-only progress
must outlast the idle threshold and complete without publishing reasoning. Partial
text/tool recovery must use linked new request IDs and distinct retained segments.
A completed file-write tool followed by interruption must retain its result and
leave the file unchanged through recovery. It also opens Helm in a real PTY and
checks `/attempts` without dispatching inference. Logs and synthetic request evidence
remain in its printed private temporary directory. This is Linux/offline evidence, separate from workspace
Rust coverage. No live-provider or native macOS/Windows result is implied. See
[quality](quality.md) for the coverage and other applicable checks.
