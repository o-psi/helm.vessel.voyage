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

Only source/rustfmt/path review and `git diff --check` are completed during
preparation. **No Cargo command, compile, test or coverage export has run.**
The next cohort coordinator should integrate this commit independently of ongoing
critical artifact publication, run the Helm library `paste::family_tests` filter
under the existing bounded sole Cargo/instrumentation window, fix concrete
failures while retaining meaningful assertions, then strict applicable checks and
one independent full workspace coverage measurement after final cohort edits.
Keep source/object/profile identity and the full denominator; focused mixed totals
are not publication evidence. Keep #353's full reachable-production goal open.
