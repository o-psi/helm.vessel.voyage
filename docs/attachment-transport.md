# Authenticated attachment socket libraries (partial #9)

These libraries connect Helm outbound to Vessel using the version-2
[event/replay codec](attachment-events.md) and existing single-owner enrollment.
They are dependencies for the full [production contract](attachment-production-contract.md).
Neither production binary mounts the attachment route or exposes operator session
execution through these libraries. Enrollment HTTP and its existing administration
remain separate. No inbound Helm task port, provider credentials, raw Session,
runtime system instructions or human PTY input are introduced.

## APIs and authority boundaries

`vessel::attachment_transport::AttachmentApi::new(enrollment, supported)` returns
an API and bounded receiver of `AuthenticatedFrame` observations. `router()`
constructs only `/v2/attachment`; a caller must explicitly mount that router.
`send(machine_id, frame)` validates direction, connection/machine/owner identities,
current enrollment and selected features before enqueueing. The socket checks
current enrollment and generation again before dequeue delivery. Only Command and
ReplayRequest are accepted from this sending API. Command principals must equal
the enrolled owner. This is transport binding, not a local policy or sharing grant.

The observation receiver carries connection, owner, machine, epoch and frame.
Call `is_current(machine_id, connection_id)` immediately before publishing queued
observations, then check the current sharing projection and local coordinator's
requirements at actual disclosure or effect commitment. A queued observation can
outlive its socket and is not proof of durable acceptance, successful execution,
canonical checkpoint persistence or process cleanup. Command expiry in the codec
is structural here; actual admission deadlines and authorized duplicate lookup
belong to the authoritative local coordinator. Never use a received frame or a
cached positive transport check as permission to execute a tool.

Helm's `attachment::transport::connect(client, offered, cancel)` owns an enrolled
`EnrollmentClient`, obtains a fresh Connect proof and returns a `Connection` with
context, bounded `send`/`receive`, activity checks, and explicit close/detach.
The connection and worker retain the enrollment file lease for their lifetime.
Sending succeeds only into a bounded transport queue. Neither side automatically
replays queued commands after reconnect; a caller must explicitly reconnect with
a fresh proof and resolve uncertain durable outcomes through the coordinator.

## Authentication, control frames and limits

Upgrade requires the exact enrolled Origin, `x-voyage-request: 2`, and exactly one
`Sec-WebSocket-Protocol: voyage.attachment.v2`. Queries, URI authority/userinfo,
missing or duplicate required headers and unsupported/multiple protocols fail
closed. Helm derives the socket URL from the enrolled origin and uses its outbound
client transport; production TLS belongs to that configured origin and deployment.
A reverse proxy must preserve the authenticated origin boundary; forwarded host
headers do not define it.

The first application message must be TEXT Authenticate within five seconds.
Enrollment consumes the signed Connect challenge exactly once before Welcome.
Welcome binds a fresh connection ID to the enrolled machine, owner, current epoch
and selected feature intersection. Features remain wire support, never capability
grants. A lost Welcome is an uncertain delivery outcome: the consumed proof cannot
be reused. Valid replacement cancels the previous connection generation. Old
socket cleanup cannot remove its successor, and queued old data must fail the
consumer's current-generation check.

The default authorization lease is 10,000 ms. Only a valid, current-epoch heartbeat
renews it with a Lease frame. A separate bounded monitor checks enrollment at most
one second between scheduled checks and expires idle connections independently
of blocked writes. Enrollment-check failure or timeout fails closed. All socket
writes have a two-second deadline. The test seam uses a valid 1,000 ms lease and
advertises precisely the enforced duration; authentication and write timeouts are
independently shortened in tests. Ping/Pong are valid WebSocket control frames
before and after authentication. Pong writes are bounded; Ping/Pong never extend
the authentication deadline or authorization lease. Binary application messages,
malformed frames and invalid frame direction close the connection.

Vessel admits at most 32 sockets including unauthenticated handshakes and 16
connected machines. Its outgoing queue is 16 frames per connection and its
observation queue is 16 frames globally. Helm has eight-frame queues in each
direction. Both byte decoding and WebSocket frame/message size are capped at
256 KiB; the socket write buffer is bounded. Saturation returns a fixed failure
or closes the affected connection, never allocates an unbounded backlog. The
Vessel send API reports queue saturation; a full observation queue closes that
socket. The Helm send queue closes on saturation. Fixed transport errors exclude
proofs, frame contents and raw network/storage diagnostic strings.

Revocation is bounded invalidation, not instantaneous recall of bytes already
transmitted. Current checks before dequeue and cancellation fencing cannot make
socket I/O atomic with a separate durable policy transaction. The local coordinator
must independently enforce current authorization at its effect boundary. Likewise,
only an authorized complete event range can use the global replay cursor: the
[filtered-range refusal rule](attachment-events.md#global-cursors-and-filtered-projections)
still applies. This library does not implement that policy projection or durable
snapshot recovery.

## Evidence and remaining integration

Vessel's real loopback socket tests cover exact upgrade headers, proof replay,
genuinely expired server-MAC/client-signed challenges, replacement, revocation,
lost Welcome and fresh-proof recovery, stale observations, authentication and idle
timeouts, malformed/binary/oversized/unsupported-feature input, direction and owner
binding, machine/socket capacities, queue saturation, and Ping/Pong without lease
or authentication extension. A deterministic stalled-write seam tests write
failure, deadline and cancellation without relying on an OS TCP buffer size.
The bounded queue test inspects a stalled receiver; it does not claim to reproduce
every production slow-peer network condition.

Real client/server fixtures compile on Unix and Windows with the private enrollment
storage implementation. Native platform results must be observed on this integrated
branch before claiming coverage.
Protocol and library tests do not prove proxy/TLS deployment, production routing,
operator access, local execution coordination, sharing/approval dispatch, complete
replay recovery, frontend behavior or service lifecycle. Those acceptance criteria
remain open under #9 and its related production-contract issues.


Third-party WebSocket dependencies log raw frames and close payloads at Debug and
Trace. Both binaries therefore compile the `log` facade with `max_level_info`;
this caps all third-party log-facade Debug/Trace diagnostics in debug and release
builds. Application `tracing` remains independently configurable. The socket tests
enable application TRACE and send proof/prompt/history canaries, verifying that
application trace output remains visible while those payloads do not appear.
Transport errors never include raw dependency error text. Buffer controls follow
the [Axum WebSocket API](https://docs.rs/axum/0.8.9/axum/extract/ws/struct.WebSocketUpgrade.html)
and [Tungstenite configuration](https://docs.rs/tungstenite/0.29.0/tungstenite/protocol/struct.WebSocketConfig.html).
