# Helm styles and terminal color fallbacks

Terminal adaptation was introduced in [#197](https://github.com/o-psi/helm.vessel.voyage/issues/197).
The application semantic-style migration is part of
[#196](https://github.com/o-psi/helm.vessel.voyage/issues/196).

## Operator behavior

Full-screen Helm and its private-terminal chrome adapt colors to the terminal.
The plain `connect list/inspect/submit` paths are unchanged; a color override does
not make non-TTY output eligible for full-screen operation.

Set `HELM_COLOR` before launching Helm:

| Value | Behavior |
| --- | --- |
| `auto` (default) | Passive environment/TTY detection through termprofile |
| `never` | Terminal-default foreground/background, preserving bold, reverse, and other text modifiers |
| `16` | Quantize RGB/indexed colors to ANSI 16 colors |
| `256` | Quantize RGB colors to ANSI 256 colors |
| `truecolor` | Preserve original colors |

An invalid value is an error before entering the screen or attaching private
terminal capture. In automatic mode **any nonempty `NO_COLOR` value** requests
monochrome, ahead of force-color/CI detection heuristics. An explicit `HELM_COLOR`
value other than `auto` takes precedence over `NO_COLOR`. Other automatic
heuristics follow termprofile (including `TERM`, `COLORTERM`, `TERM_PROGRAM`,
`FORCE_COLOR`, and `CLICOLOR_FORCE`). Detection is an estimate, not certification;
use an explicit override when SSH/multiplexer environment metadata is misleading.
For example: `HELM_COLOR=never helm connect --no-start`.

Detection happens once at each full-screen entry, before the event reader starts.
No active terminal queries, `tmux info` subprocess, passthrough setting changes,
clipboard acquisition, or input reads are used for detection. Existing terminal
restoration and private input/capture ownership remain unchanged.

## Implementation and library decision

`helm/src/theme.rs` owns semantic roles for primary/muted text, focus/selection,
running/awaiting-input/failed/completed states, private-terminal chrome, and code.
Consumers include navigation, transcript/activity, action and approval panels,
model settings, new drafts, Vessel forms, completion, overview panels, terminal
browser/private-terminal chrome, and Markdown defaults. Selection uses reverse
and bold; existing `>` markers remain. Hover uses underline/bold (navigation rows omit underline across padding), and transcript search
matches use reverse. Status text remains explicit; colors never replace labels.
Status styles preserve attention/failure bold modifiers in monochrome. Application
colors are centralized in the semantic palette; syntax and terminal content colors
remain independent and are adapted at the completed-frame boundary.
The existing bounded/sanitizing Markdown renderer and Syntect theme are retained.

Selected: **termprofile 0.2.4**, MIT OR Apache-2.0, declared Rust 1.88.0 minimum.
Enabled features: `convert`, `ratatui`, `ratatui-underline-color`; defaults disabled.
`query-detect`, `terminfo`, `windows-version`, and `color-cache` are not enabled.
`DetectorSettings::new().enable_tmux_info(false)` is important: upstream defaults
otherwise allow a `tmux info` subprocess even without the query feature.
The resolved dependency uses the existing `ratatui-core 0.1.2`, `anstyle 1.0.14`,
and `palette 0.7.7`; the lockfile adds only termprofile, not another Ratatui or
Crossterm stack. No new native library/deployment requirement is introduced by
these selected features. The build uses the installed toolchain, not an MSRV run.

Evaluated, **deferred**: Opaline 0.4.2 (MIT, declared Rust 1.85). Its published
Ratatui adapter uses core 0.1 and its optional Syntect adapter uses Syntect 5 with
defaults off, compatible by declared requirements. It offers TOML palette/token/
style resolution and `to_syntect_theme`, useful when delivering selectable themes
and aligned syntax palettes. Its defaults also include builtin themes and
gradients. This slice has no theme picker or user theme loading, so a second theme
model, loader, and TOML 0.8 dependency would not yet deliver that benefit. Opaline
was source/manifest evaluated, not integration-built. Revisit it for the remaining
semantic migration/theme-selection work rather than conflating theme choice with
terminal capability detection.

The adaptation pass changes only final visible-cell foreground, background,
underline color, and (for a no-TTY profile) modifiers. It does not change symbols,
geometry, cursor position, hit identities, canonical text, transcript anchors, or
history allocation. Truecolor is a no-op. Other modes cost O(visible cells), not
O(history). RGB/indexed conversions are cached locally per screen/profile, up to
4,096 entries, cleared on overflow. This avoids a process-global cache lock and
bounds retained memory; repeated adversarial cache misses still require conversion.
No off-screen buffers are added. Optional [navigation effects](ui-components.md#effects-and-component-gallery)
run before adaptation and temporarily use a faster refresh while active.

## Remaining scope and evidence limits

The epic also owns real-form/focus framework comparison,
attachment-safe editor integration, trees/bounded panels, agreed preview interaction,
optional effects, and any gallery. Private-terminal structured cells/input fidelity
remain coordinated with #32; this change only styles existing sanitized presentation.
No image decoding, terminal protocol, new editor, or automated suite is added.
Native macOS/Windows and real-terminal/multiplexer certification require separate
observations. Monochrome is not a claim that every legacy widget has been redesigned
for accessibility or that every terminal supports all text modifiers.

## Verification recorded for #197

On Linux with Rust 1.98.0, `cargo check -p helm --locked` and
`cargo build -p helm --locked` passed. Targeted rustfmt and `git diff --check`
passed. `cargo tree --locked -i ratatui-core` confirmed one core version, 0.1.2.
Published crate source/manifests were inspected for the feature/license decisions
above; this was not a full dependency security audit.

A temporary, uncommitted buffer probe exercised `never`, `16`, `256`, `truecolor`,
and `auto` with `NO_COLOR=please`, checking color conversion, underline reset,
preserved symbols (including `界`), and monochrome bold/reverse modifiers. An
invalid mode returned the documented error. Across 1,000 warm passes over a
160×50-cell RGB buffer with 32 repeated foreground colors, debug-build mean
adaptation-only times were approximately 250 µs monochrome, 5,005 µs ANSI-16,
5,000 µs ANSI-256, and below 1 µs truecolor (no-op). Buffer copying was outside
the timed region. These are synthetic visible-frame costs, not end-to-end redraw,
long-history latency, cold/adversarial cache performance, or optimized-build figures.
The probe was removed from the checkout; no regression suite was introduced.

A live `HELM_COLOR=never .../helm connect --no-start` Linux PTY run observed the
conversation, F3 terminal browser with `>` plus reverse/bold selection, resize from
100×30 to 35×12 (minimum-size notice), restoration to 80×24, Escape back to the
conversation, and Ctrl+C exit code 0 with terminal restoration sequences. Captured
output retained text modifiers and emitted default color resets rather than color
assignments. No terminal was privately attached and no prompt was submitted.
A redirected plain `connect --no-start list` succeeded with no escape bytes.

This does **not** verify mouse routing, approval/question modality, private input,
editing/pending submission behavior, every legacy focus cue, or native macOS/Windows.
Those paths were not changed here and remain explicit verification work for the
corresponding epic deliveries. Existing 100 ms repaint/reset traffic was observed;
this slice adds no wakeups and does not claim to optimize that traffic.

## Semantic migration verification for #196

The source audit finds application color constants only in `helm/src/theme.rs`.
Markdown retains terminal-default fallbacks and Syntect RGB output; private terminal
rendering retains terminal content colors. These are content, not application roles.
Rustfmt and diff checks cover the migrated files. Coordinated Helm compilation and
interactive evidence are recorded with the epic delivery; the #197 PTY evidence
above predates this migration and does not establish its interactive correctness.

No theme selector or Opaline dependency is introduced: the centralized role palette
provides a single application theme, while `HELM_COLOR` selects terminal capability.
The migration changes visual styles without changing geometry, focus routing,
canonical history, persistence, approvals, or private-terminal ownership.

A temporary standalone probe compiled the actual role/adaptation module against
this checkout's existing debug dependencies. All 14 roles preserved Unicode symbols;
selection reverse/bold, hover underline/bold, search reverse, and focus/attention/
failure bold survived `never`, `16`, `256`, `truecolor`, and `auto` with nonempty
`NO_COLOR`. `never` reset every foreground/background. A rendered list and status
line retained their labels and selected-row modifiers. At 160×50 cells with a
20,000-entry list, 100 reset/render/adapt iterations averaged 6.33 ms (`never`),
6.42 ms (`16`), 6.37 ms (`256`), 5.94 ms (`truecolor`), and 6.26 ms (`auto` with
`NO_COLOR`). This includes cloning list items on each iteration; it is a synthetic
widget exercise, not a full transcript redraw benchmark or a latency guarantee.
The probe and measurements remain temporary evidence outside the repository;
no test framework was introduced.

A second temporary probe exercised the actual transcript `draw` path with 10,000
canonical Markdown messages (80,000 laid-out rows), the debug build, a 160×50
Ratatui test backend, and `HELM_COLOR=never`. Initial layout/render took 538 ms;
100 cached redraws averaged 1.16 ms, and 100 redraws anchored in earlier history
averaged 2.38 ms. Search results retained reverse styling, resize to 60×24 retained
the reading-anchor identity, and canonical message count stayed unchanged. These
measurements include test-backend drawing, not terminal transport, and cover one
synthetic transcript; cold/reflow cost remains proportional to loaded history.
The temporary probe ran alongside the private-form probe (both passed) and was
removed from the source afterward. No live provider or native-platform check ran.
