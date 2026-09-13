# UX #272: integration contract and delivery ledger

Status: integrated implementation and focused Linux verification complete; broader
acceptance remains open in #272, #255 and [#273](https://github.com/o-psi/voyage/issues/273).
Tracked by [#272](https://github.com/o-psi/voyage/issues/272); corrected evidence is
[comparative review](ux-comparative-review.md) and
[tutorial benchmark](ux-tutorial-benchmark.md).

## Shared interaction contract

- Helm presents; Vessel supervises; independent Voyage processes own execution,
  canonical history, admission, policy and cleanup. This delivery improves the
  current UI. The complete framework replacement remains #216, not a hidden
  dependency or a second UI engine.
- Consequential controls identify executing host, target voyage (or unsent draft),
  and when the setting/action takes effect. Account credentials stay on that host.
- Idle Enter submits; active Enter steers the observed run. Pending admission,
  accepted delivery, failure and uncertainty have distinct language. An after-run
  queue is not promised without an owner-side queue contract.
- Stop targets an observed exact run. Detach exits observation, not execution.
  Deny, skip and Back are context-specific, not aliases for Stop. Private-terminal
  Ctrl+C reaches the child; Ctrl+] returns to Helm.
- A focused review owns its input. It names its confirm and cancel consequences,
  rejects stale targets/revisions and returns to the prior draft/reading state.
  Private authentication, terminal input and browser consent are separate scopes.
- Recovery preserves original operation identity, unsent text/attachments and
  reading position. Observation retries never replay an uncertain mutation.
  Cancellation requested is not observed cleanup or proof of process survival.
- Discovery describes actual handlers: scope, effective shortcut, availability
  and consequences. Private fields never traverse a general composer dispatcher.
- Extractive keep-N context omission is not generated summarization. Branching
  creates a distinct identity/context, not filesystem rollback. Copy uses canonical
  authored text. Diff names executing host and inspected Git scope.

## Work ownership

All implementation workers use isolated Git worktrees; the integration owner
serializes shared-shell wiring, builds, coverage measurement and publication.

| Workstream | Scope | Primary ownership |
|---|---|---|
| Onboarding | Empty account state to explicit named/default readiness | Account and draft setup |
| Outcomes | U08 / #258 denial, expiry, cancellation, unknown outcomes | Tool/approval classification and consent timeout |
| Streaming | U05 / #255 provisional/final calls and reasoning display | Provider/event/live transcript paths |
| Coding inspection | U06 canonical copy, scoped diff/file context | Dedicated inspection surfaces |
| History/context | U03/U07/U11 omission, search, anchors, branch decisions | History and context review |
| Discovery | U04/U10/U12 actions, help, effective settings | Registry/completion/discovery |
| Composer/recovery | U01/U02/U09 Stop, intent, identity and recovery | Actions, reconciliation, lifecycle/presentation |
| Front door | Complete novice tutorial and accurate auth guidance | README and setup documentation |
| Verification | X01–X16 executable evidence and explicit gaps | Focused offline journeys |
| Integration | Shared contract, focus/layout wiring, delivery | UI shell, current-state, coverage and ledger |

## Verification and completion rules

Every stream reports normal, refused, stale, interrupted and uncertain outcomes
where applicable, plus focused checks. The integrated source receives workspace
Rust coverage after final relevant edits; only its compact truthful measurement
is published. Historical shared-target objects are not current-source evidence.
Offline scripted providers and disposable workspaces are preferred. No paid
provider, credential disclosure, private browser capture or native-platform claim
is implied. Existing unrelated staged render comments and deleted scripts/test
fixtures are preserved, not silently committed or restored.

X01–X16 remain the scenario families in [the audit](ux-comparison.md). Executed,
unsupported, unverified and not-applicable must be recorded separately. Merging
features alone does not close #272. Outstanding scope stays linked and open.

## Delivery evidence

### Implemented dispositions

