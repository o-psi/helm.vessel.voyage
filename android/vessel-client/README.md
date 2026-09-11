# Native Kotlin Vessel client (#260 workstream A)

Kotlin/JVM 17, package `dev.helm.vessel`; Kotlin 2.1.20,
kotlinx-serialization-json 1.8.1, coroutines 1.10.2, OkHttp 4.12.0.
No JNI/shared Rust UI, embedded runtime, provider credentials, browser executor
or notification backend. The Android app supplies Room/Keystore and lifecycle policy.

## Wire authority

- [Public commands/replies](../../crates/voyage-protocol/src/vessel.rs)
- [Duplex frames](../../crates/voyage-protocol/src/duplex.rs)
- [Scoped socket authentication](../../vessel/src/process_http.rs)
- [Desktop transport](../../helm/src/process_client/duplex.rs)
- [Snapshot/history/chunks](../../voyage/src/attachment/runtime/process.rs)
- [Invalidation cursors](../../voyage/src/attachment/journal/observations.rs)
- [Receipts](../../voyage/src/attachment/journal/process.rs)

`Requests` builds `{protocol:1,command:{op,...}}`. Voyage operations are
flattened, never private RuntimeRequest envelopes. Socket `request_id` is fresh
per exchange and is **not** durable `command_id`. For voyage operations,
`VesselResponse.result` contains `{session_id,incarnation,result}`;
`voyageResult(sessionId)` validates that wrapper. Catalogue/capabilities/inspect
have their own shapes and must not be unwrapped as voyage replies. JSON
projections preserve fields outside the typed subset. Missing `outcome_unknown`
defaults to true. Fixtures are examples transcribed from Rust contracts, not
captured private sessions or proof of a deployed Rust backend connection.

## Connection, reads and observation

```kotlin
val endpoint = VesselEndpoint(
    "wss://vessel.example/v1/vessel/socket", vesselId, grantId, scopedToken)
val client = VesselClient(endpoint, roomJournal)
client.connect()
val catalogue = client.read(Requests.catalogue())
val canonical = client.reconcile(sessionId)
```

Supply an existing scoped grant, non-nil UUID identities and 64-hex credential.
Only WSS at the exact socket path is accepted: no URL credentials/query/fragment,
TLS bypass, insecure fallback, redirect, command retry or pairing. Platform TLS
trust and hostname verification remain active. Both `x-voyage-vessel` and hello
must match persisted Vessel identity. This UUID pin supplements TLS; it is not
certificate/SPKI pinning. Diagnostics omit credentials, request payloads and
server results. Applications must not log token/JSON properties. Production
transport does not accept a custom HTTP client; internal TLS injection is for tests.

`state: StateFlow<ConnectionState>` has `socketId` and monotonic `lossGeneration`.
Mark caches stale on loss/change, including coalesced reconnects. There is no
background reconnect: explicitly `connect()`, recover pending receipts, reload
catalogue/selected snapshot and resubscribe. `close()` permanently closes an
activation; use a new client after close. It never sends cancel/stop or deletes
journal entries.

`Requests` supports capabilities, catalogue, inspect, snapshot, revision-pinned
history, message chunks, run-output chunks, events, decisions and receipts.
History/event pages allow at most 128 items; events wait at most 10 seconds;
message/output chunks allow 65,536 **UTF-8 bytes**, not Kotlin character indices.
Follow server `next_offset`/`has_more`; truncated projections are not full canonical
messages. This module does not concatenate unlimited output/history in memory.

`observe(List<EventSubscription(sessionId,incarnation,after)>)` is a bounded cold
Flow of `VesselEvent(sessionId,incarnation,result)`. Collection subscribes;
cancellation unsubscribes. Events are **invalidations**, not assistant text.
On events/replay gaps, mark stale and reload canonical data. Read run output for
provisional text, replacing it with canonical history after reconciliation.
`reconcile(sessionId,historyOffset=0,historyLimit=64)` returns recent snapshot plus
one history page from the same revision/incarnation/connection and the snapshot
observation cursor. Revision/connection races fail rather than presenting stale
data as current. Subscribe from that cursor to replay changes during the read.
A retention gap requires a fresh snapshot.

## Mutation journal and recovery

