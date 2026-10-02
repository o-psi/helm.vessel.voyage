# Ordinary Helm observation cohort (#353)

All eighteen new parent scenarios passed in the corrected coordinated full
workspace measurement of clean `1d49636e47bd42a7aef3dc4c54741209f5790335`.
This independent cohort started from frozen `4b6b744`; its source preparation
changed no parent target, profile, environment, credential or running release.
The preceding transcript/completion/#401 delivery remains separate. The input b315144
address map has 183 observer, 24 catalogue-watch, 75 reducer and 99 update
candidate addresses; those counts are not promised coverage gains. The full
denominator and 100% reachable-production objective stay intact.

## Recorded correctness fixes

The [initial scope/owner finding](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5949082597)
links this work to [release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375).
`refresh` discarded the observed owner on session-named Snapshot/Controls reads
and labelled payloads with the earlier catalogue incarnation. It now uses
`voyage_observed` and refuses a different returned owner before retaining either
payload or its cursor. The wire remains unpinned; there is no owner adoption,
authority change, mutation or effect replay. Two older scripted snapshot
fixtures now return their actual known owner instead of manufacturing nil from
the unpinned request; their existing assertions remain.

[Inner snapshot session fencing](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5949825932):
an otherwise correct outer session/owner envelope could contain another
snapshot's session and cursor. App refused its installation, but `refresh` had
already returned that foreign cursor to seed a subscription. `refresh` now
validates the inner session before returning either the payload or its cursor.
The App installation guard remains. The existing retry delay, unpinned wire
contract and exact original pending receipt remain unchanged.

[Legacy stream starvation](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5949146043):
the first 32 eligible legacy streams could remain healthy forever without
advancing the rotation counter. A fixed two-second timer now retires only groups
whose already-filtered eligible count exceeds 32. Modern authoritative
background entries stay excluded, healthy groups of at most 32 do not churn,
and no RPC timeout changes. Only actually acknowledged application cursors
survive planned retirement; rejected or dropped ACKs request a new snapshot.

[Refusal atomicity](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5949226221):
the reducer assigned earlier session fields before rejecting a later invalid
field, and initialized partial text before rejecting an offset. It now validates
all supplied known session fields and exact delta byte offset before mutation.
Supplied invalid total-message values refuse like invalid model/name values;
omitted total remains accepted without changing the count. Unknown opaque-field
policy, valid schema, monotonic/usize guards and canonical text are unchanged.

## Actual owned journeys

The new `observation_stream_journey_tests.rs` uses the existing authenticated
loopback Peer and real App factory. Its background peer speaks actual public
Command/Subscribe/Subscribed/Event/Unsubscribe frames. It permits only readonly
Capabilities, Catalogue, passive notification Attention, Snapshot and terminal
Controls; every other command fails the fixture. No executing Vessel/Voyage,
provider, browser, protected identity or human terminal is launched.

The eighteen parents exercise:

- Held same-session/new-owner snapshot and terminal replies, without mistaken
  old-owner installation or cursor advancement.
- A correct outer envelope containing a foreign inner snapshot returns no
  cursor and leaves the entire cached snapshot, draft and frozen pending receipt
  unchanged. The real observer emits no subscription for that foreign cursor;
  a valid resnapshot recovers through the existing retry delay and subscribes
  only after the correct session's observed cursor.
- Two events ordered by actual App ACK; duplicate skipping is observed through
  a following new event and exact resumed subscription cursor.
- False ACKs for invalid later name/model and every invalid total type, with
  complete snapshot/revision/cursor/metadata/draft/history preservation, followed
  by actual fresh snapshot/subscription recovery. Valid omitted total succeeds.
- Invalid delta offset with `live_text=None` preserved; accepted run state,
  UTF-8 byte-offset deltas, model/name and lifecycle changes through App ACK.
- Actual 33-session Subscribe/Unsubscribe groups serving all eligible IDs and
  carrying an already applied cursor, with no repeated snapshots. Healthy small
  and modern authoritative groups do not rotate merely because time passed.
