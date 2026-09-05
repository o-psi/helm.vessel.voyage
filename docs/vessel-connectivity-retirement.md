# Legacy Vessel connectivity retirement

Tracking: [#77](https://github.com/o-psi/voyage/issues/77).
The operator explicitly chose a **clean break before replacement attachment works**.
This supersedes the earlier plan to preserve pairing/task-client compatibility.

## Current availability

Local Helm chat, runs, sessions, providers, tools, terminals, todos and subagents
remain available. There is currently **no Helm-to-Vessel connection mode**.
`helm attach VESSEL_URL JOIN_KEY` remains an [agreed design](vessel-session-management.md),
not an implemented command. No inbound Helm port or replacement transport was added.

Removed interfaces:

- Helm `--voyage`, its worker-name `--name` option, and the TUI `/voyage` handoff.
- Helm's enrollment-file loader/writer and HTTP pairing, heartbeat, task polling,
  result/failure upload and automatic reconnect loop.
- Vessel `pair`, `fleet`, `--pairing-ttl-secs`, `--lease-secs`, and `--stale-after-secs`.
- All `/v1/pairings/*`, `/v1/worker/*`, `/v1/helms*`, `/v1/tasks*`,
  `/v1/fleet/summary`, `/ui/helms/*` and `/ui/tasks/*` routes.
- Legacy task queues, leases, reaping/retries, fleet/task UI and shared v1
  pairing/task types. There is no compatibility shim or wire-version negotiation.

Old CLI invocations fail argument parsing; retired HTTP routes return 404. Help,
completion scripts and manpages no longer advertise the retired commands. A valid
old worker credential or current operator token cannot reactivate those routes.

Vessel retains `/health`, `/ready`, `/metrics`, `/v1/diagnostics` and `/ui`.
Health/readiness mean the process/database connection is usable, **not** that Helm
attachment works. Metrics report `voyage_connectivity_enabled 0`, not fabricated
zero fleet/task counts. Diagnostics and UI require the configured operator token
(Bearer or Basic password) and explicitly report unavailable connectivity. Without
`VESSEL_OPERATOR_TOKEN`, both are disabled (503); missing/invalid credentials get
401 when enabled. There are no remaining operator write routes.

## Data preservation and deployment

1. Stop old Helm workers and Vessel instances before upgrading. This source change
   does not terminate already-running old binaries, services or remote processes.
2. Back up Helm's config/session directories and Vessel's SQLite database using a
   consistent backup procedure. If SQLite WAL files exist, use SQLite's backup API
   or stop the old server and preserve the database and its sidecars together.
3. Remove obsolete worker startup arguments from service definitions/scripts.
   Do not replace them with an `attach` command until that feature ships.
4. Existing `helm/voyage.json` under the platform config directory is left untouched
   and is no longer read. It may still contain a sensitive legacy worker token.
5. Existing Vessel `control_plane` snapshots and `schema_migrations` are not loaded,
   migrated, overwritten or deleted. Stored queued/running states are historical,
   not evidence of a surviving run. No tasks or external tool effects are replayed.
6. Local Helm session files are neither shared nor deleted by this cleanup.
   Future enrollment will need explicit credentials and sharing consent; old pairing
   strings are not join keys. The future storage/migration contract remains pending.

Preserved data may still contain credentials, prompts and results: keep backups and
files private. Removing a transport is not secure erasure or a remote revocation
mechanism. No new revocation command is supplied by this interim build.

Rollback requires the operator to deliberately restore a compatible old binary pair
and secured backups/configuration. Old binaries may resume saved credentials and
retry uncertain work; review queued/running tasks before allowing old workers to
reconnect. Do not assume restoring a binary is safe automatic task recovery.

## Verification contract

`tests/system/vessel_lifecycle.py` replaces the old positive pairing/lease/retry
fixture with rejection coverage for every former route, credential variants,
malformed requests, concurrent requests, restart, fresh storage, legacy snapshot
preservation, auth, diagnostics, correlation IDs and Helm data preservation.
CLI/TUI units prove no worker handoff or advertised attachment command remains.
Vessel units cover inert malformed legacy snapshots, storage failure, auth failures,
and poisoned database readiness. Local-provider and full workspace tests protect
retained Helm behavior. New-transport replay/cancellation/sharing tests belong to
attachment delivery; no live network protocol was added here. Linux verification
alone does not establish macOS/Windows or live-provider coverage.
