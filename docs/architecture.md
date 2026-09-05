# Voyage architecture

Voyage separates local execution from the planned remote management plane. Helm
owns provider credentials, canonical conversation state, policy, approvals and tool
execution. Local chat, runs, sessions, terminals, todos and subagents remain available.

## Connection transition

The legacy pairing and HTTP task-worker path has been removed as an intentional
clean break under [#77](https://github.com/o-psi/voyage/issues/77). **There is currently
no Helm-to-Vessel connection mode.** Neither `helm --voyage` nor the proposed
`helm attach` is a supported command in this build.

The target [Vessel-managed session design](vessel-session-management.md) reverses
enrollment: Vessel issues a one-use invitation and Helm redeems it, then initiates
an outbound interactive connection. This is planned, not implemented:

```text
operator ── Helm TUI ── local policy ── tools

planned: Vessel ◀── outbound attached Helm ── local policy ── tools
```

See [retirement and data preservation](vessel-connectivity-retirement.md) for removed
interfaces and upgrade/rollback limitations. Old enrollment files and Vessel SQLite
snapshots remain on disk but are not loaded, migrated or executed. Removing code
does not stop already-running old binaries or revoke credentials on other servers.

## Trust boundaries

1. Provider credentials and provider continuation state remain exclusively on Helm.
2. All execution remains subject to local roots, command policy, approvals,
   cancellation and resource limits. Application policy is not an OS sandbox.
3. Future enrollment must use distinct scoped credentials and explicit session
   sharing consent. Old pairing strings are not join keys.
4. Helm initiates network connections; attachment must not require an inbound Helm
   task port. Vessel is the authenticated control plane, not an execution authority.

## Local parallel agents

Helm can supervise a bounded tree of local child agents for independent work. The
parent owns orchestration while each child has an explicit task, budget, tool
policy, cancellation scope, and durable record. Coding children can use isolated
Git worktrees; non-coding children can use ordinary policy-scoped directories.
See [Parallel subagents](subagents.md) for lifecycle and safety requirements.

## Current management-plane surface

Vessel retains:

- `GET /health`: process liveness.
- `GET /ready`: database connection check, not remote-execution readiness.
- `GET /metrics`: explicit disabled-connectivity gauge.
- `GET /v1/diagnostics`: authenticated version/transition status, no legacy content.
- `GET /ui`: authenticated static status page explaining unavailable connectivity.

All old pairing, worker, fleet, task and task-UI routes are gone. There is no task
scheduler/reaper, credential recovery or operator write surface in the interim server.
`voyage-protocol` retains common health/error responses and now contains strict v2
command-domain foundations, not legacy task types or an enabled new transport.
The [attachment foundations](attachment-foundations.md) also include a transactional
local command/run journal; neither is wired to remote execution. Enrollment, event
transport, full coordination and authorization remain open under #9/#10/#78/#79.
