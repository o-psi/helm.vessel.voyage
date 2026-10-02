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
