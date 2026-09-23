# Browser replacement verification (#333)

The shared `viewer.mjs`/`viewer.css` are the only host-browser viewer modules
used by Helm Web and native Helm. `voyage/browser/media-next.mjs` replaces the
former encoder module. The independent opt-in local-browser feature retains its
separate consent boundary. See [the contract](CONTRACT.md),
[worker contract](../../voyage/browser/CONTRACT.md), and
[host-browser design](../../docs/host-browser.md).

Local Linux checks for this cutover:

- `npm test --prefix web`: 100 JavaScript and 32 React tests passed.
- `npm run typecheck --prefix web` and `npm run build --prefix web`: passed.
- `node web/tests/browser-next-browser.mjs` and
  `node web/tests/browser-layout-browser.mjs`: real Chromium desktop and mobile
  viewer/layout journeys passed with no horizontal overflow.
- `node --test voyage/browser/test/*.test.mjs`: 19 passed, zero failed; the
  relay-only case was skipped because `TURN_SERVER` was not set. The suite
  includes decoded WebRTC, private viewer exclusion, track resize, bounded
  input, stale fencing and crash cleanup.
- `python3 voyage/tests/host_browser.py --binaries /absolute/path/to/built/bin`:
  passed with Helm rebuilt from this checkout. Two real supervised Voyages used
  mounted Web and native Helm viewers, decoded media, private typing/dialogs,
  suspended owner preparation and observed cleanup.
- `python3 packaging/test_browser_assets.py -v`: four passed; the new media
  module is in the staged asset inventory and the removed module is absent.
- Workspace Rust coverage after final code/test edits: 1,755 passed, zero
  failed, two ignored. The corrected current-object totals are 75.0092%
  lines, 71.8912% functions and 71.6406% regions; full metadata is in
  [coverage/latest.json](../../coverage/latest.json).

This is local Linux and loopback browser evidence. It does not establish native
macOS/Windows behavior, public network/TURN connectivity, installed public-site
acceptance or a successful hosted artifact. Issue #333 retains those gates and
the exact delivery identities. The media path still uses CDP JPEG frames and a
separate trusted Chromium canvas encoder; the replacement adds bounded frame
delivery and responsive track replacement without claiming a native capture path.
