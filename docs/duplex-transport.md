# Helm–Vessel full-duplex transport

The active Helm transport is one authenticated WebSocket per connection activation:
`GET /v1/vessel/socket`, subprotocol `voyage.vessel.v1`. Literal-loopback discovery
uses WS; remote HTTPS origins upgrade to WSS with trusted certificate verification.
There is no silent HTTP command or SSE fallback when upgrade fails. Upgrade Helm
and Vessel together. HTTP pairing/discovery and the explicit public HTTP/SSE API
remain available for compatibility and non-Helm callers. The scoped gateway's
private adapter to the local supervisor is not a second Helm connection.

## Contract and authority

[`voyage-protocol::duplex`](../crates/voyage-protocol/src/duplex.rs) defines bounded
JSON text frames. A server `hello` identifies public protocol, Vessel UUID and a
fresh socket UUID. Socket request UUIDs correlate commands and replies; they are
**not** durable command IDs. A command contains the unchanged public `VesselRequest`,
and its reply contains `VesselResponse`. Private runtime command/response types
are not accepted on the public socket. Vessel still resolves the independent
voyage owner and maps public operations to private IPC.

Local upgrade authenticates the private per-service credential, rejects browser
Origin headers and accepts only literal loopback. Scoped upgrade authenticates the
existing grant and pinned Vessel identity. A socket does not authorize arbitrary
sessions, workspaces or browser actions: each operation uses the existing scope,
rights, revision, expiry and incarnation checks. Revocation/expiry closes idle
connections as well as stopping authorized publication and dispatch. Bytes already
delivered and effects already dispatched cannot be recalled.

Frames are limited to 4 MiB. In-flight requests, subscriptions and outgoing queues
are bounded: 32 command slots per socket, 64 executing commands and 64 concurrent
observation reads across sockets in each server process, and a 64 MiB serialized
outgoing-byte budget. Control/reply queues have priority over event queues. Each
socket admits up to 256 session subscriptions and retains at most 65,536 correlation
IDs; reaching a limit fails closed rather than evicting identities and accepting a
repeated request. Reconnect uses a fresh socket without resending earlier commands. Heartbeat and write/response deadlines terminate stalled connections;
a slow event consumer cannot grow an unbounded queue. Control replies and event
observations are independently scheduled. Metadata invalidations retain their
durable cursor. Event gaps and owner incarnation changes require canonical snapshot
reconciliation, not a reconstruction from transient transport frames.

## Connection and failure lifecycle

Cloned Helm clients share a socket keyed by stable route UUID and activation
generation. Separate Helm processes have separate authenticated sockets; neither
becomes canonical owner. A failed socket wakes its pending requests with uncertain
delivery, terminates event observations and publishes a connection-loss fence.
A later operation may establish a fresh socket, but **no pending command, terminal
input or reverse request is resent**. Recover uncertain effects with their exact
original durable command ID and payload and an authorized receipt query.

`Client::connection_state()` provides a watch receiver containing `socket_id` and
monotonic `loss_generation`. Consumers must compare both: watch coalescing can hide
an intermediate disconnected state, but cannot hide the loss counter. Explicit
`Client::disconnect()` permanently retires that activation without cancelling a
voyage. Reconnection of a retired route needs a new activation generation. This
is distinct from an explicit run cancellation command and from observed cleanup.

## Browser extension and private traffic

The same socket supports typed `ReverseRequest::BrowserWork` metadata and a correlated
`ReverseReply::{Accepted,Unavailable}`. It is a notification/acknowledgement extension,
not an arbitrary tool executor or a browser-specific remote endpoint. The single
`Client::reverse_requests()` consumer receives the exact socket UUID and a bounded
reply handle. Missing/dropped consumers refuse; expired replies cannot migrate to
a new connection. Browser pending/claim/result operations use ordinary typed public
commands on this same socket. Browser permission, local consent, action deduplication
and effect receipts remain the browser/runtime contract, not transport authority.

Connection loss fences local browser sharing even if ordinary reconnect succeeds;
it must not silently restore consent or replay a claimed action. The browser helper
is local beside Helm and is not a second remote connection, worker relay or pairing
service. Private PTY attachment remains human-only; multiplexing its public control
operations does not put private terminal input/output into conversation history.
No transport frame logging is introduced.

## Deployment and verification boundary

Socket connections use direct TCP/TLS, not HTTP forward-proxy environment variables.
A TLS reverse proxy must permit WebSocket upgrade and preserve Authorization,
`x-voyage-grant`, `x-voyage-vessel` and subprotocol headers. Disable caching and
buffering of the upgraded stream. Configure idle timeouts above the heartbeat
window. Do not expose the plaintext upstream listener, put credentials in URLs,
follow redirects or disable certificate verification to make upgrade work.
See [process access](process-access.md), [Vessel connections](vessel-connections.md)
and the remaining [deployment security gates](security.md#first-release-audit-2026-09-08).

Offline loopback evidence is not deployed WSS/proxy evidence, a paid-provider result,
or native macOS/Windows verification. Issue #235 tracks this transport; #234 owns
the browser integration and #198 retains public deployment security gates.

See [focused Linux verification](duplex-verification.md) for observed checks and
remaining integration/deployment boundaries.
