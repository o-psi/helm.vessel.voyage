# Owned paste and image admission cohort (#353)

This source-only cohort extends the ordinary UI paste family in
[the reachability map](coverage353-reachability-map.md), which had 166 uncovered /
448 measured production lines at audited `c15ef79`. Nine new tests cover more than
30 acceptance/failure conditions using the existing private App/receipt fixtures,
owned Tokio tasks, one synthetic 1×1 PNG and a scripted loopback Vessel socket.
No native clipboard acquisition, human input, provider, browser, runtime executor,
Root/system operation or other-platform capability is added or exercised.

| Family | Substantive assertions |
| --- | --- |
| Panel late result | Rename text is sanitized and staged for explicit human confirmation; the composer remains unchanged and nothing dispatches. Changed field/owner/menu, cancellation, nontext, size limit and cleanup failure preserve the exact current field/draft. Foreign completion IDs cannot consume the pending read. |
| Panel input fence | Keys, paste, resize and mouse press cancel a matching panel read; focus/mouse movement and composer-only reads retain their stated boundary. Entering a private panel cancels the owned read. |
| Owned task retirement | A cancellation-aware task cannot report retirement until actually awaited. Finalization clears only the corresponding composer anchor and preserves text. Closed result channel, failed join and authored helper cleanup failure are errors, not successful native cleanup claims. |
| Image paste | A valid additional image preserves authored text and image order; exceeding the four-image set leaves the exact original draft/images unchanged. |
| Image-send admission | Disconnected route, pending acquisition/delivery, absent snapshot, active run, recovery, pending cleanup, deletion and archive all refuse before dispatch while preserving private text/images and existing receipt identity. |
| Receipt persistence | An owned blocking sentinel causes failure before any command dispatch/transcript delivery; the unrelated sentinel and draft remain intact. |
| Frozen image dispatch | The retained public SubmitContent preserves exact UUID/revision/authored text/ordered image metadata, excludes encoded image bytes and reopens as the same immutable original request. One refused upload produces a rejected command observation without sending SubmitContent; private draft/images and pending ID stay retained. |

The native helper lifecycle is owned by the separate clipboard cohort. These
tests supply prepared results or controlled task/channel faults, and never call
the OS clipboard reader. They do not establish successful provider image inference
or remote authenticated upload. The `begin_panel_paste` native acquisition path,
question-editor/native interaction and arbitrary storage/kernel faults retain
their independent remaining coverage/evidence obligations; this is not 100%.

## Verified cohort outcome

At clean `4c28730`, all nine focused paste/image tests passed. Compilation corrected
a missing KeyEvent import and asserted authored text from the actual ordered
SubmitContent Text parts, retaining public metadata/private-data/no-replay checks.
Formatting and strict workspace all-target/all-feature Clippy passed. Full workspace
tests passed 2,616 / zero failures / eight ignored; the corrected 17-object/381-own-
profile audit retains all 541 prior/current files and unchanged 235 foreign profiles,
with 103,430 / 133,957 lines (77.211344%). Prior/focused evidence remains archived;
the mixed default report is rejected. See `coverage/latest.json` and the gap ledger.

Remaining native acquisition/question/kernel/provider/platform and source gaps stay
explicit. This cohort does not establish 100% or full release readiness. Keep #353's
full reachable-production objective and full denominator; no skipped check is passing.
