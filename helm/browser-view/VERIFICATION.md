# #333 viewer integration checkpoint

Scope: only shared `helm/browser-view/` modules and existing Helm Web console.
No commits, Rust changes, main-worktree edits, or nested agents. Parent owns
integration, workspace coverage and hosted follow-through.

Implemented: real RTCPeerConnection offer/answer and MediaStream video; complete
ICE answer; authenticated Vessel socket adapter; Start/Connect, human/private/agent
control, navigation, UUID tabs, pointer/wheel/key, IME text composer, dialogs and
viewport controls. Input bindings are captured at enqueue, sequences consecutive,
queue bounded, stale callbacks fenced, media cleared on capture/control/identity
change or loss. Document/tab/viewport changes invalidate queued input but do not
unnecessarily recreate media. Payloads never enter IntentJournal/localStorage or
connection diagnostic content. No implicit private return or reconnect.

Verified in isolated browser333-viewer checkout:
- Node 24.21.0: `node --test tests/*.test.mjs` from web: 77 passed, 0 failed,
  0 skipped (10 focused viewer/adapter tests included).
- Node 24.21.0: `node node_modules/vite/bin/vite.js build`: passed.
- `git diff --check`: passed.
- Default host Node 20 cannot load current jsdom/undici. Node 24 executable was
  installed only under ignored target/browser-viewer-tools. Existing Composer
  vendor assets reused through ignored web/vendor symlink; no dependency changes.
- Full output retained locally in ignored target/browser-viewer-tests.log.

Integration contract warning: protocol worktree now explicitly requires a fresh
caller-supplied non-nil attachment_id on Attach. Viewer follows this. At final
inspection runtime human_effect still checked `attachment_id.is_nil()` and minted
an ID, which conflicts with protocol.valid(). Runtime owner must reconcile this;
viewer does not weaken validation or send both forms. Offer expects unwrapped
`value:{type:'offer',sdp}`. Stopped status may have null mode/binding.

Actual remote WebRTC decode/end-to-end worker + runtime + Web/native integration
has NOT been verified here. The focused peer test checks negotiation and actual
ontrack MediaStream assignment using a peer test double; it is not native browser
or network qualification. Parent must complete integrated testing after runtime
and worker integration. No Rust coverage claim is made.
