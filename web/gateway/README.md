> **Legacy relay:** the current Helm Web console does not use this gateway.
> Retained for deliberate rollback, legacy clients and comparative fixtures.
> See [direct-WSS migration](../../docs/helm-web.md#upgrade-and-retire-the-conversation-gateway).
> Do not stop an existing deployment until matching browser/Laravel/Vessel upgrades are verified.

# Helm Web tenant gateway

Node >=22; `npm ci --ignore-scripts`, then `npm test` / `npm start` in this
folder. Tests additionally use OpenSSL for an ephemeral TLS fixture certificate.
This service forwards a restricted browser protocol to publicly reachable Vessels;
it is not a general executor, HTTP proxy, or provider credential service.

## Configuration and deployment

- `HELM_WEB_GATEWAY_SECRET`: printable ASCII secret, at least 32 bytes; shared
  privately with Laravel. Never put it in a browser, URL, or log.
- `HELM_WEB_AUTH_URL`: fixed canonical loopback **HTTP** URL, e.g.
  `http://127.0.0.1/console/gateway/authorize`. Literal `127.0.0.1` or `[::1]`
  only, no credentials, query, fragment, redirects or browser override.
- `HELM_WEB_ORIGIN`: exact browser origin, normally `https://console.example`.
  Loopback HTTP origins are permitted for the browser shell during development.
- `HELM_WEB_GATEWAY_HOST`: `127.0.0.1` (default) or `::1`, never a public bind.
- `HELM_WEB_GATEWAY_PORT`: defaults to `8787`.

The old `HELM_WEB_VESSELS_FILE` and `HELM_WEB_ALLOW_LOOPBACK_WS` are refused.
There are no global aliases, cached credentials, or production private-upstream
switches. TLS terminates at a trusted reverse proxy for the browser. Forward only
`/socket` upgrades (preserving exactly one Origin) to this listener; **never proxy
`/pair` or `/probe` publicly**. These endpoints require a direct loopback peer,
no Origin header, and the server secret. A reverse proxy is itself a loopback
peer, so the proxy path restriction is an essential deployment boundary.
Disable request-body/Authorization logging in both proxy and Laravel. This
service emits only fixed startup/listener diagnostics, not request diagnostics.

## One-time authorization

The browser sends `{ "type": "authenticate", "ticket": "..." }` as its first
WebSocket text frame, not in the upgrade URL. Tickets are opaque canonical
base64url encoding of at least 32 random bytes (maximum encoded length 512).
The gateway POSTs `{ "ticket": "..." }` to the configured authorization URL,
with `Authorization: Bearer <HELM_WEB_GATEWAY_SECRET>`. Only a 200 JSON response
of at most 64 KiB is accepted, within five seconds. Laravel must atomically
consume the ticket and verify the current database session, tenant ownership,
connection and revocation; no gateway cache, retry, signed-ticket fallback, or
restart blackout substitutes for server-side one-time redemption.

Response, with exactly these keys:

```json
{
  "sub": "64-hex-character-session-hash",
  "tenant": "tenant-uuid",
  "vessel": "tenant-connection-uuid",
  "exp": 1700000060,
  "connection": {
    "url": "wss://vessel.example/v1/vessel/socket",
    "token": "private-vessel-token",
    "grant_id": "grant-uuid",
    "vessel_id": "vessel-uuid"
  }
}
```

`exp` is integer Unix seconds, in the future and no later than now +60 seconds.
The lease closes both transports at expiry. Fresh single-use tickets renew the
same socket only when `sub`, `tenant`, `vessel`, URL, token, grant ID and Vessel
ID are unchanged. Overlapping authentication attempts fail closed. Identity or
credential changes require a new socket. Outage/refusal does not retain an old
lease by silently skipping redemption. Successful upstream hello retains
`{ "type": "ready", "vessel_id": "..." }`; credentials never reach the browser.

## Public upstream boundary

Only canonical WSS `/v1/vessel/socket`, port 443, is accepted. Pairing accepts
an HTTPS origin (with optional trailing `/`), also port 443. No userinfo, query,
fragment, redirects, noncanonical paths, or alternate ports. All DNS answers
must be global unicast: private, loopback, link-local, shared address space,
documentation, benchmark, multicast/reserved IPv4 and non-global/special IPv6
(including mapped IPv4, NAT64, Teredo and 6to4) are refused. IPv6 admission is
conservatively limited to `2000::/3` excluding special ranges; some special
otherwise routable addresses are intentionally refused. DNS has a five-second
bound. One validated address is pinned via the actual socket's `lookup`, with
no pooled agent or subsequent DNS fallback. TLS certificate/hostname checking
and SNI remain enabled for the original hostname. Address literals undergo the
same address policy and native TLS IP certificate verification.

Module tests inject fixture transports; `server.js` never reads an environment
option or file to replace production validation or TLS verification.

## Local Laravel service contract

Both endpoints require exact `POST` paths, JSON, and
`Authorization: Bearer <HELM_WEB_GATEWAY_SECRET>`. Input is limited to 16 KiB,
responses to 64 KiB, and at most eight concurrent local requests. Unknown paths,
methods, query variants, browser Origin, missing/wrong secret, extra input keys
and arbitrary command fields are refused. Errors contain only
`{"error":"gateway refused"}` (404/403/503/502 as applicable); transport timeout
may instead close the connection. Do not treat a lost pairing response as proof
that the invitation was not redeemed.

- `POST /pair` accepts exactly `{endpoint,principal_id,invitation_id,command_id,
  code,vessel_id}`. Validates and pins the public HTTPS origin, then POSTs
  `/v1/vessel/pair` with `{protocol:1,principal_id,invitation_id,command_id,code}`
  and `x-voyage-vessel: <vessel_id>`. Successful Vessel envelopes
  `{protocol:1,result:credential,error:null,outcome_unknown:false}` are returned
  intact and privately to Laravel. Refusals/unknown outcomes become fixed
  errors. No automatic retry: retain the original command and invitation IDs
  for reconciliation. Laravel must validate and encrypt the returned credential.
- `POST /probe` accepts exactly `{connection:{url,token,grant_id,vessel_id}}`.
  Opens the public pinned WSS socket with protocol `voyage.vessel.v1`, verifies
  the hello's Vessel identity, sends only the `capabilities` command, verifies
  reply correlation, returns filtered capabilities **at the top level** including
  `vessel_id`, and closes the socket. It cannot issue arbitrary commands.

Browser operation allowlisting, strict hello/reply validation, request-ID
non-reuse, 4 MiB frame/backpressure limits, 64 connections, four per session
subject, 32 inflight requests, 60 frames/second, five-second hello/heartbeat,
15-second request timeout and 10,000 seen IDs remain enforced. No automatic
replay after disconnect; mutation outcomes may be unknown.

Protocol sources: `crates/voyage-protocol/src/duplex.rs`,
`crates/voyage-protocol/src/vessel.rs`, `vessel/src/process_http.rs`, and
`vessel/src/process/pairing.rs` (all paths relative to repository root).

## Verification scope

`npm test` exercises a real local HTTP single-use issuer and WebSocket upstream
via module dependency injection, tenant/connection renewal fences, redemption
replay and outage, framing/operation/connection/rate/time limits, capability
projection, local endpoint auth, exact pairing wire request, resolver all-address
validation, pinned lookup, bounded HTTP failures, and a real TLS WebSocket
fixture with valid and mismatched hostnames. These are not live Laravel OAuth,
public network DNS/Vessel, provider, or native-platform tests. No Rust changes
or coverage refresh are part of this gateway-only delivery (#290).
