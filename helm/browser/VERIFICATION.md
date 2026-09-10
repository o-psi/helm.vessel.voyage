# #234 local helper follow-up evidence — 2026-09-10

Scope: `helm/browser` only, continuing the existing isolated adapter worktree after
`11fe6cd`. The interrupted uncommitted edits were inspected and retained. No Cargo,
provider calls, parent-worktree edits, remote browser connection or general test
infrastructure were introduced. The parent owns integration and full-duplex/runtime
end-to-end verification. Issue #234 acceptance and related #9/#194/#198 were read
through the public GitHub API; CLI authentication was unavailable for issue updates.

## Changes and actual findings

- Fixed a reproduced interactive pointer bug: focusing the viewport scrolled the
  companion before coordinate calculation, making a click hit the wrong position.
  Focus now preserves scroll; the viewport follows the configured aspect ratio.
  Keyboard/mouse state is released across authority changes/blur, and the local
  input queue is bounded. Mode changes refresh tab button state.
- Local approvals now show current/destination origin, bounded page-supplied
  element signature, non-password fill value, and transfer filename/size/MIME.
  The UI uses text nodes; page descriptions cannot approve anything. Stdout approval
  events contain only request identity/pending state. Fresh references are checked
  again after approval; click/fill no longer bypass Playwright actionability.
- Strengthened binding shape/expiry-field checks and preserved idle/first-run/later
  run behavior. The actual bound session/run is shown locally. Every fence clears
  old chooser/transfer grants as well as observations; sharing is never renewed by
  heartbeat. The README spells out the parent's control-event/epoch handshake.
- Preserved filename/MIME in actual website upload payloads, consumed direct local
  chooser staging, added local transfer discard controls, and bounded/sanitized
  disclosed names to the broker's 128-byte UTF-8 limit. The current broker rejects
  zero-byte remote file disclosures: local save works, while helper disclosure
  explicitly refuses with `empty_download_local_save_only` instead of sending an
  invalid result.
- Tracked HTTP upstream sockets as well as CONNECT sockets for revocation, rechecked
  grants after DNS, and tore down connections on grant downgrade. Reused sockets
  receive one tracking listener: a larger transfer probe exposed and fixed repeated
  close-listener accumulation rather than suppressing Node's warning. Socket errors
  during teardown have an explicit handler. IPv6 documentation addresses are denied.
- Bounded local operation completion independently of control/status handling.
  Disk accounting tolerates files/directories disappearing during normal Chromium
  activity; live over-budget detection still fences/closes the context. This is
  application accounting, **not a hard filesystem quota**.
- Validated retained receipt shape/ownership/count and preserved uncertain outcomes
  on journal failure. Browser close updates readiness visibly.
- Cleanup retains the original close promise, including rejection. It cannot reply
  early to overlapping shutdown. A rejected Chromium close retains lock/staging and
  returns `cleanup_unresolved`; lock-removal errors are not silently swallowed.
  Helper exit, EOF, cancellation or force-kill alone is **not** positive cleanup
  evidence.

## Verified locally

Environment: Linux, Node `v26.8.2`, Chromium `152.0.7977.82` (Arch Linux), pinned
`playwright-core 1.63.0`.

- `node --check` for helper, security, companion, main scoped probe and cleanup probe.
- Main scoped probe passed against actual sandboxed Chromium and a synthetic local
  website, including:
  - exact Host/Origin/CSRF/session enforcement, one-use launch secret, controller
    exclusion, private-network and redirect-origin refusal;
  - real companion frames and keyboard text reaching the same viewed page, with
    that human input absent from helper stdout until explicit sharing;
  - real raster output, inspection refs, local deny/allow and bounded prompt expiry;
  - actual human-picked **and** remotely staged file bytes/name/MIME arriving at
    the website; separate stage/grant/effect confirmation;
  - actual download bytes saved locally without disclosure, separately granted and
    approved remote disclosure, empty local save with safe remote refusal, and
    local discard releasing transfer slots;
  - local-only approval metadata assertions (origins, element description, fill
    value, filenames, exact sizes and MIME), and content-free stdout events;
  - duplicate no-replay/conflict handling, stale refs, concurrent fresh raster and
    control fencing, and takeover after the navigation receipt was **dispatched**
    and the fixture website had actually received the slow request;
  - idle binding `run_id:null`, first actual non-null run, idle heartbeat retaining
    the run pin, later-run fence, explicit reshare, and wrong/malformed binding;
  - heartbeat loss and no automatic renewal, private file/receipt modes, absence of
    recording artifacts, clean profile restart and durable unknown/cancelled
    recovery from explicitly synthetic historical receipts;
  - unexpected executor lock refusal without changing that lock, and actual live
    session disk-budget detection closing Chromium.
- Cleanup probe passed both delayed overlapping-close and rejected-close scenarios.
  It instruments only private copies, not production hooks. A rejection is injected
  after actually quiescing the real Chromium context, avoiding deliberate orphaning.
  Both concurrent callers wait; rejection persists on subsequent shutdown; no
  `closed:true` appears; lock/staging remain even after Node exits. The close call
  count is exactly one in both scenarios.
- Separate live Linux `/proc` inspection observed three helper Chromium renderers
  with `NoNewPrivs=1` and `Seccomp=2`; no Chromium descendant had `--no-sandbox`.
- Pinned package/lock agreement, all seven embedded assets, companion asset paths,
  documentation paths, and `git diff --check` were checked.

Final combined run retained private evidence under the checkout's `.local/`:

- `helm-browser-probe-hu9XNf`
- `helm-browser-cleanup-5otgoO`
- Separate sandbox process inspection: `helm-browser-sandbox-dl964fop`

Earlier repeated broad passes include `helm-browser-probe-zG7jBz` and
`helm-browser-probe-gFjnUT`. Initial failed probes are not counted as passing
coverage. Raw profiles, fixture copies, staging and receipts remain private and
are not committed. The synthetic unexpected-lock/failure evidence intentionally
retains locks; these are not guessed cleanup candidates.

## Parent boundary and remaining limitations

Read-only comparison against `shared-local-browser/crates/voyage-protocol/src/browser.rs`
and the parent's browser runner/helper confirmed the typed action/result fields,
idle-run semantics, local control-event forwarding and environment-cleared private
stdio boundary. No Rust compilation or transport/runtime end-to-end success is
claimed here. The parent must verify reshare/control publication on its actual
pinned duplex connection and integrate these follow-ups; do not infer that from
helper-only probes.

No native macOS/Windows evidence, live provider execution, arbitrary public-site
compatibility certification, hard OS disk quota or security against a compromised
same-user process/Chromium is claimed. See README for the precise interaction,
network and zero-byte disclosure limits. Close rejection remains an unresolved
production cleanup obligation regardless of helper exit; these synthetic checks
do not authorize removing a real orphan lock or replaying an uncertain effect.