- Replay-gap and legacy invalidation refresh, malformed event pages/cursors/
  inner session identities, stale owner/route ACK refusal and explicit owner
  transition retirement without relabelling the new payload.
- Eight concurrent held snapshot reads while actual keyboard editing remains
  responsive; a second real peer's events remain live while the first is held.
- Modern catalogue selection hydrating only the selected voyage and retiring
  its old stream, plus exact frozen pending command/incarnation/text retention
  across events and replay-gap recovery.

Each stage has a finite owned deadline. Observer task cancellation is awaited,
every active stream must issue its one actual owned unsubscribe, and the owned
socket task must terminate after explicit disconnect. Those observations prove
only fixture transport/task retirement, not Voyage/browser/native cleanup.
Canonical histories, drafts, pending receipts and mutable-job absence are
checked at their actual appropriate boundaries; catalogue projection shedding
is not mislabelled deletion of authoritative conversation history.

## First coordinated gate fixture correction

The coordinator's frozen `95b6159` full measurement produced a Helm result of
841 passed and one failed among 842 parents. The failed parent was the new frozen
pending-receipt journey; its original generic wait did not identify the stage.
That full attempt subsequently terminated unsuccessfully: 2,859 passed, one
failed and eight ignored across fifteen Cargo parent test targets. The twenty
new public-gateway and eighteen saved-owner/steering parents passed in that
attempt. Failed evidence remains retained; the original elapsed stage remains
unknown rather than being inferred from the source diagnosis.

Source inspection found that the scripted session event used revision 18, while the later cursor-21 full
snapshot inherited the helper's default revision 17. App correctly refused that
regressive canonical snapshot; its cursor could not install. The fixture now
returns the current revision 18 and the same applied name, without changing the
production guard, cursor-21 assertion, pending identity, draft, canonical history
or wait deadline. All three waits in this parent have explicit fixed stage names
so another failure identifies subscription, event ACK or gap recovery. This is a
fixture correction, not an observed production observation defect.

## Corrected coordinated gate

The corrected source passed formatting and strict workspace Clippy, followed by
the full workspace coverage command and exact current-object audit described
below. No separate focused execution is claimed. The fixture now uses revision
18 and the same applied name for cursor-21 recovery; the canonical revision
guard, original pending identity, draft/history assertions and wait deadline
remain intact. No provider, deployed browser/TLS/cost, protected identity,
macOS/Windows or stable-release acceptance follows from these tests.

## Verified measurement and remaining delivery

The corrected full workspace gate measured clean
`1d49636e47bd42a7aef3dc4c54741209f5790335`: **2,860 passed, zero failed and
eight ignored** across fifteen Cargo parent test targets. All **56** new parents
passed: eighteen Helm observation, ten saved-owner, eight steering and twenty
public-gateway cases. Child processes are not extra test parents. Formatting
and strict workspace Clippy passed.

The terminal export/audit selected **18 distinct current objects**: fifteen test
objects and three actual workspace entrypoints. It retained 499 owned profiles;
all 235 foreign profiles remained unchanged. All prior mappings were retained
across 542 workspace files plus one standard-library source. Summary, detailed
JSON and HTML totals agree, and the exact objects were archived. Lines are
**107,223 / 134,975 (79.439155%)**, functions **9,710 / 12,300**, and regions
**169,252 / 223,661**. Compared with clean BEB, covered lines increased by 606,
the denominator by 24 and coverage by 0.434924 percentage points. Accounting
reconstructed all 543 files and 12,300 function groups: 27,752 aggregate uncovered
line units versus 25,629 unique zero addresses, a difference of 2,123 = 116
maximum/union + 2,034 overlap − 27 shadowing. These are accounting categories,
not a production reachability count or exclusions.

The [compact coverage record](../../coverage/latest.json) belongs to this measured
source. Full evidence remains ignored and retained by the coordinator. Publication
and the actual hosted build/archive are pending at this checkpoint; no native
installation of this newly measured source is claimed. Existing native acceptance
remains bound to its separately recorded qualified source, including `613`.
The full measured denominator and 100% reachable-production objective stay
unchanged; [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353) and
[release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375) remain open.
