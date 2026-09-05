# Vessel connectivity

Vessel supports explicit outbound connections initiated by Helm. It requires no
inbound Helm task port, and provider credentials stay on the executing Helm.
Current setup is documented in these guides:

- [Enrollment lifecycle](attachment-cli.md): explicit enrollment, status, rotation,
  revocation and offline detach, with private identity storage and secret input.
- [Foreground presence](attachment-presence.md): authenticated heartbeat-only
  connections that expose no session content or execution authority.
- [Dedicated remote sessions](remote-sessions.md): opt-in foreground execution of
  one new managed session with authenticated list, inspect, submit, replay and cancel.
- [Security and operations](security-operations.md): TLS proxy, authentication,
  private storage, policy, revocation and recovery boundaries.

Health and readiness describe service/database availability. They do not establish
that a Helm is connected or authorized to execute. Diagnostics distinguish enabled
modes from current connections; a presence-only peer cannot receive execution
commands. Existing local sessions are not implicitly shared.

The planned [voyage model](voyages.md) adds Helm as the local and remote operator
interface, with an independently placed coordinating Helm and a user-scoped set of
execution participants. A voyage is open-ended and needs no project or repository.
The current dedicated HTTP worker does not deliver that orchestration. Browser
console work is deferred. Tracking remains in
[#77](https://github.com/o-psi/voyage/issues/77), with the current attachment
requirements in [the production contract](attachment-production-contract.md).

Keep enrollment, session stores and backups private. Disconnecting an observer is
distinct from cancellation; current worker transport loss conservatively cancels
its active turn. Reconnection never replays effects, and uncertain crash outcomes
require explicit recovery with honest cleanup evidence.
