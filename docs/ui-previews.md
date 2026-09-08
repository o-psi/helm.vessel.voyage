# Composer image previews

Alt+P toggles inline previews of the images already owned by the selected Helm
composer. Previews are off initially. Image names, dimensions, byte sizes and
owned `[Image N]` elements remain available without previews. This is a local
display preference; toggling it does not submit, upload, change a pending command,
or alter the saved attachment. There is no preview modal or capture action.

Preview decoding consumes retained attachment bytes. It does not read the OS
clipboard again, reopen the original file, or fetch URLs. Deleting the source
file after a successful paste does not remove the saved attachment or its preview.
An unavailable or oversized preview leaves the attachment available for the
normal send path. The preview limits below are separate from image admission and
provider limits in [Images and pasted screenshots](multimodal-implementation.md).

## Terminal behavior

`HELM_IMAGE_PROTOCOL` accepts `auto` (default), `halfblocks`, `kitty`, or `text`.
Invalid values fail startup with a configuration error. Selection happens before
the terminal event reader starts, using environment variables and operating-system
window geometry. It sends no terminal capability query, spawns no helper, and
does not change tmux passthrough settings.
While previews are enabled, Helm also rereads passive pixel geometry in its
existing UI loop, so font-only changes can be noticed even when the terminal
does not deliver a changed row/column resize event.

| Setting and terminal | Display |
| --- | --- |
| `text`, or a no-color profile | Existing text metadata; no pixel graphics or colored halfblocks |
| `halfblocks` with color | Portable colored halfblock cells |
| `auto` in a directly identified Kitty terminal, with truecolor and valid pixel geometry | Kitty graphics |
| `kitty` with truecolor and valid pixel geometry, outside a multiplexer | Explicit Kitty graphics selection |
| Missing pixel geometry, tmux/screen, or other auto-detected environments | Halfblocks when color is available, otherwise text |

Automatic Kitty identification requires `TERM=xterm-kitty` and a nonempty
`KITTY_WINDOW_ID`, without SSH indicators. Explicit `kitty` still requires valid
passive cell metrics and truecolor. `TMUX`, `STY`, tmux/screen `TERM`, or
`TERM_PROGRAM=tmux` suppress native graphics even with an explicit request. The
largest native variant must fit within 512×512 pixels at the reported cell size.
No-color follows Helm's resolved color profile: `HELM_COLOR=never`, or nonempty
`NO_COLOR` with automatic color selection, uses text. An explicit Helm color
override retains its documented precedence.

The worker prepares three complete size variants, up to 16×4, 32×8 and 48×12
terminal cells, preserving aspect ratio. Rendering chooses a complete variant
that fits the available rectangle. It does not crop, decode, resize, or encode
during drawing. The display variant may enlarge a small source to fit its cell
rectangle; the preview remains separate from the original attachment bytes.
The inline preview strip needs at least 28 rows in the available composer view
and 16 columns per image; smaller views retain metadata without the strip. If
no prepared variant fits, metadata remains usable.

Native graphics reserve twelve random screen-local image IDs for four image
slots and three size variants. Transmission and targeted deletion suppress
protocol replies. Cleanup deletes only those IDs, never all terminal images.
Cached native objects are discarded with their terminal resources so a later
preview transmits fresh data rather than reusing a deleted terminal image.
Switching drafts or voyages, deleting an attachment, opening a covering dialog,
disabling previews, and exiting invalidate the applicable native state. A normal
resize reuses the prepared size variants. Changed passive pixel-per-cell metrics
trigger targeted cleanup and preparation with the new geometry, retaining the
screen's reserved ID namespace. If native metrics become unusable, the screen
falls back to halfblocks. A narrow view can hide the strip while retaining its
bounded prepared resources for a later wider view.

## Ownership and resource bounds

One dedicated worker thread owns preview decoding. It is created lazily on the
first enabled preview request with owned images; thread creation failure falls
back to an unavailable-preview notice while retaining metadata. Its mailbox holds only the
latest queued request and the latest result; stale generations cannot update the
current view. Each request contains at most four distinct owned image identities
and 2 MiB of compressed data in total. One active request and one queued request
can therefore retain up to 4 MiB of compressed preview input. SHA-256 is checked
against the retained bytes before preparing or reusing a cached preview.

PNG, JPEG and WebP use the already supported image codecs. Preview admission
limits dimensions to 8192 on either axis, a total of 4,194,304 pixels, and at most
32 MiB of declared decoded output. The decoder's 32 MiB allocation limit is
best-effort library accounting, **not an OS memory limit or a cap on total Helm
memory**. Decoder working memory, thumbnails and protocol encodings also consume
memory. The worker reduces accepted images to at most 512×512 pixels, then
flattens transparency against a neutral dark background before preparing variants.

The cache holds at most four prepared previews, keyed by image slot, owned UUID
and digest. Removing or changing the active images invalidates obsolete work.
Cancellation is checked between stages; an individual library decode call cannot
be interrupted. Shutdown joins the worker after that bounded-input operation
returns, rather than leaving an unowned background thread. Preview errors do not
become model-visible messages or expose encoded image data in diagnostics.

