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

# #273 X14 acceptance extension — 2026-09-13

Base `dfdecea084aab46501c3d04e618c9940d4924fff`, isolated `ux273/browser`
worktree. Read #273 scope/comments and relevant #272/#260 issue bodies through
GitHub's public API (CLI authentication was unavailable). Parent owns Rust
coverage, integration, issue updates and publication. No Rust edits/builds here.

## Executed Linux Chromium checks

Node 26.8.2, Chromium 152.0.7977.82, pinned playwright-core 1.63.0.
`npm ci --ignore-scripts --no-audit --no-fund` installed one dependency locally.
No machine configuration, personal profile, external website or real credential
was used. Chromium sandbox remained enabled, including the companion renderer.

Run from the worktree root:

```sh
mkdir -p target/ux273-browser
chmod 700 target/ux273-browser
(cd helm/browser && npm ci --ignore-scripts --no-audit --no-fund)
HELM_BROWSER_PROBE_ROOT="$PWD/target/ux273-browser" node helm/browser/probe.mjs
HELM_BROWSER_PROBE_ROOT="$PWD/target/ux273-browser" node helm/browser/cleanup-probe.mjs
node helm/browser/outcomes-probe.mjs
node helm/browser/ux272-offline-boundaries.mjs helm/browser/helper.mjs
```

Actual results:

- Expanded `probe.mjs`: **PASS**, exit 0 (`probe-v3.log`). In addition to its
  existing navigation/input/transfers/receipt/race/budget checks, real companion
  buttons dismiss sharing, deny and allow one effect, interrupt pending consent
  by human takeover, return private, and explicitly reshare. Denial and expiry
  reasons are asserted separately. The companion remains alive while Helm's
  heartbeat stops: it reports disconnect, disables sharing and revokes remote
  access. Heartbeat recovery alone cannot reshare. Explicit local reshare restores
  access. Closing the companion then revokes sharing despite continued heartbeat.
- `cleanup-probe.mjs`: **PASS**, exit 0 (`cleanup.log`). Real contexts closed in
  both fixtures. Overlapping shutdown awaits one close; synthetic rejection retains
  lock/staging and unresolved status even after helper exit. This is not an actual
  unkillable Chromium failure and does not establish parent reconciliation.
- `outcomes-probe.mjs`: **PASS**, consent outcomes and unresolved precedence.
- Offline boundaries with the required helper argument: **PASS**, six duplicate
  states, six conflicting identities, five pre-admission refusals, no dispatch.
  An initial invocation without its documented argument failed; not counted as pass.
- Node syntax, Python compilation, and Git diff whitespace checks passed.

The first real probe exposed a stale assertion: private-mode takeover now returns
`refused` / `invalidated`, not `cancelled`. Only the test expectation changed.
An initial expanded run raced the previous rendered approval button; awaiting its
removal before creating the next request fixed the test synchronization. No
runtime helper/permission/sandbox change was needed. Evidence remains private under
`target/ux273-browser`; process inspection after these runs found no matching live
fixture helper, Chromium, Helm, Vessel or Voyage processes.

## Transport runner and concrete blocker

`transport-probe.py` adapts the pre-existing local #234 one-off transport probe,
not the retired broad test suite. It uses isolated HOME/XDG/TMP, loopback website
and synthetic Responses provider, a fake env-backed account enrolled via normal
Vessel commands, and checks parent binary hashes before any product launch.
Setup installs only into the fixture's private worktree-local XDG directory.
Its intended checks include actual image/click/history transport, private marker
withholding, close and explicit reconciliation without replay; optional TUI mode
adds F6, detach cleanup and unsent draft retention. These are **not yet passing
#273 transport evidence**.

```sh
BROWSER_PROBE_RUNTIME_ROOT=/home/psi/voyage/target \
BROWSER_PROBE_BIN=/home/psi/voyage/target/debug \
BROWSER_PROBE_MANIFEST=/home/psi/voyage/target/ux273/build-manifest.json \
python3 helm/browser/transport-probe.py
# Add BROWSER_PROBE_TUI=1 for the actual TUI variant.
```

The runtime root must be short enough for Linux Unix sockets and writable; it
contains only newly allocated private fixture directories. Keep adapter installs
and other evidence inside the worktree. Do not bypass the identity check.

Observed preparation failures and fixes:

1. A mise shim refused under isolated HOME. Resolve Node's actual executable
   directory for the fixture PATH, without changing mise trust or host config.
