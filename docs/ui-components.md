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

### Live Working indicator

[#226](https://github.com/o-psi/voyage/issues/226) introduced a spinner in the
selected voyage's heading. [#227](https://github.com/o-psi/voyage/issues/227) adds
playful labels. [#229](https://github.com/o-psi/voyage/issues/229) moves the names
to JSON and expands the bundled set to 24, including **Pondering**, **Noodling**,
**Percolating**, **Woolgathering**, and **Wiggling gears**. Names rotate in file
order every four seconds alongside the ten-frame spinner. Names are padded to
the longest label’s display width, so neither frames nor names shift the terminal
summary. These are decorative names for the same running state, **not** specific
execution steps, token throughput, percent complete, or proof of remote liveness
between updates.

A gentle TachyonFX foreground sweep moves across the letters and back over
2.4 seconds. It blends the ordinary muted text into cyan without dissolving,
replacing, moving or hiding any characters. The effect is confined to the exact
visible word rectangle, excluding the spinner, host prefix, terminal summary,
and narrow-layout action button. Per-frame geometry is cleared before layout;
modal overlays suppress the effect. It runs before terminal color adaptation.

Waiting for input, stopping, completed, failed and interrupted states remain
static. Disconnection displays Reconnecting; a retained running snapshot without
a live process displays Status unavailable instead of continuing to animate.
`HELM_MOTION=never` and monochrome render plain **Working**, without rotating
names, spinner or shimmer.

The indicator uses the existing 100 ms redraw and a monotonic display clock—no
extra timer, background task, transcript invalidation or faster repaint is needed.
Missed animation cycles are skipped, not queued. One retained effect is sampled
within a bounded cycle; there are no off-screen buffers. Tool-result rows and
authored text are not animated, and no decorative label is stored in history or
exports. Actual terminal color capability affects how smooth the sweep appears.

### Custom working statuses (JSON)

Defaults live in [`helm/assets/working-statuses.json`](../helm/assets/working-statuses.json),
a JSON array with one string per entry. They are embedded in the binary, so an
installed Helm does not depend on a source-tree asset path. Editing bundled
source data takes a rebuild; a **runtime override requires no rebuild**:

```json
[
  "Pondering",
  "Noodling",
  "Wibbling",
  "Brewing ideas"
]
```

Save that as `working-statuses.json`, then launch:

```sh
HELM_WORKING_STATUSES="$PWD/working-statuses.json" helm connect --no-start
```

The explicit file replaces the defaults; there is no merge or automatic config
file discovery. Relative paths resolve from Helm’s launch directory. Helm reads
it once before terminal setup; restart Helm to pick up edits. There are no reads
on the animation path. Spinner frames and padding are precomputed, and shimmer
width follows the longest configured name (clipped on narrow screens).

Limits: a regular UTF-8 JSON file at most **64 KiB**, containing **1–128 strings**.
Each label must be trimmed, nonblank, **1–32 display columns**, and at most
**128 UTF-8 bytes**. Control characters, line/paragraph separators and bidi
controls are rejected, not interpreted as terminal commands. Multiword labels
and ordinary Unicode are supported. JSON comments, objects and non-string
entries are not accepted. Missing, malformed or invalid explicit files fail with
an error before entering the alternate screen—even in reduced-motion mode—rather
than silently ignoring the override. Unix FIFO paths are rejected without waiting.
All entries are decorative running-state names; they do not replace actual
waiting, failure or completion statuses.

### Effects dependency

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

### Working indicator verification (#226)

Linux locked Helm check/build and targeted formatting/diff checks passed. An
ad-hoc probe compiled the actual `effects/working.rs` source and observed ten
distinct nine-column frames, static opt-out, non-live suppression, unchanged
non-working labels, cycle wrap and skipping after a simulated stall. A second
probe extracted the actual header-state function with minimal input fixtures and
checked running, disconnection, stopped/unavailable processes, pending decisions,
waiting/stopping/terminal run states and archival precedence. These are bounded
source probes, not a restored test suite or full end-to-end/live-provider evidence.
Real-terminal visual acceptance and native-platform certification remain unverified.
The build used the concurrent working checkout; unrelated changes are not included
in this delivery.


### Playful label and shimmer verification (#227)

Linux locked Helm check/build and targeted formatting/diff checks passed. An
ad-hoc probe compiled the actual Working source and checked four padded names,
forty spinner/name combinations, fixed eleven-column width, static opt-out and
non-working/non-live fallbacks. Buffer checks observed multiple foreground shades
while symbols, backgrounds, modifiers and every cell outside the word rectangle
remained unchanged. Full-cycle/stall sampling matched; reduced motion left the
buffer untouched. Per-frame geometry reset and modal suppression through a
Ratatui test backend were also observed. This temporary probe is not a restored
automated suite. No provider usage or native-platform/real-terminal visual
certification is claimed. Builds used the concurrent checkout; unrelated edits
remain excluded from this delivery.


### JSON status verification (#229)

Linux `cargo check -p helm --locked` and `cargo build -p helm --locked` passed.
Ad-hoc actual-source probes checked the 24 defaults and 240 equal-width frames,
Unicode padding, dynamic shimmer bounds, motion/state fallbacks, and rejection of
empty/wrong-type/whitespace/control/bidi/invalid-UTF-8/oversize/over-count/over-width
input. The 128-label boundary was accepted. A runtime override replaced defaults
without recompilation; edits did not change an existing instance, and a new
instance observed them. Offline isolated PTY checks accepted a custom Unicode
file and restored the terminal on Ctrl+C; empty, missing, oversized and FIFO
files failed before terminal setup. No Vessel or provider was started.

Targeted formatting, diff, JSON asset and local documentation link checks passed.
These bounded probes are not a recreated suite, live-run visual acceptance, or
native macOS/Windows/terminal-font certification. Builds used the concurrent
checkout; unrelated work is excluded from this delivery.
