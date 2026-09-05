# Voyage architecture

Voyage separates local execution from the planned remote management plane. Helm
owns provider credentials, canonical conversation state, policy, approvals and tool
execution. Local chat, runs, sessions, terminals, todos and subagents remain available.

## Outbound attachment presence

Vessel issues a one-use enrollment invitation and Helm redeems it. An enrolled
Helm can maintain an authenticated, foreground outbound connection with
`helm attachment --directory /absolute/identity connect`. See
[enrollment](attachment-cli.md) and [presence](attachment-presence.md) for setup,
shutdown, revocation and bounded diagnostics.

```text
operator ── Helm TUI ── local policy ── tools
Vessel ◀── outbound Helm presence (heartbeats only)
```

Presence negotiates no application capabilities and cannot dispatch work or
expose sessions. The [Vessel-managed session design](vessel-session-management.md)
remains the target for explicit sharing, interactive control and services.

## Trust boundaries

1. Provider credentials and provider continuation state remain exclusively on Helm.
2. All execution remains subject to local roots, command policy, approvals,
   cancellation and resource limits. Application policy is not an OS sandbox.
3. Enrollment uses distinct scoped credentials. Sharing session data and remote
   control require separate explicit consent and current local authorization.
4. Helm initiates network connections; attachment must not require an inbound Helm
   task port. Vessel is the authenticated control plane, not an execution authority.

## Local parallel agents

Helm can supervise a bounded tree of local child agents for independent work. The
parent owns orchestration while each child has an explicit task, budget, tool
policy, cancellation scope, and durable record. Coding children can use isolated
Git worktrees; non-coding children can use ordinary policy-scoped directories.
See [Parallel subagents](subagents.md) for lifecycle and safety requirements.

## Current management-plane surface

Vessel provides:

- `GET /health`: process liveness.
- `GET /ready`: database connection check, not remote-execution readiness.
- `GET /metrics`: whether attachment presence is configured.
- `GET /v1/diagnostics`: authenticated status and bounded current presence metadata.
- `GET /ui`: authenticated static status page.
- Explicitly configured enrollment administration and `/v2/attachment` presence.

`voyage-protocol` contains strict v2 command, feature and event contracts. The
[attachment foundations](attachment-foundations.md) include a transactional local
command/run journal. Production presence does not connect these foundations to
remote execution. Full coordination, sharing, operator sessions, approvals and
services remain open under #9/#10/#78/#79 and #77.
