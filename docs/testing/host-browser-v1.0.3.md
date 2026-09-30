# v1.0.3 browser qualification

The Linux x86-64 optimized candidate passed the maintained synthetic cross-client
journey with core source `31dd1150b43e716190d627ce2d556f4621881057` and Helm Web
source `c13a8c5a7e4f468eafca31e13c3ccf3c37215ebd`. This is local qualification,
not stable publication or production deployment evidence.

## Tested inputs

The three executables came from the unpacked candidate archive. Their digests
matched both `release.json` and the corresponding optimized build outputs:

| Executable | SHA256 |
| --- | --- |
| Helm | `6c68c2fb796500afa6ed571dc0c1b5f8b2c122cf633e59ffa6a7c7811009cd76` |
| Vessel | `48505d93bf2e2c3d5666bf5980abbed369e3e3929278a1599cc05f6e0a139db9` |
| Voyage | `bde5a11b73ebd7c4a89758e1e304cd3ca3716e3e67de92665fb97b5a45e18792` |

The explicitly selected checkout worker matched the packaged worker, SHA256
`199493c6545db715acfbe5cf0e1ff147dbe5e658168337f50616d2582cd200dc`.
The Web shared viewer and browser vendor snapshots matched the core files
byte for byte. The fixture reused pinned Playwright 1.63.0, existing `ws`,
Node 26.8.2 and sandboxed headless Chromium 152.0.7977.82. It downloaded no
dependencies and used fresh private HOME/XDG/state directories.

Run from the matching core checkout, substituting existing absolute paths:

```sh
python3 voyage/tests/host_browser.py --agent-interactions \
  --binaries /absolute/path/to/unpacked/archive/bin \
  --web-resources /absolute/path/to/webhelm/resources/js \
  --ws /absolute/path/to/webhelm/node_modules/ws \
  --node /absolute/path/to/node \
  --evidence-dir /absolute/path/to/ignored/evidence
```

## Observed results

The command exited zero. All three journey phases passed: live mounted/shared
viewer plus native Helm local-owner/access-file routes, fresh native preparation
after actual automatic suspension, and fresh Web preparation after suspension.
There were two independent Vessel-supervised Voyages and 38 scripted loopback
provider exchanges with no fixture errors; no real provider or inference budget
was used. Reconnecting viewers did not replay the run or request more inference.

The agent sequence exercised observed references, fill/select/check/key/double
click, child-frame inspection/fill, diagnostics, reload and read. Both viewers
then exercised ordinary first-action control, private takeover, navigation,
history/title, modal responses and text input. Assertions excluded private human
input and private URLs from the completed conversations and checked independent
Voyage state, exact owner/revision preparation, one-time native bootstrap and
explicit close/disconnect behavior.

Both site-class runs observed the external stylesheet, cookie-gated image, open
shadow content and cross-origin child replay/control. Recorded local input-to-
visible samples were 241 ms and 228 ms; these are fixture observations, not a
performance guarantee. Desktop/mobile pre-private screenshots were inspected;
the shared viewer controls and page were rendered without horizontal overflow.

Final cleanup observed no owned processes, no forced PID cleanup, and stopped
both fixture HTTP server threads. Each viewer phase observed browser closure.
Raw logs, phase reports, sanitized summaries and pre-private screenshots are
retained under ignored `target/verification-v103/final/browser-cross-client-31dd115`.
The live viewer phase report SHA256 is
`7106cb917be852fe3ba25f56d868ca40c0121b85f973601edd3db51d1e326854`.

## Remaining deployment evidence

Read-only requests to the deployed Helm Web origin passed normal TLS certificate
verification: `/up`, `/landing`, the Vite manifest and its referenced scripts
returned HTTP 200. Anonymous `/` redirected to `/landing`. The deployed React
asset had SHA256 `bcc3ea48269381157e6f3ac687427d5a5703439bddb519e28a477ca957f49684`,
which differed from the local candidate asset
`5e838a09f8d5919edb29972ec43635ce923c31111854fd441f4eb55841d16c0d`.
This does not establish that the candidate Web source is deployed.

The browser connector exposed no authenticated browser surface. Both configured
administrative SSH routes were unreachable, so no updater status command ran and
no deployment request was issued. Authenticated production TLS/client acceptance
and exact deployed-source verification remain open. These journeys also do not
establish Root-bound public transport, native macOS/Windows, arbitrary public-site
security, installer behavior or observed cleanup on other platforms.
