# Helm composer editing source cohort

Scope: [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353).
The audited db3d358 address map retained134 ordinary production candidates in
`helm/src/composer.rs`, especially editing-key dispatch, word/vertical movement
and bounded history. This cohort invokes those actual APIs with owned in-memory
drafts; it does not recreate the editing algorithm or use a terminal/provider.

Nineteen scenarios assert exact authored Unicode text and byte caret positions,
display-cell vertical movement, word/line/draft selection and deletion,
atomic owned image labels, text-only internal cut/yank and its insertion bound,
undo/redo invalidation of asynchronous paste anchors, count/byte history bounds,
submission cleanup, caller-owned nonediting keys, and prompt recall that retains
the unsent draft and refuses image replacement. Literal image labels remain text;
removed image ownership cannot reappear through edit history.

Production changes are only a cfg-test declaration. Source formatting/diff review
does not establish passing behavior, native clipboard or runtime delivery. The
cohort awaits the coordinated final local gate, full Rust coverage with the
unchanged denominator, publication and hosted build follow-through. Refresh the
full uncovered address map afterward; do not subtract these candidates in advance.
