# Scoped Helm web gateway (#289)

A standalone Node >=22 service. It is **not** a Voyage executor and never calls
private runtime IPC. See [Helm Web deployment](../../docs/helm-web.md) for the
Laravel integration. Install/run:

```sh
cd web/gateway
npm ci --ignore-scripts
npm test
npm start
```

## Configuration

| Variable | Meaning |
| --- | --- |
| `HELM_WEB_GATEWAY_SECRET` | Required shared Laravel/Node HMAC secret, raw UTF-8 string of at least 32 bytes; not base64-decoded |
| `HELM_WEB_ORIGIN` | Required exact browser origin, e.g. `https://helm.example.com`; no slash/path |
| `HELM_WEB_VESSELS_FILE` | Required absolute path to private JSON configuration, owned by gateway UID, regular file, no symlink, mode 0600 or stricter |
| `HELM_WEB_ALLOW_LOOPBACK_WS` | Set exactly `1` to allow plaintext **literal** `127.0.0.1` or `[::1]` upstreams; otherwise only `wss:` |
| `HELM_WEB_GATEWAY_HOST` | Default `127.0.0.1` |
| `HELM_WEB_GATEWAY_PORT` | Default `8787` |

The private JSON object maps aliases to exactly these fields (replace placeholders):

```json
{
  "local": {
    "url": "wss://vessel.example.com/v1/vessel/socket",
    "token": "PRIVATE_SCOPED_GRANT_TOKEN",
    "grant_id": "00000000-0000-4000-8000-000000000001",
    "vessel_id": "00000000-0000-4000-8000-000000000002"
  }
}
```

Use a least-privilege Vessel grant. Each upstream connection sends
`Authorization: Bearer <token>`, `x-voyage-grant`, `x-voyage-vessel`, and WebSocket
subprotocol `voyage.vessel.v1`. TLS verification remains enabled; no redirects,
URL credentials, query, fragment or alternative upstream path are permitted.
Configuration is loaded once at startup. Never put this file under public web
storage or commit real credentials. Linux file permissions were tested; native
Windows ACL security is not established.

Terminate public browser TLS at the same-origin reverse proxy and route its
`/socket` upgrades to this service, preserving Origin. No forwarded-origin headers
are trusted. There is no HTTP API or health payload; ordinary HTTP gets 404.
Keep this listener private. Reverse-proxy handshake limits and network access
controls supplement (not replace) gateway limits. Do not log frames, tickets,
query strings or authorization headers at the proxy. The service logs only fixed
startup/failure messages, never payloads, upstream errors or configuration values.

## Browser contract

Open `wss://<web-host>/socket` without a query string or browser subprotocol.
Origin must exactly equal `HELM_WEB_ORIGIN`, with exactly one Origin header.
The first text frame, within 5 seconds, must be:

```json
{"type":"authenticate","ticket":"BASE64URL_PAYLOAD.BASE64URL_SIGNATURE"}
```

The signature is HMAC-SHA256 over the **encoded payload string**, signed with the
shared secret, both segments unpadded canonical base64url. Exact payload fields:

```json
{"aud":"helm-web-gateway","sub":"64-character-hex-session-hash","jti":"fresh-UUID","exp":1234567890,"vessel":"local"}
```

`sub` is the stable SHA-256 operator session hash, not a cookie or raw session ID.
`exp` is integer Unix seconds, strictly future and at most `floor(now/1000)+60`.
Laravel must cap it at the session's fixed eight-hour expiry, issue fresh random
UUID `jti`s, and only issue tickets for authenticated/authorized aliases. The gateway
cannot observe Laravel logout directly: stop refresh on logout; the remaining
lease ends within 60 seconds. Synchronize the two machines' clocks.

After validating the upstream hello's `protocol:1`, UUID socket ID and configured
Vessel UUID, the gateway emits exactly:

```json
{"type":"ready","vessel_id":"00000000-0000-4000-8000-000000000002"}
```

Journal keys must use this actual Vessel UUID, not the alias. Refresh the ticket
through Laravel every 30 seconds and send another authenticate frame on the same
socket. Renewal requires the same `sub` and alias, consumes a new `jti`, updates
the lease to the ticket expiry and emits the same ready frame (once upstream is
ready). Renewal does not reconnect or resend commands. Expiry closes both sockets,
including pending requests; transport loss **never** proves mutation refusal.
Inspect a durable receipt with the original command ID after reconnecting.

Tickets are single-use across all connections in this process until their expiry.
Run **one gateway process**, not clustered/replicated instances with independent
replay stores. The production entrypoint enforces a 60-second admission blackout on every
startup (upgrades receive 403) to outlive tickets consumed by the old process.
Stop the old process before starting its replacement; overlapping replicas are
not supported. Durable/shared ticket storage and rolling multi-worker deployment are
not implemented. Keep the secret stable between Laravel and this process.

