# Live events integration and rollout matrix (#372)

Status: acceptance matrix for #366; the plan alone is **not passing-test evidence**. Run against the
integrated public-v2 source and deployed WebHelm revision; record actual commands,
results and source identities in #372. This document does not define the wire
contract or modify the clients. Contract changes belong to #366/#367–#371.

## Baseline and contract dependencies

At baseline `origin/main` `7657ef1`, the public Vessel API is v1. Its event
subscription carries session ID, incarnation and `after`; the result contains
metadata-only `public-v1` observation events, with 2,048 retained events. On a
replay gap, clients must hydrate an authorized snapshot. The plain Helm client
currently refreshes a snapshot after each event. These are **baseline behavior**,
not evidence for the proposed content stream. See
[`crates/voyage-protocol/src/vessel.rs`](../crates/voyage-protocol/src/vessel.rs),
[`docs/current-state.md`](current-state.md) and
[`helm/src/process_client/plain/session.rs`](../helm/src/process_client/plain/session.rs).

Before writing cross-client fixtures, settle and publish in the shared contract:

- How a client opts into public-v2 over SSE and duplex, how support is advertised,
  and how a v1 peer rejects or falls back without parsing v2 as v1. Keep existing
  v1 requests and subscribers working.
- The typed envelope's session/incarnation, ordered replay cursor, revision,
  stable run/message/tool/decision/command identities, payload size limit and
  whether content is provisional delta, committed text, or a reference requiring
  a bounded read. Define duplicate, reordered, omitted and retained-gap handling
  without confusing sequence, revision, and process incarnation.
- Replay retention and slow-consumer policy: bounded queues/pages, explicit gap
  indication and one safe resynchronization path. Restart must not assert that
  transient content survived unless it was durably recorded.
- Privacy projection of terminal/browser private input and authorized output;
  no client should infer permission from an event or use it to replay a command.

## Acceptance matrix

All fixtures use synthetic providers and isolated data roots. A passing assertion
must name the observed source, subscription type, cursor range and canonical
snapshot/revision; compile-only evidence is insufficient.

| Scenario | Required oracle | Intended verification surface |
| --- | --- | --- |
| Initial hydrate then live content | One authorized initial snapshot, ordered bounded deltas/commits matching final canonical history; no snapshot per ordinary event | Integrated Voyage→Vessel→TUI/Web fixture and per-client tests |
| Disconnect/reconnect | Both clients can drop transport and resume from their own cursor without re-admitting work; duplicates deduplicated by identity | Extend offline `voyage/tests/client_reconnect.py` and client tests |
| Duplicate, missing, reordered events | Repeated cursor does not double text/tool rows; out-of-order/gap does not silently advance state; one bounded hydrate or explicit failure | Deterministic stream injection in each client; server replay fixture |
| Runtime or Vessel restart | Session identity survives, incarnation change is detected, durable events replay or explicit gap forces snapshot; command receipts remain exact | Isolated supervised-process restart fixture and both clients |
| Slow client/retention overflow | Producer stays bounded; fast peer continues; slow peer observes a gap and recovers by snapshot without replaying commands | Stream buffering limits and isolated two-subscriber fixture |
| Long conversation | Bounded frame/payload and memory; paged older history remains available; new events converge without retrieving all history each update | Large synthetic transcript, paged-read and client rendering tests |
| Simultaneous TUI and Web | Independent subscriptions receive same canonical terminal state; one disconnect does not cancel or stall the other | Linux TUI process + Web browser client against one Vessel/session |
| Version compatibility | v1 client↔v2 server unchanged; v2 client↔v1 server negotiates/falls back explicitly; incompatible opt-in is refused, not ambiguously accepted | Wire and client fixture matrix for SSE and duplex |
| Privacy and authority | No private terminal/browser input appears in v2 payload, logs or public snapshots; revocation terminates subscriptions and commands remain exact-once | Redacted synthetic fixture and scoped-grant tests |
| Rollout and installation | Core delivered SHA included in actual hosted build/artifact; WebHelm deployed SHA observed independently, authenticated console loads correct assets | Nightly run and archive identity; Web update receipt, `/up`, landing and authenticated console |

Existing focused checks include `voyage/tests/client_reconnect.py`,
`voyage/tests/delivery_recovery.py` and `helm/src/process_client/duplex_tests.rs`.
They are starting points, not substitutes for v2-specific assertions. Follow
[`docs/quality.md`](quality.md) and workspace coverage instructions after final
Rust edits. Do not run concurrent Cargo builds or publish dirty-tree coverage as
clean source. Browser authentication/deployment requires observing the actual
production revision: a Git push and `/up` response do not prove what is deployed.

## Recording rollout evidence

Record on #372: integrated source SHA(s), exact local commands and outcomes,
fixture result/cursor and any failures, workspace coverage summary, hosted build
run/artifact identity when required, WebHelm source and deployment receipt, and
explicit Linux-only or browser/manual limitations. Close only after all required
oracles have evidence; leave unresolved rows open with a concrete next action.

## Current fixture coverage

The public-v2 process fixture now includes catalogue cursor checkpoint/paging,
retained journal replay across Vessel restart, fresh Voyage incarnation and repeated
one-event pages. Its optional `--long-history` mode exercises real event retention
overflow and paged older history with independent fast and delayed read cursors.
The real TUI/Web journey in `goals.py --only web` exercises partial response display
in both clients, Web-only transport loss while TUI continues, and automatic Web
recovery to the same completed canonical run without duplicate text or inference.
See [quality commands](quality.md#ordered-event-replay-and-reconnect).

These fixtures complement deterministic client reducer cases for malformed,
duplicate, out-of-order, wrong-incarnation and missing-offset events, and the Vessel
bounded-queue slow-consumer test. Retention overflow is not a measurement of socket
congestion, browser memory, or rendering a large history. The deployed authenticated
console and remaining adverse/large-history client journeys still need explicit
results in #372; do not close the matrix from process or component evidence alone.
