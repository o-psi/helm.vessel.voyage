# Helm component decisions

[UI adoption epic #196](https://github.com/o-psi/helm.vessel.voyage/issues/196)
selects libraries where they improve a concrete workflow. It does not mandate the
whole candidate catalogue or replace Helm's event loop.

## Delivered component surfaces

- [Semantic styles](ui-styles.md): application-owned roles, passive termprofile
  capability adaptation, explicit color overrides and non-color selection cues.
- [Scoped interaction](ui-interaction.md): private Vessel forms and navigation,
  with one focus owner, keyboard-accessible actions and visible focused controls.
- [Composer editing](ui-editor.md): Unicode editing and coordinated text/image
  state, with explicit undo retention and asynchronous paste behavior.

These guides record selected dependency features, comparisons and verification.
The bounded sanitizing CommonMark/GFM renderer remains authoritative; replacing it
with a Markdown widget would lose established content and sanitization semantics.

## Trees and scrolling

Do not add tui-tree-widget to the current voyage sidebar. Its entries are a flat,
recency-ordered set with stable voyage identities; introducing a hierarchy would
add navigation without representing a current product relationship. Revisit a tree
when a real hierarchical operator workflow is specified, with identity-backed
expansion/selection, removed-node reconciliation and fresh mouse geometry.

Do not add tui-scrollview to the transcript. Its logical message/activity anchors,
loaded history, search, streamed output and expansion state are not equivalent to
an offset into an offscreen buffer. Retain that projection and visible-area
rendering. The Vessel form uses bounded line clipping and focus reveal directly;
allocating a second width-by-height cell buffer does not improve this small form.
A future scrollview must bound both dimensions and their product before allocation,
including u16 geometry, rather than merely capping the line count.

These are completed library-selection decisions, not undelivered tree or scrollbar
features promised by this epic.

## Attachment previews

Images retain UUID-owned inline markers and attachment metadata. The subsequent
[preview delivery #201](https://github.com/o-psi/helm.vessel.voyage/issues/201)
adds opt-in Alt+P thumbnails with bounded background decoding and Kitty,
halfblock and text fallbacks. There is no image modal or automatic image-URL
fetching. See [composer previews](ui-previews.md) for dependency review,
resource bounds, passive terminal selection and measured verification.

## Private terminals

Structured terminal cells and input-mode fidelity remain under
[#32](https://github.com/o-psi/helm.vessel.voyage/issues/32). The current human-only
response contains sanitized plain screen rows; a tui-term renderer alone cannot
recover discarded styling. That work requires both protocol ends, bounded cell
transport, wide-cell/cursor semantics and preservation of private input ordering,
capture cutoff, leases and restoration. Its default vt100 adapter must not
silently introduce a competing parser version. This epic changes semantic chrome,
not the private terminal wire contract or cell serialization costs.

## Effects and component gallery

TachyonFX navigation polish was added in [#225](https://github.com/o-psi/voyage/issues/225),
following the original effects deferral in #196. Selecting a different visible
voyage briefly settles its sidebar action button from white to its normal accent
color (240 ms, ease-out). This deliberately small first effect leaves names,
status labels, transcript text, layout, input and private terminals unchanged.
It does not animate draft rows or simulate execution progress.

`HELM_MOTION=auto` (default) enables this in color terminals; `HELM_MOTION=never`
disables it. Monochrome (`HELM_COLOR=never`, or automatically detected no-color)
also disables effects. Invalid motion values fail before terminal setup.
There is no OS accessibility-preference detection; set the environment override
when reduced motion is preferred.

`helm/src/process_client/ui/effects.rs` owns one replaceable effect, using current
sidebar hit rectangles. Hidden navigation, modal panels and terminal resize cancel
it; rapid selection replaces rather than queues effects. The existing 100 ms
refresh becomes approximately 33 ms only while an effect is active, then returns
to 100 ms. Elapsed wall time advances the effect, so stalls do not stretch it.
Effects run before terminal color adaptation, without off-screen buffers or
changes to canonical content. This is not a cross-terminal frame-rate guarantee.

Selected dependency: **tachyonfx 0.25.2**, MIT, published MSRV unspecified;
default features disabled, only `std` and `std-duration` enabled. No DSL parser,
web-time or sendable effects. It shares Ratatui core 0.1.2 with Helm, adding no
second terminal backend. Its Unicode width requirement resolves 0.2.2; additional
transitives are bon/bon-macros, prettyplease, compact_str 0.10 and micromath.

Do not add tui-pantry or another gallery dependency to the shipped application.
Temporary bounded buffer/PTy probes are sufficient for this component migration;
a gallery would be optional development tooling and would not establish regression
coverage or product acceptance. Reconsider when a maintained component catalogue
has an owner and a concrete development use.

## Evidence boundaries

Verification is recorded in the linked surface guides. Linux observations do not
establish native macOS/Windows behavior, real terminal graphics protocols, live
SSH/sudo/pager fidelity or multiplexer compatibility. No live-provider usage is
required for this UI work. Broader interface acceptance remains tracked by
[#14](https://github.com/o-psi/helm.vessel.voyage/issues/14).

## Delivery verification

Linux validation for this delivery includes locked Helm checking/building, strict
Helm all-target Clippy, eight existing composer checks, eight existing attachment
checks and two temporary actual-module probes for private forms and the transcript.
The temporary probes were removed from the checkout; their evidence and the
standalone library/editor probes remain under `/tmp/epic196-*`.

A real `HELM_COLOR=never helm connect --no-start` PTY session used isolated HOME/XDG
storage and synthetic input. It exercised private form Tab/Shift-Tab traversal,
reconnect toggle, 100×30 to 55×18 resize, returning to the composer, combining/ZWJ/
wide text, selection replacement and undo/redo. The synthetic invitation was masked
and absent from persisted files. Exit returned 0 and restored the alternate screen.
A second image workflow inserted a generated PNG through bracketed path paste,
replaced the selected owned marker with text, and confirmed undo did not resurrect
it; the saved draft had the replacement text and no images or markers. An earlier
invalid-checksum PNG fixture was rejected with the prior draft intact; it is not
counted as a successful image insertion. The final valid fixture evidence is in
`/tmp/epic196-pty-wedhn0yg`; the first UI run is in `/tmp/epic196-pty-ek29motj`.

These observations used no provider request, actual OS clipboard, remote pairing,
private terminal attachment or live approval. Temporary actual-module probes cover
private pending/focus behavior; they do not establish an end-to-end remote setup or
live approval workflow. The VT parser used to inspect captures has limited joined
emoji rendering; persisted canonical Unicode was checked separately.

No package or release was produced. The checkout's pre-existing deletion of
`scripts/release-documents.txt` and packaging/quality scripts was preserved, so
archive guide inclusion was not verified. Source links and manifest dependency
membership were checked; packaging is not reported as passing.

### Navigation effects verification (#225)

On Linux, `cargo check -p helm --locked` and `cargo build -p helm --locked`
passed against the working checkout. Targeted rustfmt and diff whitespace checks
passed. Dependency inspection confirmed one Ratatui core and the selected
TachyonFX features. An ad-hoc buffer probe observed changing foreground colors
while preserving symbols, modifiers, backgrounds and all cells outside the
button; the effect finished after 264 ms of sampled steps and after a simulated
five-second stall. Offline PTY smoke checks observed `HELM_MOTION=never` and
monochrome `auto` enter/exit the TUI and restore the alternate screen on Ctrl+C;
an invalid value failed before alternate-screen entry. No Vessel or provider was
started for those probes.

These are bounded implementation checks, not a recreated regression suite,
interactive visual acceptance, an MSRV run, or native macOS/Windows/multiplexer
certification. Builds included unrelated concurrent checkout edits; only the
effects implementation and its dependency/documentation changes are delivered here.
