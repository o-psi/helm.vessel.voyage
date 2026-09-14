# Main conversation workspace — first redesign delivery

Tracked in [#272](https://github.com/o-psi/voyage/issues/272). This is an implemented
main-shell/transcript slice, not completion of the whole redesign.

## Current interaction

The main conversation/draft view has a named top row: **Helm**, **New**, **Find**,
and, for a selected conversation, **Changes** and **More**. **Stop** appears while
an observed run is eligible for its existing exact-run review. At narrow widths,
controls that do not fit remain available in the Helm menu. No visible F-key
launcher strip is required to discover these actions.

Click **Helm** or press **Ctrl+P** to open the menu. Up/Down or Tab/Shift+Tab moves
selection; Enter/Space opens; Escape or an outside click closes without affecting
the draft. The menu exposes search-all-actions, conversation actions, terminal,
browser, workflows, archives, settings, connections and help. Unavailable contextual
actions explain that a conversation or active run is required. Existing F-key and
slash paths remain compatible; they are secondary paths, not the main navigation
presentation.

Named controls reuse the existing handlers, not generated composer commands.
Selection/incarnation changes invalidate old menu targets. Resize and repaint clear
pointer geometry. Menu paste is consumed without entering the composer. Private
account/connection forms, workspace selection, decisions, inspection and other
modal scopes retain input precedence. The menu never attaches a terminal, shares a
browser or changes permissions merely by opening; those handlers keep their own
reviews and authority checks.

The sidebar now heads **Conversations** (or **Archived conversations**) rather than
promoting Vessel management above the work. Connections remain under the Helm
menu and the compatible private connection shortcut. Initial draft guidance is
shorter, and an unopened program inventory does not advertise stale status.
Provider-attempt diagnostics for older, unloaded turns are no longer appended
beside current output. Their records remain available through `/attempts`; when a
turn's message boundary is loaded, its failure/retry history still appears there.
Live error/retry handling and canonical history are unchanged.

## Verification

- Focused Rust App/render/input tests cover 40×18 and 100×32, named controls,
  compatibility-strip absence, menu paste isolation, draft retention, keyboard and
  pointer Find, stale selection, resize invalidation and unavailable Stop.
- Transcript regression covers unanchored successful and partial provider attempts,
  then loading their actual turn boundary and retaining failure visibility.
- All 170 Helm library tests passed in the focused run.
- `cargo build -p helm -p vessel -p voyage --locked -j 8` generated runnable Linux
  binaries. `python3 helm/tests/main_workspace.py --bin-dir target/debug --output
  target/main-ui/pty-final` exercised actual Helm through supervised offline Voyage
  fixtures at 120×40 and 40×18, with named menu/Find, retained draft, paste isolation,
  no unintended message submission and observed fixture cleanup. Captures remain
  ignored under `target/main-ui/`.
- Formatting passed. Strict Clippy is **not clean**: the default affected-package
  check stops on the existing Voyage coordination large-enum warning; the no-deps
  Helm check also exposes pre-existing large-enum, collapsible-if and test-layout
  warnings. The new workspace module's reported collapsible-if was fixed. These
  failures are not passing quality evidence and unrelated cleanup is not included.
- Workspace Rust coverage passed: 473 tests, 0 failures, 1 ignored; lines 38.2122%,
  functions 36.4099%, regions 37.1115% (all above the prior measurement). Measured
  after final source/test edits and recorded in
  `coverage/latest.json`; reused shared-target artifacts must match the complete
  current Cargo executable set. See [quality](quality.md) for scope limitations.

No live-provider, native macOS/Windows, shared-browser capture or paid-account
journey is claimed. No Rust test replaces those separate acceptance obligations.

## Remaining redesign work

The main shell now offers named access, but the full contextual result/resource
integration, single inference/account popover, redesigned decision screens and
end-to-end onboarding remain open in #272. Menus still open some existing detail
surfaces; this is not a claim that all capabilities already feel unified. Existing
running Helm processes keep their executable until explicitly restarted; building
or publishing source alone does not change a running screen.

The local managed installer dry-run would restart the active Vessel even with
`--no-start`. This UI-only delivery did not apply that service change. The runnable
client is `target/debug/helm`; reopen using that explicit path to see it. The
installed `helm` command is not claimed updated.