`Mutation.submit`, `.steer`, `.cancel`, `.respond` require a durable command UUID
per explicit intent, observed revision and absolute Unix-millisecond expiry.
Active-run operations also require observed incarnation/run identities.
`respond` adds decision UUID and exact public response JSON: approvals use
`"approved"`/`"denied"`; question selection uses
`{"status":"selected","index":0,"answer":"exact option"}`. Questions are not
authorization. Server rights/current decision state remain authoritative.

The adapter implements these suspend methods:

```kotlin
insert(entry: JournalEntry): Boolean
record(commandId: String, state: JournalState, responseJson: String?)
pending(vesselId: String): List<JournalEntry>
```

`JournalEntry` fields: `vesselId`, `sessionId`, `commandId`, `exactRequest`,
`state=UNCERTAIN`, `responseJson=null`. Exact request is the serialized public
VesselRequest, excluding socket correlation/credentials. Adapter requirements:

1. Atomically **commit durably before returning true**, unique by commandId.
2. Return false only for identical vessel/session/exact payload bytes. Throw on
   mismatches; never overwrite intent or refresh expiry.
3. Atomically update observation fields only; never regress RESOLVED to UNCERTAIN.
   Retain resolved entries for deduplication.
4. Use private storage without backup/logging. Bound pending reads (recommended
   128 entries) rather than loading an unlimited journal.

`mutate` commits UNCERTAIN before send. Cancellation/storage failure/process death
can leave uncertainty even when no bytes were sent. Existing intents are never
redispatched: repeated `mutate`, `recover(entry)` and `recoverPending(limit=32)`
use receipts only. Unknown/refused/unavailable receipt is not permission to retry.
No `resolve(original)` or blind replay path exists. Receipt identity must match
saved session/command before uncertainty resolves.

RESOLVED means admission/refusal observed, **not execution or cleanup complete**.
Accepted submission, requested cancellation and queued steering still require
observation. Present server status separately from local journal state. Failure
to save a reply can leave UNCERTAIN; recover through receipts when storage returns.

## Bounds and remaining acceptance gaps

- 32 in-flight requests, 4 MiB outgoing queue/application inbound message limit,
  64-level JSON nesting limit, 32 subscription groups, 256 sessions per request.
  Event queues have eight items and a 4 MiB aggregate per-socket byte budget.
- Request/hello deadline 15 seconds; TLS connect timeout eight seconds; ping
  interval five seconds. Failure fences the socket and fails pending work.
- **Hard-read-bound gap:** OkHttp 4.12 aggregates inbound WebSocket messages before
  `onMessage`; no public pre-aggregation size limit is exposed. Application
  rejection above 4 MiB cannot prevent larger allocation from a malicious trusted
  peer (including fragmented/compressed messages). This is not equivalent to the
  Rust bounded reader. Keep #260 open for this hardening acceptance gap.
- No terminal attachment, uploads, QR pairing, push backend, iOS/browser shell,
  provider/configuration management or automatic catalogue polling. Unsupported
  reverse browser requests receive `unavailable`.
- JVM TLS fixtures do not establish deployed scoped WSS/revocation, Android
  lifecycle, native device networking or Keystore behavior. B owns those checks.

## Verification

From `android/` with B's root toolchain:

```sh
JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :vessel-client:test
```

On 2026-09-11, Linux x86_64, OpenJDK17, Gradle8.11.1 against B root commit
`8e9978f`: `:vessel-client:test --console=plain --no-daemon` exited 0;
**21 tests passed**, zero failures/errors/skips (12 client/journal/protocol,
9 TLS/socket tests). Tests cover wire fixtures, durable-vs-socket identity,
trusted/untrusted TLS, identity mismatch, size/nesting rejection, subscriptions,
reverse refusal, deadline/loss, durable-before-send, storage failure,
restart receipt recovery, immutable intent collisions, unknown/mismatched
receipts, reconciliation, read/mutation separation and diagnostic redaction.
Earlier JUnit signature/test teardown failures were fixed before the final run.
Reports/logs remain in ignored `build/`. No Rust changed and no Rust coverage
claim is made. A did not run device/emulator/deployed Vessel verification;
consult B's delivery evidence for subsequent native checks. README source links
and `git diff --check` are checked separately.
