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


## Cutover inventory and proposed ownership

This inventory is from core source `07278f2f` and the production Web checkout,
not a passing transport trace. Every row remains an implementation gate. The
existing `public-v2` feed is **not** an event-only connection: it requires snapshot
hydration and returns `recovery: snapshot` on a retention gap.

| Interaction | Current entry points | Required replacement/removal proof |
| --- | --- | --- |
| Admission and commands | `crates/voyage-protocol/src/duplex.rs`, `vessel.rs`; TUI `process_client/transport.rs`; Web `resources/js/vessel-client.js` | Versioned hello admits compatible clients before mutations; typed commands and correlated durable outcome events; incompatible peers receive upgrade guidance. Socket request IDs must not replace durable command IDs. |
| Catalogue and lifecycle | TUI `ui/catalogue_watch.rs`, `ui/archive.rs`; Vessel catalogue/change commands | Bounded catalogue entity initialization and barrier, followed by replayable upserts/removals; eliminate full catalogue hydration and timer fallback. Scope-filtered sparse positions are valid. |
| Conversation, history and large content | TUI `ui/observe.rs`, `ui/live_reducer.rs`, `ui/transcript/navigation.rs`, `plain/session.rs`, `export.rs`; Web `resources/js/conversation-stream.js` | Bounded message/history sequences and UTF-8 content chunks; eliminate snapshot seeding and snapshot recovery, without assembling a renamed full-session event. |
| Runtime production and retention | `voyage/src/attachment/journal/observations.rs`, `voyage/src/server/observations.rs`, `voyage/src/server/suspended.rs` | Durable content-bearing entities, bounded initialization at a consistent journal fence, explicit gap/incarnation reinitialization; no raw provider arguments or private input. Coordinate suspended-owner changes with #256. |
| Routing and downgrade | `vessel/src/process/service.rs`, `subscriptions_final_tests.rs` | The local #374 source removes the public-v2-to-v1 retry after decoder failure (focused Rust validation pending); a rejected projection is an upgrade error, not permission to hydrate from legacy transport. |
| Text, reasoning, tools and diagnostics | `live_events.rs`, `provider_attempt.rs`, TUI `ui/previews/`, `ui/provider_attempts.rs` | Stable canonical run/attempt/message/tool references and bounded typed values; provider identifiers retain separate scope and evidence quality. Coordinate model/tool presentation with #418. |
| Decisions, cancellation and retained work | `VoyageCommand::Decisions/Respond/Cancel`, TUI `ui/run_controls.rs`, `ui/receipts.rs`, `ui/reconcile.rs`; Web `Decisions.tsx` | Outcome and decision entities plus exact retained resource/child obligations. Stop remains available for owned work after foreground terminality; requested cancellation is not observed cleanup. |
| Settings, models, accounts and usage | `VoyageCommand::Controls/SetInference/SetAccountInference`, `VesselCommand::Accounts/AccountUsage/DiscoverModels`; TUI `ui/accounts/`, `ui/inference/` | Typed bounded metadata entities and exact command results, no refresh RPC following metadata-only invalidation. Existing account authority and private enrollment remain separate. |
| Creation and launch | TUI `ui/new_draft/launch.rs`, Web `new-voyage-delivery.ts` | Creation admission/outcome and initial entity sequence replace post-start Snapshot; preserve start/submission command identities. Coordinate with #328. |
| Browser, terminals and transfers | `host_browser.rs`, `terminal.rs`, `VoyageCommand::ReadArtifact/UploadImage/Terminal/HostBrowser`, TUI `ui/terminals.rs`, `process_client/host_browser.rs` | Event command/progress/outcome paths preserve bounded authenticated content transfer and private-input fencing. Internal DOM and terminal screen representations are not session snapshots. |
| Updates, notifications, workspace, goals and execution panels | `VesselCommand::Update*/Notifications/Execution`, `VoyageCommand::Workspace*/Goal*`; TUI corresponding `ui/` panels; Web corresponding React panels | Inventory each panel request and background refresh; content-bearing entities and exact outcomes, without broadening privileged or update authority. |
| Non-chat TUI entry points | `process_client/frontend.rs`, `frontend/{github,workflow,resume}.rs`, `commands.rs`, `export.rs` | Same event-only loading/admission path as chat; zero Snapshot calls outside panels as well as within them. |

### Proposed connection semantics (not shipped)

The cutover should use a new explicitly admitted connection version, rather than
changing the meaning of `public-v2` underneath running peers. Authentication and
scoped authority precede subscription or mutation admission. Initialization has a
unique generation, owner incarnation, selected entity/history scope, bounded typed
pages and an explicit completion barrier at a retained cursor. Changes racing that
barrier replay after its cursor. Partial initialization is never authoritative;
clients discard its staging generation on failure, gap or owner change.

Replay cursors are ordered journal positions, not contiguous per-session counters.
Duplicate delivery is idempotent within the generation; sparse positions do not
imply loss. Gaps, expired initialization and incarnation changes require a new
bounded entity initialization, never Snapshot, synthetic state or replay of an
external tool effect. Slow consumers get an explicit recoverable transport outcome
and retain their pending durable command identities.

Every command targets canonical owner/session/incarnation plus applicable run and
entity identity. The executing owner retains principal/payload binding and exact
deduplication; socket correlation is only delivery correlation. Reconnect observes
the outcome of a retained command, it does not create another command ID. Provider
native IDs cannot authorize control. Missing historical canonical identities must
remain unresolved or explicitly scoped derivations, not fabricated UUIDs or rewritten
history. Cleanup state is an observed entity independent of foreground run state.

Both reducers must consume the same typed initialization, delta, error and outcome
fixtures. Required evidence includes rejected incompatible clients, sparse and
reordered delivery, duplicate pages, interrupted initialization, incarnation changes,
private-data exclusion and uncertain commands preserved across reconnect. Rust
protocol additions alone do not satisfy either client migration or installed/deployed
verification. Production Web changes require coordinated ownership in `o-psi/webhelm`;
this inventory does not authorize edits there or establish Web delivery.


### In-progress implementation boundaries

`voyage-protocol::event_connection` stages version-3 initialization and control
admission types separately from the still-advertised v1 socket. Initialization
entities are capped at 32 KiB each and 8 MiB per selected scope; larger scope
requires pagination, and large content uses byte-offset UTF-8 chunks. The matching
Web modules `event-initialization.js` and `event-command.js` are isolated reducers,
not yet used by the production connection. They must not be advertised as a
completed migration. Control operation coverage currently includes only cancel,
rename and access changes; remaining commands, owner production, retention replay,
transport admission, both connected reducers and removal of snapshots remain gates.

The Vessel downgrade removal deliberately returns the original uncertain read
outcome rather than retrying an incompatible private decoder. This does not broaden
authority or retry a mutation. Legacy subscription callers remain during migration;
old routes cannot be removed until both clients have a verified replacement.