| Finding | Delivered outcome |
|---|---|
| Onboarding | One front-door guide, friendly named account/API setup, first-send readiness, explicit persistent host-default consent; no implicit prompt send |
| U01/U02 | Reviewed exact-run Stop; detach continues; idle Submit/active Steer/pending/delivered distinctions; no invented after-run queue |
| U03 | Raw slash and sidebar Keep-N review describes extractive reduction, retained old user text, canonical history and unknown preflight counts |
| U04/U10 | Shared searchable catalogue, availability/scope metadata, private-safe F1 overlay, draft F8 focus, unsupported-size input guard |
| U05 | Bounded separate provider disclosures, provisional tool identity/reconciliation, keyboard expansion, final collapse; detailed #255 limits retained |
| U06 | Canonical copy review, exact-result executing-host inspection and explicit Git scopes, no rollback/local remote-workspace fallback |
| U07 | Loaded-display search disclosure, previous/next matching/user navigation, older loading and reflow anchors |
| U08 | Human denial, expiry, invalidation, unavailable approval, policy refusal and unknown execution separated through reports and browser consent |
| U09 | Target/host/effective timing on consequential reviews, fixed original-command recovery and separate private boundaries |
| U11 | Genuine inclusive saved-user-message cutoff, immutable source prefix snapshot, both protocol ends, stale identity/revision/tool-group validation |
| U12 | Effective values and known provenance exposed; fixed keymaps/no general extension loader retained by explicit reasoned no-change decision |

Keymap remapping without focus-aware collision/escape checks or a general loader
without origin/permission/revocation contracts would create unsafe partial
features. This delivery does not advertise either. Workflow/runtime tool extension
surfaces remain distinct. #216's complete framework replacement is not included.

### Actual evidence

- Workspace coverage workflow after final Rust formatting: **466 passed, 0 failed,
  1 ignored** (live-provider test). Lines **35,457/95,956 = 36.9513%**;
  functions **3,320/9,382 = 35.3869%**; regions **56,292/156,854 = 35.8882%**.
  Changes versus prior: +2.0622/+1.7687/+2.0117 percentage points respectively.
  `coverage/latest.json` records dirty-source fingerprint, command and versions.
  All 11 current Cargo executables retained; 447 source files, no duplicate logical
  sources or mismatched functions. Default mixed-cache/missing-distribution-source
  diagnostics retained under `target/coverage-report/issue272-final`; corrected
  current-object export supplies the published totals.
- Integrated `cargo check -p helm -p vessel --locked -j 8` passed.
- [Sixteen focused TUI cases](ux-272-verification.md) passed on v3 with no cleanup
  errors: first task, Unicode editing, original receipt recovery, separate adversarial
  replay test, Stop/detach, inspection/copy, session drafts, historical branch,
  approval approve/deny/expiry and question choice/custom/back/skip/expiry.
- [Four streaming journeys](../streaming255-verification.md) passed on v2: local
  and scoped-loopback completion/interruption, slow interleaved Unicode arguments,
  separate reasoning disclosure, keyboard expansion, reconnect, no early effects,
  canonical reconciliation and cleanup. Later changes were inspection-only.
- [Private terminal](ux-272-private-terminal-v2.md): v3 rerun passed 14 grouped
  boundary checks including private Ctrl+C/Ctrl+], capture cutoff and observed cleanup.
- Existing scoped approval fixture updated for current synthetic named/default
  accounts and scoped account-use authority. Seven session and three workspace
  grant cases passed, including expiry/revocation/race/cancel; fixture cleanup observed.
  Earlier failed fixture preflights are retained, not called passes.
- [Browser source-function probes](ux-272-browser-operator-v2.md) passed explicit
  consent reasons, duplicate/conflicting receipts, withheld content and predispatch
  refusal. Real Chromium cleanup probe timed out with dependencies unavailable;
  no browser journey pass is claimed.
- Pinned competitor probes and source evidence remain qualified in the focused
  report. Documentation paths, action inventory, links, tables, command syntax and
  diffs checked; 772 pinned citations and 138 baseline action IDs retained.

PTY evidence used pre-formatting v3 Rust source; the final source edit was scoped
Rust formatting, followed by the workspace coverage measurement. Raw logs, fixture
credentials, PTYs and detailed reports stay in ignored/private evidence locations.
Only compact sanitized reports and reproducible focused scripts are published.

### Unfinished acceptance and publication limits

[#273](https://github.com/o-psi/voyage/issues/273) tracks the remaining broader
matrix, full Chromium, comparable competitor journeys and platform/provider gates.
#255 retains its additional provider/streaming-specific acceptance. #272 is not
closed merely because implementation merged. Images, live auth and entitlement,
separate-host/TLS, native macOS/Windows and complete workflow/private-enrollment
journeys are not certified by this delivery.

The checkout's pre-existing deleted release scripts/manifest and two broad test
fixtures are preserved. No deleted infrastructure was restored. Consequently no
release-package/installer-asset verification or update to the absent release
manifest is claimed. Existing staged rendering comments remain user work, outside
this delivery. This is source delivery, not a new released installer.