2. Worktree-local Vessel runtime path exceeded Linux's Unix-socket limit. Allocate
   only its runtime directory in the explicitly supplied short root above.
3. Legacy config-only creation refused `default_account_required`. Updated fixture
   to enroll an explicit synthetic account and bind it in config; no real account.
4. Before executing that updated journey, the supplied manifest no longer matched
   `target/debug/voyage`. The check refused before launch. Expected hash began
   `9bdd7c4483ef`; observed hash began `3b27d0bb94b6`. Helm/Vessel hashes matched.
   Parent must supply a stable, refreshed binary attestation before rerun. The
   account adaptation and later journey/reconcile assertions remain unverified.

Full Helm/Vessel transport close/reconcile, native macOS/Windows, remote-host
routing and live-provider acceptance remain open; helper passes cannot close those
gates. Independent remote policy and local consent are unchanged. No user frames,
private receipt files, synthetic credentials or diagnostic logs are published.

## Resumed transport acceptance — v2 manifest, 2026-09-13

The stale-manifest blocker above is resolved, not a current refusal. Executed the
following against the parent's released v2 hashes (source fingerprint
`964815dac62d3137b73672aeb50e32b1db3e17d6d0917c9b1f0d9fab0dab3072`), with
Chromium `152.0.7977.82`, real Helm/Vessel/Voyage and only synthetic loopback
provider/website traffic. No Cargo or runtime/security edits:

```sh
BROWSER_PROBE_RUNTIME_ROOT=/home/psi/voyage/target \
BROWSER_PROBE_BIN=/home/psi/voyage/target/debug \
BROWSER_PROBE_MANIFEST=/home/psi/voyage/target/ux273/build-manifest-v2.json \
python3 helm/browser/transport-probe.py
# Repeat with BROWSER_PROBE_TUI=1 for the real PTY/F6 variant.
```

Harness corrections: use `start_settings` with an explicit synthetic account
binding and set the isolated host default through `account_set_default` for
reconciliation. A TOML `config_path` was invalid (the launch config is JSON), and
config-only creation did not convey the account binding. Keep Chromium TMPDIR
under the short, freshly allocated runtime fixture too: the long worktree TMPDIR
made browser startup refuse; shortening TMPDIR allowed startup without relaxing
sandbox or policy. No real account, host default or browser profile was touched.

Final executions after adding explicit cleanup observation:

- **CLI PASS, exit 0:** `browser-integration-j5d3cq36`, log
  `target/ux273-browser/transport-v2-final.log`.
- **TUI PASS, exit 0:** `browser-integration-b3t7_k5_`, log
  `target/ux273-browser/transport-tui-v2-final.log`.
- Both explicitly share from a suspended voyage, inspect/screenshot/click through
  the real transport, approve the local click, observe exactly one visible click,
  verify actual JPEG input to the synthetic provider and canonical image artifact
  history, then verify private-mode marker withholding. Command, lease and delayed
  browser-work traffic retain one duplex socket. Browser close leaves the voyage
  intact. Retained old-inner-binding reconciliation resumes the suspended owner,
  removes pending action evidence, and causes no provider/effect replay.
- TUI additionally launches via F6, exits with Ctrl+C and retains the unsent draft.
  Local companion consent is driven through its authenticated API; rendered
  companion interaction is covered by the separately recorded helper probe.
- Each final fixture's private `cleanup-audit.json` records child exits `[0,0]`,
  **zero matching live processes, zero remaining fixture loopback listeners and
  zero executor locks**. Receipts/profile evidence are retained, not deleted.
  A separate process inspection also found no matching earlier transport fixtures.

**Important intermittent gap:** first TUI fixture `browser-integration-hnkd1f6n`
failed after sharing, during the tool/consent run: companion connection refused;
TUI reported `Conversation unavailable` then unresolved cleanup/effects. Its
`cleanup.json` says local cleanup observed, but action receipts remain; local exit
is not proof of remote effect reconciliation. No lock/receipt was erased and no
uncertain effect was replayed. A fresh independent TUI fixture
`browser-integration-a3b74nzf` and the final TUI fixture both passed without product
changes. This failure's cause is **unresolved**, not a proven fixed runtime bug.
Parent should retain it as a stability investigation; successful journeys alone
do not establish reliable repeated TUI operation or real failed-cleanup recovery.
Remote-host policy, native platforms, live providers and an actual unkillable
Chromium remain outside this evidence. Prior helper-level synthetic close rejection
must not be relabeled transport failed-cleanup acceptance.
