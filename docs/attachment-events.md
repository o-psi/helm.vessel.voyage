# Attachment observations and replay (partial #9)

`voyage_protocol::events` and the new `stream::Frame` variants define bounded
observations, not a production transport. Neither Helm nor Vessel opens a socket
or executes commands through this module. Their conformance integration tests
compile and exercise the same public contract.

Authenticate offers optional `features`; Welcome selects an intersection with
local support. `Features::confirm` rejects unoffered selections. Empty lists default
and are omitted, preserving existing basic v2 encodings. Existing strict v2 codecs
reject populated new fields: this is not a claim that old peers support events or
that automatic downgrade/reconnection is safe. Unknown or duplicate features and
missing dependencies fail closed. Replay, tool activity and usage depend on
sequenced events. Features are protocol support, never installation capabilities,
sharing grants, identity, or permission to execute/disclose anything.

After structural decode, adapters must call `Frame::validate_features` on both
incoming and outgoing frames with the authenticated connection's selected features.
They must independently check current authenticated principal/installation scope,
epoch, sharing and capabilities at dispatch or disclosure commit. Structural
handshake decoding does not authenticate the feature list. No cached boolean can
replace current authorization. Pre-authentication envelope size remains 256 KiB;
JSON allocations are bounded by that input ceiling, then event count/content limits
are validated. This is not streaming parsing with a pre-allocation element quota.

Events identify connection, session, run and a session-wide cursor. Zero means no
observation; emitted cursors are positive and fit the Journal's signed integer
range. Events distinguish accepted, running, cancellation requested and terminal
(completed, incomplete, cancelled, failed, interrupted). A cancellation request
is not proof of cleanup. Checkpoints carry revision only. Text deltas have a
64 KiB UTF-8 byte ceiling; tool events carry bounded labels, call IDs and fixed
outcomes, never executable calls or raw arguments/results. Usage is cumulative
per run: replace counters, never add a replayed observation again. Terminal events
and partial text do not constitute canonical history, a persisted success record,
or proof of actual process cleanup. The codec does not enforce a whole run's
lifecycle because a subscriber can join mid-run.

Replay requests specify an exclusive `after` cursor and limit 1–128. Pages carry
`after`, `latest` and positive contiguous events starting at `after + 1`. A page
may end before latest; continue from its last cursor. Empty pages require
`after == latest`. Cursors ahead of latest fail. Missing prefixes, tails, interior
gaps or evicted events require explicit `snapshot_required`, not silently skipped
records. That signal contains only requested/latest cursors, not a Session snapshot.
The existing Journal's 1024-event replay limit must be paged and byte-bounded by a
future adapter; this module does not modify its durable admission/replay contract.
An adapter also correlates replay response request IDs and honors request limits.

`EventSequence` checks connection/session scope and atomically validates a whole
page before advancing. Exact repeated last observations are deduplicated; conflicting
same-cursor or older events fail. It stores only the last event, not an unbounded
history. A snapshot-required response never advances this helper. Create a new
receiver at a cursor only after independently obtaining and applying an authorized,
consistent safe snapshot. Snapshot projection and transport recovery remain future
work. Raw Session data, provider continuation, runtime system instructions and PTY
streams have no representation here. Tool observations never replay effects.

`event_contract` tests cover golden projections, feature omission and rejection,
limits, malformed/unknown fields, gaps, paging, stale scopes, duplicate terminal
observations and atomic failure on an unnegotiated tail. These run in the protocol,
Helm and Vessel crates. Network, OS and provider behaviors are not introduced by
this slice; their future transport acceptance tests remain required.

## Global cursors and filtered projections

The Journal cursor is global within a session, not a sequence renumbered per
recipient. This slice provides no valid filtered-stream representation. Before
subscribing or returning a replay range, an adapter must establish that it can
represent and currently disclose **every** event in that range under both selected
features and sharing policy. Otherwise it must refuse that subscription/range
with an authorized fixed denial; never omit the record, send unsupported/private
content, or renumber cursors. A long-lived subscription must stop if that condition
ceases to hold. Negotiating all event features solves only representability,
not authorization. Feature subsets are usable only for wholly representable ranges.

`snapshot_required` is for an authorized retention gap, not a way to evade feature
or policy refusal. Even cursor/latest metadata requires current permission; an
adapter must not disclose private event occurrence through cursors or gap signals.
A future per-recipient projection or non-disclosing advance protocol requires its
own explicit design and tests. Tests here reject both an unsupported tail and an
omitted middle event atomically; no projection or authorization engine is claimed.
