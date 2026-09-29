# Event-only Helm–Vessel transport: v1.1.0 release requirement

**Planned requirement, not a claim of current behavior.** v1.1.0 is the next
planned release after v1.0.3. The delivery record and acceptance checklist are
[#374](https://github.com/o-psi/helm.vessel.voyage/issues/374), in the
[v1.1.0 milestone](https://github.com/o-psi/helm.vessel.voyage/milestone/4).

Legacy snapshot observation is deprecated for the v1.0.3 transition and must be
removed by v1.1.0. The transitional event work in
[#366](https://github.com/o-psi/helm.vessel.voyage/issues/366) retains compatibility
and snapshot recovery; that is not the v1.1.0 endpoint.

## Required experience and protocol

Both Helm TUI and production Helm Web must use the new event protocol for every
interaction and update with Vessel. Typed command envelopes have stable identities
and correlated outcomes. Content-bearing events let clients update their local
state directly. Metadata-only invalidation followed by a snapshot read does not
meet this requirement.

Initial loading and late subscriptions use bounded entity/history event sequences
with a consistent cursor and completion barrier. Large histories and values remain
paged or chunked. Reconnect uses replay; retention gaps and incarnation changes use
explicit bounded reinitialization events. A monolithic session snapshot renamed
as an event is not an acceptable implementation.

The migration covers catalogue, conversations, text/reasoning, tools, decisions,
commands, settings, accounts/usage, lifecycle, attachments/artifacts, host browser,
terminal and update operations. Audit every existing request and subscription in
both clients and map it to the shared contract. Remove obsolete routes, types,
public-v1 negotiation, refresh timers, reducers and fallback branches. Old peers
receive an actionable upgrade response instead of a silent legacy fallback.

Voyage remains the execution owner. Preserve scoped authorization and revocation,
private-input fencing, stable identities, exact command deduplication and observed
cleanup. Canonical disk checkpoints, backups and browser DOM representations have
different responsibilities and are not being removed. They must not become a
workaround for retired session-snapshot transport. Connection authentication and
bounded content transfer retain their security boundaries.

## Release gates

- Complete the interaction inventory and implement all mapped event paths in
  Voyage, Vessel, Helm TUI and the separate `o-psi/webhelm` production client.
- Prove zero legacy snapshot calls in transport traces and journey tests, including
  initial load, long history, simultaneous clients, sparse/duplicate/reordered
  delivery, gaps, disconnect/restart, cancellation, decisions and failure recovery.
- Verify private data stays excluded and stale peers cannot fall back silently.
- Record local tests and coverage, actual built source/artifact identities,
  installed TUI and deployed Web evidence. Keep #374 open until removal is complete.

The nightly target remains v1.0.3 while that release is pending. After v1.0.3 is
published, advance the target to v1.1.0 through the normal release process.
Planning and milestone closure do not publish a stable release.
