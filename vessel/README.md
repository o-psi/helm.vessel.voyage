# Vessel

Vessel is the durable Voyage control plane. Its SQLite database is migrated at
startup and written with WAL and full synchronous durability. Worker credentials
are stored as SHA-256 verifier hashes, never as bearer-token plaintext.

```sh
vessel --database /var/lib/voyage/vessel.db --bind 127.0.0.1:9480
vessel pair voyage:v1:PAIRINGCODE
vessel fleet
```

Task delivery uses ownership leases: every pull creates a unique lease, stale
completions are rejected, expired work is requeued up to three attempts, and
explicit failures can be retryable or terminal. `POST /v1/tasks/{id}/cancel`
cancels queued or running work. Operational APIs include `GET /v1/helms`, `GET
/v1/tasks`, and `GET /v1/fleet/summary`.
