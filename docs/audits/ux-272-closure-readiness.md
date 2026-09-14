# #272 main-UI finish-up and closure readiness

This delivery continues the main-screen work, not a claim that every item in the
[comprehensive redesign](https://github.com/o-psi/voyage/issues/272#issuecomment-5665022513)
is finished. **#272 is not ready for unconditional closure.**

## Implemented in this pass

- The composer exposes **Model** (one options entry) and **Access**, rather than
  four competing account/model/thinking/service fields. Model opens a focused
  **Model options** panel with account and advanced next-turn settings. Choosing
  Account hands off to the existing private sign-in/default-consent flow; this
  panel never accepts credentials. `/preferences` and Helm → Model and account
  provide the keyboard path. Existing direct slash selectors remain supported.
- Application navigation and conversation work are separated. Helm contains New,
  Find, settings, connections, help and leaving. **This conversation** groups
  changes, canonical answer copy, tasks, delegated work, terminals, browser,
  workflows, lifecycle actions and Stop using existing authoritative handlers.
- A named **Paste text or image** menu action reuses explicit clipboard acquisition.
  The composer border no longer advertises a permanent shortcut dashboard, and the
  idle transcript no longer displays a second permanent shortcut strip.
- At less than 80 columns, pending questions/permissions use a wide bottom review
  rather than squeezed 20-column side panes. The conversation retains a compact
  context above it; the request shows its prompt, scoped choices, expiry,
  confirmation/back/deny/skip actions and host/draft status. Wider terminals keep
  side-by-side review. Unrelated connection-launch chrome no longer covers the
  review. No authority or decision semantics changed.
- New option/menu pointer targets and selection/incarnation guards preserve drafts,
  reject stale actions and consume paste. Private/request scopes win over the
  options panel; opening account controls does not expose private material to chat.

## Actual verification

- 174 Helm library tests passed, including new combined-options, private-account
  handoff, stale-owner, paste/draft-preservation and conversation-menu tests.
- `helm/tests/main_workspace.py` exercised real Helm with a synthetic local
  Vessel/Voyage at 120×40 and 40×18: named controls, combined options, conversation
  work menu, retained draft, no unintended submission, and a real narrow question
  answered through the UI with return to composition. Screens and result are under
  ignored `target/finish-ui/integrated-final/`.
- All 16 existing focused cases in `helm/tests/ux272_journeys.py` passed in the
  final run, recorded as **passed-partial**, not complete X01–X16 certification.
  They cover readiness, composer, recovery/dedup, Stop, inspection, session/history
  navigation, and approval/question variants. Final evidence is under ignored
  `target/finish-ui/journeys-delivery/`, with binary/source manifest.
- The first full run had one **historical fixture cleanup failure** after its
  behavior assertions succeeded: teardown signalled a newly branched runtime before
  its normal idle-suspension checkpoint. Its guardian recorded cleanup, but normal
  `stopped.json` was absent, so it was not counted passing. The fixture now waits
  for matching-incarnation, cleanup-observed idle suspension before teardown;
  the isolated rerun and subsequent full runs passed. Failed evidence remains in
  `target/finish-ui/journeys/`; no product recovery or cleanup success was invented.
- Formatting, Python syntax and documentation-link checks apply to this delivery.
  Strict Clippy still fails on the previously recorded 20 Helm warnings; no new
  options/workspace warning was reported. This is not a clean Clippy pass.
- Full workspace Rust coverage passed **477 tests, 0 failures, 1 ignored**: lines
  **38.6215%**, functions **36.7898%**, regions **37.5182%**, all increases over
  the prior record. Measured after final source/test edits with the
  complete current executable-artifact selection and committed
  `coverage/latest.json`. It does not replace native/live/browser acceptance.

## Issue disposition

| Scope | Current disposition |
|---|---|
| U01 Stop/detach/decision meanings | Implemented scoped controls and tested Linux paths; newly simplified request layout verified. Full all-context/platform acceptance is not established. |
| U02 submit/steer/recovery | Implemented and focused Linux evidence. After-run queue remains explicitly unsupported, not a hidden implementation obligation. |
| U03 extractive compaction | Implemented disclosure/review; canonical history preserved, no generated-summary claim. |
| U04 discovery and main UI | Shared discovery plus named app/conversation navigation delivered. Full contextual cards/message actions and all-screen coherence remain below. |
| U05 streaming | Delivered propagation/reconciliation and focused streaming evidence; residual matrix remains #255. |
| U06 inspection/copy | Implemented executing-host read-only scopes and canonical copy; focused fixture passed. Not per-message pointer copy, arbitrary file-click navigation or complete file-review acceptance. |
| U07 history/search | Implemented loaded-scope search, navigation and anchors; not exhaustive global search or every long-history race. |
| U08 permission timeouts | Completed in closed #258; do not reopen by listing it as absent. |
| U09 host/account/privacy | Existing scoped authority retained; combined options preserves private handoff. Separate-host/TLS/native evidence remains distinct. |
| U10 help/layout | Named main navigation, fixed shortcuts, layout guards and focused 40×18/120×40 tests. Full colour/input/accessibility matrix still unverified. |
| U11 branching | Historical-boundary review implemented; final focused branch fixture passes without filesystem rollback/inference. |
| U12 preferences/extensions | Combined inference/account entry delivered; fixed keymap/no general extension loader are documented decisions. Full provenance and future framework scope remain separate. |

## What still prevents closing the issue as currently written

1. **Full requested UI integration:** tasks/agents/programs/browser are grouped by
   conversation, but still open existing detail surfaces rather than fully
   contextual transcript/resource cards. Per-message actions, clickable change
   navigation and comprehensive continuity across every screen are not complete.
   Do not describe grouped menus as finishing that design scope.
2. **Reliability:** #274 retains intermittent browser disconnect and image-upload
   refusal diagnostics. Later successful fixtures do not resolve an unknown cause.
3. **Remaining acceptance:** #273/#255 retain unexecuted or partial variants,
   pinned competitor launch prerequisites, native macOS/Windows, actual separate
   hosts/TLS and approved live-provider/account evidence. Previous real Chromium
   passes mean missing dependencies are not still the blanket browser blocker.
4. **Framework replacement:** #216 remains a separately requested architectural
   cutover. This delivery reuses current handlers and does not pretend to replace
   App orchestration with a new framework.

Closure requires completing these obligations or an explicit user-approved
scope disposition. Merely linking them elsewhere is not completion. Keep #272
open, check only evidence-supported delivered items and retain the remaining list.

## Deployment

A build alone does not update an already-running Helm. The managed install path
publishes all four binaries atomically and updates/restarts the supervisor while
checking that prior live owners retain their identities. Apply only after reviewing
its actual plan; preserve previous releases and do not restart voyage owners.
Installation outcome and normal-command executable/hash must be observed before
claiming that `helm` is updated. Running clients still require reopening.

### Observed installation outcome

Managed installation completed with exit 0 for release
`20845d6d5722343fe6b8176e7cab7ca10877ff9c2af72bafcef77992f8d8b7bf`.
The normal `helm` command resolves to that release and matches the tested binary
SHA-256 `b25067b3dee31dd5d8a28895dc01e2825d22139987f534a871fc82c8b801de3c`.
The supervisor reports active/running; installer authenticated readiness and
prior-owner identity checks completed. The installation journal has `pending: null`
and retains the previous release. The installed four-binary directory passed the
main workspace PTY journey, including combined options, work menu and narrow
question, with fixture cleanup observed (`target/finish-ui/installed-pty/`).

The first shell installation call timed out during staging before publication;
inspection confirmed the old current pointer and no pending journal mutation.
The same reviewed installation resumed its verified staging under a persistent
terminal and completed. It was not blindly replayed after an uncertain service
change. Existing Helm processes still require reopening; no voyage-owner restart
or cancellation was requested.
