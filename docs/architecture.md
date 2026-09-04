# Voyage architecture

Voyage separates execution from orchestration. A Helm owns credentials, conversation
state, policy decisions, and tool execution on one machine. Vessel knows where Helms
are and whether they are alive, but does not reinterpret agent messages or execute
their tools.

## Trust boundaries

1. A Helm provider adapter communicates with an LLM API using a local environment
   secret. Provider credentials are never registered with Vessel.
2. Helm's policy resolves every native filesystem operation beneath configured roots.
   Shell remains an OS-level capability and should be paired with container or account
   isolation for untrusted workloads.
3. A served Helm can require a bearer token. TLS is expected at a reverse proxy until
   native TLS and mutual Helm/Vessel identity are added.
4. Vessel stores public Helm metadata and liveness. The current registry is in memory;
   a durable repository interface is the next control-plane persistence boundary.

## Wire contract

`voyage-protocol` defines the `/v1` request and response shapes. Helm exposes:

- `GET /health`
- `GET /v1/info`
- `POST /v1/tasks`

Vessel exposes:

- `GET /health`
- `GET /v1/helms`
- `POST /v1/helms/register`
- `POST /v1/helms/heartbeat`
- `GET|DELETE /v1/helms/{id}`
- `POST /v1/helms/{id}/tasks`

Registration is followed by a 30-second heartbeat. Vessel marks missed heartbeats
stale and then offline, while retaining inventory for inspection.

## Planned production layers

- Ratatui full-screen Helm frontend with streaming events and approval modals
- Authenticated registration, secret references, TLS/mTLS, and scoped principals
- SQLite/Postgres Vessel registry and append-only task/audit history
- Streaming task transport, cancellation, queues, leases, and idempotency keys
- MCP tool servers and per-Helm capability advertisement
- Context compaction and artifact-aware session storage
- Observability, quotas, provider retry/backoff, and deployment packaging