Commands/replies use the actual public Rust envelope, not REST or tool arguments:

```json
{"type":"command","request_id":"00000000-0000-4000-8000-000000000003","request":{"protocol":1,"command":{"op":"snapshot","session_id":"00000000-0000-4000-8000-000000000004"}}}
```

```json
{"type":"reply","request_id":"00000000-0000-4000-8000-000000000003","response":{"protocol":1,"result":{},"error":null,"outcome_unknown":false}}
```

Allowed operations and fields beyond `op`:

| Operation | Required fields |
| --- | --- |
| `capabilities`, `catalogue` | none |
| `inspect` | `session_id` |
| `snapshot`, `decisions` | `session_id` |
| `history` | `session_id`, `offset`, `limit`; optional nullable `expected_revision` |
| `message_chunk` | `session_id`, `index`, `offset`, `limit`, `expected_revision` |
| `run_output` | `session_id`, `run_id`, `offset`, `limit` |
| `receipt` | `session_id`, `command_id` |
| `events` | `session_id`, `after`, `limit`, `wait_ms` |
| `submit` | `session_id`, `command_id`, `expected_revision`, `expires_at_ms`, `prompt` |
| `steer` | submit fields plus `incarnation`, `run_id` |
| `cancel` | `session_id`, `incarnation`, `run_id`, `command_id`, `expected_revision`, `expires_at_ms` |
| `respond` | cancel fields plus `decision_id`, `response` |

Voyage operations also accept optional nullable `incarnation`, except live
`steer/cancel/respond` where a non-null UUID is required. `inspect` is a Vessel
operation and does not accept incarnation. `response` is the Rust JSON value;
Voyage validates its decision-specific contents and retains authority. Numeric
fields must be nonnegative safe JS integers. Unknown fields and operations are
refused (including coordination injection, execution, browser, terminal, accounts,
start, grant and subscriptions). Prompt is a nonempty string.

`limit` is 1..128, except `message_chunk/run_output` allow 1..65536 bytes;
`events.wait_ms` is 0..10000. Use bounded request polling: snapshot every second,
decisions as needed, catalogue every ten seconds. No subscribe/unsubscribe,
unsolicited event or reverse frames are accepted. A reverse request closes both
sockets without executing or acknowledging it. Preserve exact revision/incarnation
from fresh snapshot/decision observations in client mutations.

Reply envelopes are strictly validated and correlated with pending UUID requests.
Ordinary public `result` and refusal `error` values remain unchanged; they are Rust
`serde_json::Value` results, not private IPC envelopes. Capabilities are the sole
projection: require protocol/Vessel identity and filter `features` to the
non-execution gateway subset; retain only protocol, vessel_id, version, scope,
expiry, grant revision and session ID. No rights, credentials, future arbitrary
metadata, broad voyage_operations, SSE/subscription, start/account/browser
capability advertising is forwarded. UI availability must still respect actual
runtime refusals. Do not log reply data: it may contain operator conversation.

## Bounds and failure semantics

- 4 MiB/message in either direction, compression disabled; buffered sends also
  limited to 4 MiB. Binary and malformed frames close the connection.
- 64 WebSockets globally, 4 per session hash; HTTP server additionally caps TCP
  connections at 80, with 5-second request/header deadlines.
- 32 pending requests/connection; 60 inbound frames/second token bucket with burst
  60 (including authentication); 10,000 correlation IDs/connection (no reuse),
  10,000 unexpired consumed ticket IDs/process. Capacity exhaustion fails closed.
- Initial authentication and upstream handshake/hello deadlines: 5 seconds each.
  Request deadline: 15 seconds. Gateway does not retry timed-out commands.
- Ping/pong both sockets every 5 seconds; missing pong by next tick closes both.
  Close handshake gets one second before termination. Browser pong is automatic.
- Fixed close reasons only: policy 1008, capacity 1013, transport/deadline 1011;
  ws uses 1009 for oversized messages. Do not treat close as a durable receipt.

## Verification and source anchors

Reviewed `crates/voyage-protocol/src/duplex.rs` (`ClientFrame`, `ServerFrame`),
`crates/voyage-protocol/src/vessel.rs` (`VesselRequest`, `VoyageCommand`,
`VesselCommand`, `VesselResponse`), `vessel/src/duplex.rs`,
`vessel/src/process_http.rs`, and `vessel/src/process/access/{connection,routing}.rs`.

`npm test` uses Node's test runner with real local WebSockets and a fake upstream:
headers/subprotocol, pinned hello, command/refusal correlation, renewal/replay,
lease expiry, Origin/path/auth failures, forbidden frames and commands, capability
projection, limits, timeouts, heartbeat and private-file restrictions. This is not
a live deployed Vessel/provider test, native Windows test, or Laravel UI test.
No Rust edits or Rust coverage refresh are part of this scope.
