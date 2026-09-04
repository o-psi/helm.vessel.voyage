# Voyage architecture

Voyage separates execution from orchestration without requiring Helms to accept
inbound connections. A Helm owns credentials, conversation state, policy decisions,
and tool execution. Vessel manages identity, liveness, queues, and task history.

```text
                       one-time pairing string
Helm operator ─────────────────────────────────────▶ Vessel operator

Helm ── authenticated outbound heartbeat/poll/results ──▶ Vessel
  │                                                       ▲
  └── local policy ── tools                         task control
```

## Pairing and connection

1. `helm --voyage [VESSEL_URL]` creates an ephemeral Helm identity and asks Vessel for
   a ten-minute pairing code.
2. Helm prints `voyage:v1:XXXXXXXXXX` and polls only for its pairing status.
3. An operator passes that string to `vessel pair STRING` or the future Vessel UI.
4. Vessel claims the pending Helm and activates its private worker credential.
5. Helm sends outbound heartbeats, pulls queued tasks, and posts results. It never
   binds an inbound task port and may remain behind NAT or a firewall.

The current transport is authenticated HTTP polling. The protocol boundary permits a
future WebSocket or HTTP/2 stream without changing the pairing and authority model.

## Trust boundaries

1. Provider credentials remain exclusively on Helm and are never sent to Vessel.
2. Every task still passes through Helm's local approvals, filesystem roots, command
   policy, timeouts, and resource limits.
3. The pairing code is short-lived and authorizes association, while the unprinted
   worker credential authenticates subsequent traffic.
4. Vessel is responsible for operator authorization, durable task state, credential
   rotation, and encrypted transport in production deployments.

## Wire contract

`voyage-protocol` defines versioned pairing, Helm inventory, queued tasks, and results.
Vessel exposes:

- `POST /v1/pairings/start` and `GET /v1/pairings/{code}` for Helm
- `POST /v1/pairings/claim` for an operator consuming the printed string
- `POST /v1/worker/heartbeat`, `GET /v1/worker/tasks/next`, and
  `POST /v1/worker/tasks/{id}/result` for outbound workers
- `GET /v1/helms`, `GET|DELETE /v1/helms/{id}` for fleet management
- `POST /v1/helms/{id}/tasks` and `GET /v1/tasks/{id}` for queued work

## Production layers

- Persistent Helm identity and encrypted worker credentials
- WebSocket/HTTP2 event streaming, cancellation, leases, and reconnection replay
- Authenticated operator claims, TLS/mTLS, scoped principals, and credential rotation
- SQLite/Postgres Vessel persistence and append-only task/audit history
- Ratatui Helm frontend, MCP tools, context compaction, observability, and packaging
