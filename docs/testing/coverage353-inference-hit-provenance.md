# Inference mouse-hit source provenance (#353)

This source-only review uses the audited Rust source
`cb929ff47383bb49f82e553a291f097268873eae` and prepared event cohort
`50ccb51ff2ad4ff586af6060e21e9f57c9c42a10`. The cohort only adds cfg(test)
module declarations and tests; it does not register new production hit targets.
No Cargo, provider, human input or browser operation was performed in this review.

## Exact dormant guard

`helm/src/process_client/ui/inference.rs` defines `Controls` with derived
`Default` at 161–162 and a private `hits` vector at 178. Production App
construction in `ui/mod.rs` 261 initializes these controls with `Default::default`.
A whole `helm/src` reference review found only these operations on this specific
vector:

- `inference/render.rs` 19 clears it when retiring hit targets.
- `inference.rs` 788 clears it on resize.
- `inference.rs` 802–811 borrows/iterates/finds/maps it during ordinary left-mouse
  handling when no picker is open.

There is no production push, assignment, extension or mutable alias that installs
an element. The vector therefore remains empty throughout the current supported
App lifetime. The `.find` predicate cannot execute and the subsequent `Some(field)`
positive branch cannot pass. A test that directly inserts a private hit would not
prove an ordinary production entry to those positive effects.

The current exact zero-DA map contains guarded addresses **808, 809, 810, 813,
815, 816 and 817** in `inference.rs`. These are candidates for the precise
`structurally_dormant_current_renderer` classification with this invariant as
proof. Closing-span addresses 810/817 remain subject to region accounting; this
classification does not claim seven distinct aggregate LLVM line-gap units.
Lines 800, 802–807 and 811–812 remain ordinary executable mouse/no-match handling.
Line 811 contains both the reachable mapping operation and an uninvoked closure;
it is not classified wholly unreachable.

## Existing live controls remain ordinary

This proof concerns only the per-field `Controls.hits` vector. It does not apply
to the profile footer `options_hit` (renderer 42–44), model `chooser_hits`, picker
row `choices` (renderer 216), picker `cancel_hit` (renderer 241), profile-panel hits
or account hits. These have maintained renderer producers. The prepared cohort
uses actual rendered account connection, inference footer and cancel rectangles,
plus keyboard/model callback paths; it does not fabricate per-field rectangles.

Do not remove any source/object/address from coverage or change exclusions.
Refresh the exact map after the next audited measurement and preserve the complete
Rust denominator and unreconciled aggregate accounting. Dormant producer wiring
is a source fact, not successful UI acceptance or permission to add new controls.
The 100% reachable-production objective remains open.
