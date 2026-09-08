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

Current images remain UUID-owned inline markers and attachment metadata. There is
no image modal or automatic image-URL fetching. Optional inline thumbnails require
an agreed interaction and bounded decoding/caching before adoption.
[Preview follow-up #201](https://github.com/o-psi/helm.vessel.voyage/issues/201)
records that separate delivery scope, including ratatui-image feature review,
Chafa avoidance, terminal negotiation and protocol/halfblock/text fallbacks.
No image decoding or image protocol costs changed in this delivery.

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

Do not add tachyonfx in this delivery. Focus, selection and statuses must remain
immediately legible; there is no necessary animated state transition. Helm has no
new effect wakeups, so no effects preference or reduced-motion switch is needed.
The existing refresh interval is not a claim of smooth or optimized animation.

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
