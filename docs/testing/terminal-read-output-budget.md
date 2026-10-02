# Terminal read response budget and source cursor (#404)

Prepared correction for [#404](https://github.com/o-psi/helm.vessel.voyage/issues/404),
linked to [coverage #353](https://github.com/o-psi/helm.vessel.voyage/issues/353)
and [release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375). Root's
coordinated local gate, coverage/current-object accounting, publication and hosted
build remain pending. Source preparation did not execute tests or native effects.

The full gate at `c7ffb4c` successfully started the literal Unicode PTY fixture
and observed dropped capture bytes, then its first Registry read failed with an
output-limit result. Previously the process reader spent the whole context
budget on raw captured bytes, appended a remaining-bytes footer and status, and
advanced the cursor before the oversized report was withheld by the Registry.

## Prepared contract

`ctx.max_output_bytes` bounds the complete process read string in UTF-8 bytes:
status prefix, rendered capture and any exact remaining-bytes footer. The reader
reserves the actual status size first. The capture chunk is then reduced until
its entire rendered form fits; invalid terminal bytes can expand to replacement
characters, so a source-byte limit alone is insufficient.

The cursor counts original captured bytes, not displayed characters or rendered
bytes. An accepted chunk advances through exactly the original slice that was
decoded. Its footer counts the source bytes left after that slice. Status lookup,
status-budget or body/metadata construction failure leaves the stored cursor
unchanged. An unread buffer with space for a footer but no source is refused;
an empty buffer can return only its status if that complete status fits.

A response boundary avoids splitting a known complete UTF-8 character solely
because of its output budget. Invalid source and a fragment already lost at the
capture bound retain the existing lossy display behavior. Such fragments are
not silently relabeled as valid input; the typed dropped-byte observation remains
independent. Privacy still withholds capture permanently after human attachment.
Its notice must fit completely or the read fails without cursor advancement.
This change does not alter input, capture retention, terminal ownership,
confidentiality processing or execution authority.

## Prepared verification

The real ordinary-child PTY/ToolRegistry journey retains the 256-byte budget,
900-character Unicode output, capture overflow and typed gap observation. It
waits within the existing stage limit for that exact output to be captured,
then verifies one-byte and status-only read failures do not advance the cursor.
Successful reads must fit the unchanged complete budget, move forward through
exact original slices, report remaining bytes, and introduce no replacement
character into a subsequent complete Unicode slice. Observed shutdown and owned
process retirement remain required.

Six focused byte-contract cases cover complete ASCII delivery under repeated
small reads; two-, three- and four-byte Unicode boundaries across several budgets; adversarial invalid
UTF-8 expansion with exact source consumption; metadata-only/character-too-small
refusal and empty output; complete private notice bounds; and the exact lossy
mapping of a fragment already dropped from capture. The usual standard test-file
exclusion and complete workspace denominator remain unchanged. No check is
reported passing before Root completes the coordinated gate.

## Final integrated verification

Clean measured source `6ae8774cfa1218ae010118a991de20bb4eed6fbf` passed the
full workspace gate: **3092 passed, 0 failed, 8 ignored** across15 Cargo parent
targets. All six new byte contracts and the unchanged256-byte actual Registry/PTY
journey passed. Formatting and strict all-target/all-feature Clippy passed.

The combined cohort adds120 semantic parents plus one inert child entry; nested
child summaries are excluded. Failed source/pre-test/fullgate evidence is retained,
including the c7ffb4c failure that exposed this production bug. No failing report
is labelled passing.

Audited18 current objects,570 own profiles,235 unchanged foreign profiles and all
543 prior workspace mappings. Summary/detail/HTML and archived-object LCOV agree.
Lines **109994/135808 =80.99228322337417%**, functions9932/12371,
regions173517/225021. Compared with2802782, +1222 covered lines,+81 measured
production lines,+0.852002 percentage points. Standard scope/exclusions unchanged;
100% reachable coverage remains unfinished.

All544files/12371groups/everyJSON-LCOVcounter reconcile:25814 aggregate gaps
versus23756 unique zero addresses; difference2058=136maximum/union+1949overlap
-27shadowing. This is accounting, not a complete reachability classification.

Gate invocation6bc6201f4a16469aae3d095487c174d4 exited0,567s,10GiB peak;
exportb18ce1cdda474a5cbdf161589aa2e4e0 exited0,27s,8317296640B peak;
accountingaabc92ef77014cafa58eebc3365dfedc exited0,315928576B peak.
All bounded units used zero swap and retained actual terminal evidence. Raw logs,
profiles, reports and hash-archived objects stay ignored under
`target/coverage-report/v103-ordinary-ui-helper-operator-budget-final/`; only the
compact source-bound summary is published in `coverage/latest.json`.

Normal main publication and hosted build/archive follow-through remain pending
at record creation. #404 must not close on tests/push alone. This does not
qualify native browser behavior, provider execution or a stable release.