## Verification boundaries

Implementation sources are `helm/src/process_client/ui/previews/backend.rs`,
`decode.rs` and `render.rs`. Verification evidence is recorded in issue #201.
Two temporary Linux development-profile probes exercised the actual
decode/worker and renderer modules. Count/byte/dimension/pixel admission, alpha
flattening, invalid raster and hash refusal, cache reuse/clear, latest-generation
delivery, cancellation and joined shutdown passed. Renderer checks covered
aspect bounds, untouched surrounding cells, zero geometry, text/halfblock
fallbacks, single native transmission and scoped quiet deletion.

| Synthetic fixture | Observed duration |
| --- | --- |
| Small PNG, cold worker decode and real protocol preparation | 1.180 ms |
| Same small PNG, cached worker response | 32.160 µs |
| 4-megapixel PNG, 86,135 compressed bytes, cold worker and real protocol preparation | 413.808 ms |
| Same 4-megapixel source, cached worker response | 4.644 ms |
| Source above the pixel ceiling, refusal | 66.854 µs |
| Join while an active decode was cooperatively cancelled | 96.694 ms |

These are individual fixture measurements, exclude actual terminal display, and
are not latency or shutdown deadlines. The temporary modules were removed after
verification; they do not introduce a repository test suite.

Six isolated Linux PTY cases passed against the built Helm binary: halfblocks,
explicit Kitty with synthetic OS pixel geometry, Kitty without pixel geometry,
multiplexer fallback, nonempty `NO_COLOR`, and explicit text mode. Assertions
checked that preview toggles preserve saved draft text, markers and attachment
counts; deleting the owned image removes it; and exit preserves the next saved
attachment. The generated source file was removed before previewing, so the
successful preview used retained bytes. Halfblock cases produced pixel cells;
text cases emitted neither native graphics nor halfblocks.

The Kitty PTY case additionally verified fresh transmission after re-enabling or
returning to an image draft, no native transmission in the narrow view, and fresh
preparation after a font-metric change with unchanged row/column counts. Disable,
draft switch, deletion, font-metric change and exit with a displayed image each
emitted exactly twelve quiet, scoped deletion commands. All transmitted IDs
belonged to that same twelve-ID namespace. Alternate-screen entry and restoration
and a zero exit status were checked in all six cases.

The initial PTY fixtures exposed two measurement pitfalls: uniform or averaged
pixel rows can render as colored spaces rather than halfblock glyphs, and a PTY
child without a controlling terminal does not receive normal resize signals.
Corrected fixtures use distinct rows that fit the smallest pixel variant and a
controlling PTY. Those failed fixture results were retained alongside final
evidence rather than reported as product failures or passing checks.

Linux PTY protocol fixtures can establish emitted bytes and cleanup sequences,
but do not establish how a real Kitty terminal displays pixels or native
macOS/Windows behavior. No provider call is needed to verify this local display
path.

A third temporary actual-App probe verified the existing-voyage composer: root
rendering displayed metadata and colored halfblocks, Alt+P preserved the exact
serialized pending command and owned image state, frozen editing stayed frozen,
and owner changes, deletion, help and private Vessel setup invalidated preview
work. It used an offline client and synthetic data; no voyage process or provider
was contacted. An invalid protocol setting was also rejected before entering the
alternate screen.

Locked Helm checking/building, strict all-target Clippy, changed-source formatting,
relative documentation links and dependency graph checks passed. No package was
produced: the checkout's pre-existing packaging/quality script and release-document
manifest deletions were preserved. Archive guide inclusion was not verified.

## Dependency decision

`ratatui-image` 11.0.8 is MIT licensed and declares Rust 1.86.0. Helm disables
all its default features: no Chafa dynamic/static linking, image-default codec
bundle, example Tokio runtime or extra terminal backend feature. The crate's
build script does no Chafa discovery without those features. Direct protocol
constructors bypass every Picker constructor: even the upstream halfblock picker
can run a tmux passthrough-setting command.

The direct `image` dependency reuses version 0.25.10 with PNG/JPEG/WebP only,
matching Voyage. Ratatui core remains 0.1.2 and Crossterm 0.29.0. The resolved
lockfile adds 28 packages, including upstream's unconditional pure-Rust Sixel
encoder/quantizer even though Helm selects only Kitty or halfblocks. There is no
new dynamically linked native image library. The new packages' published license
metadata offers MIT or Apache-2.0 terms; `self_cell` offers Apache-2.0 as an
alternative to GPL-2.0-only. Some packages also offer Zlib or LLVM-exception terms.
This is dependency metadata/source review, not a full security or legal audit.

The highest declared Rust minimum among added packages is 1.90 (`quantette` and
`ordered-float`), so the resolved graph's requirement is higher than the image
widget's own declaration. Packages without a declared minimum were not assigned
one. Builds use the installed Rust 1.98 toolchain; no minimum-toolchain or native
macOS/Windows validation is claimed.
