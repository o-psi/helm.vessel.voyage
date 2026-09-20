# #333 worker checkpoint

Scope: only `voyage/browser` in existing browser333-worker worktree. No commits,
Rust builds, global installs, provider calls, or nested agents. Existing package,
lock, journal, encoder and security drafts retained/extended. Added worker.mjs
and test/*.test.mjs. CONTRACT.md is the integration notification to parent.

## Implemented

Private JSON-lines stdio, sandbox-required Playwright Chromium task process and
separate trusted encoder process, private CDP pipe, CDP JPEG -> bounded newest
frame queue -> canvas.captureStream -> actual VP8 WebRTC. No HTTP control server.
Dynamic public HTTP(S) policy through DNS-pinned authenticated proxy, explicit
private/localhost origins, live revocation generation check. Chromium proxy is
not claimed to be OS egress containment. Independent receiver peers, complete
SDP gathering, four-viewer limit. Human control/private modes, private controller
retention on disconnect, control/capture epochs, peer reset before control ack,
agent result withholding across fences, dialog interrupt lane. Structured agent
inspect/ref click/fill/scroll/navigate/screenshot/tab/upload/download and human
pointer/key/text/scroll/resize/dialog/navigate/tab input. Bounded input/line/media/
receipt queues. Fsync durable lifecycle/agent journal; monotonic volatile input
and signal ledgers instead of per-input lifetime journal exhaustion. Unknown
crash receipts never replay. Cleanup failure retains ownership lock.

## Verification actually run

`cd voyage/browser && npm test`: final run exit 0, **8 passed, 0 failed, 0 skipped**,
4.55 seconds. Node v20.20.2; Chromium 152.0.7977.82 from executing host. npm 12
warns it does not support this Node version, but test command completed normally.
No new dependencies installed; existing pinned playwright-core 1.63.0 used.

- Address classification rejects loopback/local/metadata/mapped addresses.
- Proxy with public_web=true denies loopback, permits explicit private origin,
  then denies after dynamic replacement.
- Reopened dispatched journal receipt becomes unknown; changed ID rejects;
  plaintext request payload absent from receipts.
- Frame backlog drops intermediate frames and fences queued data.
- Simulated unobserved browser cleanup retains worker.lock.
- Spawned stdio worker accepts init/shutdown, replies JSON only, rejects malformed
  JSON and exits 0 with lock removed.
- 2,100 synthetic inputs: bounded 2,048-entry volatile ledger, only one durable
  init receipt; evicted old input sequence refuses without reexecution.
- Actual task browser chrome://sandbox confirms seccomp-BPF enabled. Synthetic
  HTTP page navigation, inspect, fill, tab creation exercised.
- Actual separate receiver Chromium reports **7 decoded inbound frames** in final
  run. Initial test decoded zero with mDNS candidates; fixed by disabling mDNS
  obfuscation only in isolated encoder/receiver, not task browser. This discloses
  real host ICE candidates, explicitly documented for parent authorization.
- Two encoder peers admitted; private takeover while navigation is blocked returns
  within 3 seconds, blocked agent returns control_fenced, peer count is zero before
  ack, noncontroller viewer removed, agent inspect refuses.
- Worker pointer-up blocked by native modal completes after concurrent worker
  dialog input (interrupt lane), rather than serial deadlock.
- Private input dedup exact match succeeds without replay, changed ID refuses,
  secret sentinel absent from durable receipt files, private disconnect remains
  private, explicit controller return works. Actual browser close observed.

`node --check worker.mjs` and `node --check security.mjs` passed during iteration.
Explicit linked-worktree Git metadata status: only untracked voyage/browser/;
`git diff --check` exit 0 (untracked files not covered by Git diff). No other file
ownership taken. An initial normal-Git invocation failed due inherited metadata
path; corrected to existing explicit linked-worktree git-dir, no metadata changes.

## Remaining qualification / integration obligations

This is functional worker code with actual synthetic media, not full #333 system
acceptance. Parent still owns Rust adapter, authentication, per-socket authority,
process supervision and installation. CONTRACT documents incompatible additions:
consecutive signal_seq, full status binding, complete ICE/no trickle, human input
shape mapping. A parent passing protocol enum JSON directly will not work.

No remote NAT/TURN, native macOS/Windows, live-provider or public-internet test was
performed. No latency/CPU SLA claimed. Dynamic public policy is implemented, but
external DNS/TLS redirect/rebinding stress is not covered by these synthetic tests.
Upload/download/resize/key/error branches are implemented but not each exercised
end-to-end yet. Stdio oversized-frame/backpressure stress and abrupt SIGKILL orphan
reconciliation need parent-supervisor testing. Chromium temporary downloads can
exceed the returned-data cap before completion; supervisor disk quotas required.
OS egress isolation and process-tree kill/reconciliation remain supervisor duties.
WebSockets and service workers intentionally blocked; no arbitrary evaluate tool.

Next safe action: parent review/integration against CONTRACT, then expand the
missing action/error/stdio qualification. Do not delete a stale lock or replay an
unknown receipt to get a green test. No stable/release delivery is claimed.
