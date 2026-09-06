# Remaining GitHub delivery plan

User mandate: comprehensively resolve remaining issues, including epics. Latest complete open/closed inventory: 83 issues, retained in `.local-git/issues-resume-inventory.jsonl`. Root user edits are preserved. Current development gates are Linux-only; historical native results below are not evidence for the current build; affected-platform requirements remain explicit in their tracking issues.

## Local validation and hosted CI plan (2026-09-06)

The operator approved full local Linux validation before publication and manual-only
hosted quality runs to reduce Actions time and cost. Tracking:
[#17 approved follow-up](https://github.com/o-psi/voyage/issues/17#issuecomment-5561090158),
extending [PR #100](https://github.com/o-psi/voyage/pull/100). This current policy
supersedes historical requirements below to wait for duplicate hosted CI results.
The refreshed issue inventory contains 88 open/closed issues; older inventories
below remain historical records.

- [x] Record the approved execution policy in this plan and `AGENTS.md`.
- [ ] Implement one local quality entry point shared with manual CI. Preserve
  formatting, strict Clippy, all workspace tests, locked release builds, every
  existing system fixture, evaluation-definition validation, and full/installer
  packaging with checksums. Inventory both quality workflows before consolidation.
  Use unique package labels and retain exact revision, commands, results and logs.
- [ ] After the runner is verified, replace push/PR quality triggers with
  `workflow_dispatch`; keep GitHub/Forgejo counterparts consistent. Preserve
  separate tag-only platform release builds. Do not dispatch paid reruns or create
  tags for ordinary development verification.
- [ ] Verify full step coverage, nonzero failure propagation, workflow structure,
  unique packaging and an actual complete local run; then publish implementation
  to main and verify the remote workflow definitions. Report failures honestly.

Coordinate with the other active agent before changing shared workflow files or
starting a full suite. One owner should validate each integration tree; concurrent
builds need separate target directories. Preserve ongoing work and existing logs.
Acceptance-specific failure/security tests and approved live/platform validation
remain required where applicable; local Linux success is not platform or live
provider evidence. Documentation-only changes retain their documented exception.

Implementation status: workflow triggers and the shared runner are still pending;
this planning update does not disable existing or future push/PR runs. Branch
ruleset and classic protection queries returned HTTP 403, so enforced GitHub checks
were not verified. Report any actual publication blocker without bypassing it.

Evidence: all 303 available run records were fetched, including 300 quality runs.
[Successful run 34027937732](https://github.com/o-psi/voyage/actions/runs/34027937732)
took about 17.5 minutes: release build 334 seconds and Rust tests 237 seconds,
with system checks running sequentially. No total billing estimate is claimed.
Plan-only validation checks paths, command accuracy and diffs; no runtime behavior
is changed or runtime passing result claimed by this update.

## Current product direction (2026-09-05)

The accepted [voyage model](voyages.md) names every Helm session a voyage,
including a new local chat. Helm is the program and operator interface. Planned
participation across an explicitly user-scoped set of Helms extends those same
voyages through Vessel; it does not create a separate kind of session. The
interface, coordinating agent and participants may then run on different Helms.
No repository, component map or permanent task-to-host assignment is required.
The proposed [coordination contract and implementation map](voyage-coordination.md)
specifies explicit quiescent handoff, scope drain/cancel, assignment idempotency and
consent-bound context. Concrete wire/storage integration and runtime delivery remain
unfinished; the contract is not epic completion.

Published foundations include local sessions/completion (#88), dedicated remote
worker HTTP operations (#129), workflow secrets (#130) and private policy defaults
(#131). The [worktree cleanup](worktree-cleanup.md) records their integration and
verification. The unified Helm management interface and multi-Helm coordination
remain unfinished under #77/#14/#78, with protocol, identity and privacy dependencies
#9/#10/#79. Browser console work is deferred. Workflow CLI supports explicit
[attended missing-input collection](saved-workflows.md#attended-cli-input-collection).
TUI profile switching remains open under #70.

The refreshed inventory for this documentation audit contains 83 open/closed issues,
22 open, preserved in `.local-git/evidence/product-alignment-20260905/`. The
[scope and verification plan](https://github.com/o-psi/voyage/issues/77#issuecomment-5555415622)
reuses the existing epic. Documentation alignment does not deliver runtime behavior
or satisfy semantic/live evidence under #83/#22.

All following sections are historical planning and verification checkpoints. Their
commit IDs, pending states, branch/worktree counts and issue lists describe the time
recorded. They do not override the current product model, current GitHub issue
states, Linux development policy, or actual source. Preserve their evidence without
treating obsolete browser/phase plans as current requirements.

## Historical active-work checkpoints

- Main is at e74f9b5a: the full local completion/managed stack (#88), saved workflow CLI (#123), and runtime administrator ceiling (#124) are delivered with passing Linux baselines and CI. Issues #81 and #66 are closed after acceptance review.
- Onboarding correction [#125](https://github.com/o-psi/voyage/pull/125) merged asfaa31b44 after832 normal/two-CPU tests, all23systems and bothCIruns passed; #69 closed. The inherited WaitFailed fixture correction retains every failure/retention/sanitization assertion with20 two-CPU repetitions.
- Policy documentation [#126](https://github.com/o-psi/voyage/pull/126) merged as2f4c89c8 after documentation/link/source/format verification and both complete Linux CI runs passed. Named profile CLI lifecycle and actual explicit launch selection now proceed in a separate isolated worktree; global/project defaults and TUI switching remain later scope.
- Remote execution under #77/#9/#78/#79 is implementing the actual foreground dedicated-session lifecycle, transactional public projection, cancellation receipts and authenticated reconnect. Independent authority review is active; no remote execution delivery is claimed.
- Saved workflows #67 retain TUI parameter entry and secret-input work; the actual TUI invocation/form slice is active. Completion #82/#83 retain remote output and behavioral evidence. The latest local completion evaluation failed accounting and timed out; the separate subscription path returned HTTP401.
- Questions #80 is closed after independent acceptance review and four actual local-model tool/UI/continuation/persistence cases. The small model incorrectly summarized the custom result; semantic failure remains under #22. Eight complete requests reported42,437input/156output; one prior interrupted request has unknown usage. Nine combined requests and165seconds stayed within12/360 caps. All processes reaped; no live evaluation is active. Evidence .local-git/evidence/questions-local-live/analysis.json; #22finding5554599977 and #80closeout5554600116.
- Independent review of the next slices reproduced and corrected selected-profile loss during frontend relaunch, concurrent profile-reader lock contention, and workflow preview modifier/default-navigation errors. Actual profile nested-child and PTY fixtures pass; final feature baselines are not yet complete. Remote public projection is being hardened against secrets split across stream chunks before its real-binary lifecycle fixture and final baseline.
- Draft MAIN PRs [#127](https://github.com/o-psi/voyage/pull/127) (TUI workflows, b92c70ef) and [#128](https://github.com/o-psi/voyage/pull/128) (named policy CLI, initially74ea421) are published. Full Linux baselines and CI are running. Profile work will integrate the frozen workflow head and check their interaction before final verification;127 is scheduled to merge first. The remote actual-binary fixture passes all three native transports, but crash/revocation recovery remains in development.
- PR127 local baseline is complete:849 workspace tests normally and under two CPUs, all24 system fixtures (actual policy-ceiling pass), strict quality, locked release,12 eval definitions and packaging/checksums; all32 stages exit0. Both CI runs remain pending. A separate #67 secret-input slice will use explicit run-scoped references and output-suppressed one-shot shell bindings, preserving policy and keeping values out of canonical history; design/test scope is being recorded before implementation.
- PR127 merged into MAIN1857b517f185319074b4ffe21474b5ac9ad56187 at20:56:57Z. BothCI33991014450/33991016427 passed; fetched merge tree6128b4bc equals testedb92c70ef. #67 delivery5554733074 records849 normal/two-CPU tests,24systems,32 passing baseline stages; issue remains open for secrets/plain prompting. PR128 combined workflow/profile effect-denial and preview-time deletion fixtures pass; exact confirmation now additionally binds snapshot incarnation and canonical store directory, with reproduced red/green coverage.
- PR128 final9f9a40b local baseline passed all32 stages:871 workspace tests under two CPUs,25systems, strict quality, locked release,12eval definitions and unique packaging/checksum. CI33991647502/33991646774 remains pending. Next70 private-defaults API approved; separate tests-first work may start from frozen128 while preserving its source. Secret scope67#5554736248 is active in an isolated worktree. Remote forced-death/revocation fixture passes; a deterministic pending-reply ownership race is being corrected before its freeze.
- PR128 merged MAINe74f9b5a751e9072c74fa3669c9dde140413f174 at21:08:56Z; bothCI33991647502/33991646774 passed and fetched tree51953f3a exactly matches tested9f9a40b. #70 remains open for defaults/TUI switching. Defaults implementation is isolated and uses create-only config output to preserve existing bytes; no new in-place editor or repository autoload.
- Other remaining issues retain their acceptance criteria below. Historical checkpoints are evidence of past states, not current merge blockers.

## Historical dependencies and delivery order

1. Verify existing local features (#86, #80, #59), implement P0 request safety (#64).
2. Run ownership/readiness (#81), reconciliation (#82), end-to-end verification (#83), resilient execution (#5).
3. Outbound protocol (#9), enrollment (#10), sharing (#79), coordinated sessions (#78), UI (#14), approvals (#21/#72); then attachment epic (#77).
4. Isolation (#62), per-agent limits (#37), policy profiles (#70), budgets (#71).
5. Extensions (#63/#75), MCP lifecycle (#65), local providers (#66), workflows (#67/#76), GitHub (#68), onboarding (#69), images (#74), voice (#73).
6. Packaging (#18), measured dogfood (#22), replacement epic (#19).

Each feature needs issue discussion review/update before edits, acceptance-driven tests, baseline Linux gates and packaging, and honest platform/provider evidence. Epics remain open until all exit criteria are met. No issue is considered delivered by this plan.

## Initial inventory and historical delivery checkpoints

- [#86: Allow steering messages during an active Helm run](https://github.com/o-psi/voyage/issues/86) — delivered in #87
- [#83: Validate and document completion-gate behavior end to end](https://github.com/o-psi/voyage/issues/83) — pending
- [#82: Gate final responses with bounded work reconciliation](https://github.com/o-psi/voyage/issues/82) — pending
- [#81: Track run-scoped completion readiness for subagents and todos](https://github.com/o-psi/voyage/issues/81) — pending
- [#80: Add native multiple-choice questions tool with custom answers](https://github.com/o-psi/voyage/issues/80) — pending
- [#79: Enforce session-sharing consent, retention and remote authorization boundaries](https://github.com/o-psi/voyage/issues/79) — pending
- [#78: Coordinate local and Vessel-managed sessions in the Helm runtime](https://github.com/o-psi/voyage/issues/78) — pending
- [#77: EPIC: Manage Helm sessions from Vessel through outbound attachment](https://github.com/o-psi/voyage/issues/77) — pending
- [#76: Add team-shared prompts, policies, and workflows](https://github.com/o-psi/voyage/issues/76) — closed in the latest reviewed inventory
- [#75: Publish a plugin SDK and stable extension API](https://github.com/o-psi/voyage/issues/75) — pending
- [#74: Support image and screenshot attachments in multimodal turns](https://github.com/o-psi/voyage/issues/74) — pending
- [#73: Add optional voice input to Helm](https://github.com/o-psi/voyage/issues/73) — closed in the latest reviewed inventory
- [#72: Add remote/mobile notifications and secure approval handoff](https://github.com/o-psi/voyage/issues/72) — pending
- [#71: Add resource telemetry, cost dashboards, and enforceable budgets](https://github.com/o-psi/voyage/issues/71) — pending
- [#70: Add reusable policy profiles and trust presets](https://github.com/o-psi/voyage/issues/70) — pending
- [#69: Add repo-aware onboarding and generated project guidance](https://github.com/o-psi/voyage/issues/69) — pending
- [#68: Add GitHub-native workflows for issues, pull requests, checks, and review](https://github.com/o-psi/voyage/issues/68) — pending
- [#67: Add reusable saved workflows and parameterized commands](https://github.com/o-psi/voyage/issues/67) — pending
- [#66: Add local-model and OpenAI-compatible provider presets with endpoint discovery](https://github.com/o-psi/voyage/issues/66) — pending
- [#65: Add first-class MCP marketplace, installation, and lifecycle UX](https://github.com/o-psi/voyage/issues/65) — closed in the latest reviewed inventory
- [#64: Enforce context-window limits and compact automatically before every provider request](https://github.com/o-psi/voyage/issues/64) — delivered in #87
- [#63: Add external extension and skill packaging, installation, and discovery](https://github.com/o-psi/voyage/issues/63) — pending
- [#62: Add OS-level sandbox adapters for Linux, macOS, and Windows](https://github.com/o-psi/voyage/issues/62) — pending
- [#59: TUI: Keep conversation user-facing and isolate activity/log diagnostics](https://github.com/o-psi/voyage/issues/59) — delivered in #87
- [#37: Enforce per-agent tools, approvals, budgets, and resource limits](https://github.com/o-psi/voyage/issues/37) — delivered in #87; scope and CI reviewed before closure
- [#22: Run structured dogfood and incumbent-harness cutover](https://github.com/o-psi/voyage/issues/22) — pending
- [#21: Define attended and unattended approval semantics](https://github.com/o-psi/voyage/issues/21) — pending
- [#19: EPIC: Make Helm ready to replace the incumbent harness](https://github.com/o-psi/voyage/issues/19) — pending
- [#18: Package and operate Voyage across supported platforms](https://github.com/o-psi/voyage/issues/18) — pending
- [#14: Build Vessel fleet and operations UI](https://github.com/o-psi/voyage/issues/14) — pending
- [#10: Implement Helm identity, pairing, and worker authorization](https://github.com/o-psi/voyage/issues/10) — pending
- [#9: Version and stream the outbound Helm task protocol](https://github.com/o-psi/voyage/issues/9) — pending
- [#5: Build resilient agent execution semantics](https://github.com/o-psi/voyage/issues/5) — pending

## Historical verification and blockers

PR #87 head 0489b9e: [quality run 33941455410](https://github.com/o-psi/voyage/actions/runs/33941455410) passed all jobs. Linux passed formatting, strict Clippy, 375 workspace tests, locked release build, nine system fixtures, 11 evaluation definitions, unique packaging and checksums. macOS and Windows passed workspace tests and locked release builds. Both duplicate push/PR workflow runs succeeded. The PTY resize regression also passed local normal and fragmented reads.

The first-wave source is merged. New completion-gate source has separate, unfinished verification; prior passing results do not establish its readiness. Current GitHub access works. Live subscription smoke previously returned HTTP401 before tool execution, with zero reported usage; it is not a live pass. The replacement and attachment epics retain their full unfinished scope.

## Delivery checkpoint: 2026-09-05

- [PR #91](https://github.com/o-psi/voyage/pull/91) merged as 086f0ed: current sharing authority and capability intersection. Full Linux baseline (387 tests, nine systems, release, packaging) and native macOS/Windows CI passed. #79 and #77 remain open for their full criteria.
- [PR #88](https://github.com/o-psi/voyage/pull/88), head 768c402: completion ownership, reconciliation and checkpointing. Full Linux baseline (447 tests, 14 systems, release/package), additional two-CPU tests and all native CI platforms pass. Draft pending live/operator acceptance; configured subscription provider returns HTTP 401 and login renewal has been requested.
- [PR #89](https://github.com/o-psi/voyage/pull/89), head 85a2f51: strict compiled attachment codec. Full Linux baseline passed; final steering-fixture synchronization correction passes locally and Linux/macOS CI. Current Windows CI pending. Partial #9 only.
- [PR #90](https://github.com/o-psi/voyage/pull/90), head 187a957: private native Windows enrollment storage. Linux baseline passed. Native Windows killed-writer regression exposed journal recovery failure; investigation/fix remains active. Partial #10 only.
- [PR #92](https://github.com/o-psi/voyage/pull/92), head 29a4658: run-long session fencing, dependent on #88. Full Linux baseline (454 tests, 15 systems, release/package) and two-CPU tests pass; current native Windows CI pending. Partial #78 only.

Next independent slices are explicit crash-safe session transfer with provenance and quiescent schema upgrade (#78), and negotiated attachment features with sequenced events and bounded replay (#9). Both are isolated from operator state and depend on the reviewed foundations. Neither completes the attachment epic or exposes remote execution.

Durable machine-readable status and exact logs are in ignored `.local-git/active-work.json` and `.local-git/evidence/`. CI completion must be observed at the relevant head; live evaluation definition validation is not live execution.

## Delivery checkpoint: attachment transport integration

[PR #89](https://github.com/o-psi/voyage/pull/89) merged as 0909082 after full Linux and native platform CI. [PR #93](https://github.com/o-psi/voyage/pull/93) merged as 654fd4d after 426 workspace tests, full Linux release/system/package verification and both native macOS/Windows runs. Together with merged #91, these deliver strict codec, sharing-authority and event/replay foundations; #9/#79/#77 remain open.

[PR #90](https://github.com/o-psi/voyage/pull/90) passed both native Windows tests and release builds at ec44d46 (402 native Windows tests); Linux baseline at 227e4f1 passed 406 tests and all nine systems. Integration with the merged protocol documentation required a conflict resolution, so current head 7c90033 has fresh checks running. Do not treat the earlier results as the current merge status.

[PR #94](https://github.com/o-psi/voyage/pull/94) adds explicit crash-safe transfer and schema provenance, dependent on #92. Its full Linux baseline passes 496 tests (also under two CPUs), all 15 systems, release and packaging. Source transfer still fails before writes on non-Unix; no operator state has been migrated. A later native CI failure in an inherited completion test identified an in-memory-versus-durable readiness race. Test-only 62bce48 fixes it and passes 50 two-CPU repetitions; #88, #92 and #94 now include it at heads 62bce48, 338adf5 and 64e425f respectively, with fresh native checks pending.

The real socket branch `feat/attachment-transport` now passes 464 workspace tests with both client and server fixtures enabled for Unix and Windows. Four observed regression fixes cover Ping-before-Welcome, short lease renewal, enrollment ownership through socket teardown and expired queued writes. Logging canary tests preserve application TRACE while excluding dependency raw frame/proof/prompt/history logs. Release/system/package checks and native CI are in progress. Production routing and remote execution remain unwired until authoritative coordinator and sharing boundaries are complete.

Parallel next slices are explicit enrollment lifecycle commands and read-only inspection (#10), and private native Windows Journal/database/sidecar storage (#78). The configured live provider still returns HTTP 401; login renewal was requested and live acceptance remains blocked. No additional issue or epic is closed by these partial foundations.

## Delivery checkpoint: native verification and enrollment CLI

Both current CI runs for #88 (62bce48), #92 (338adf5), and #94 (64e425f) pass on Linux, macOS and Windows. The completion live/operator acceptance remains blocked on provider authentication; dependencies stay draft. Native checks for #90 at 7c90033 are still running.

[PR #95](https://github.com/o-psi/voyage/pull/95) passes the full Linux baseline with 464 workspace tests, all nine system fixtures, optimized build and development packaging, plus a separate two-CPU workspace run. Native Windows CI found four Vessel socket test failures; investigation is active and these are not treated as passing verification.

[PR #96](https://github.com/o-psi/voyage/pull/96) adds native Windows journal storage, dependent on #94/#90. Full Linux verification at e5a1c5d passes 533 workspace tests, all 15 system fixtures, strict quality checks and development packaging. Native CI remains in progress.

[PR #97](https://github.com/o-psi/voyage/pull/97) exposes enrollment lifecycle commands and read-only inspection, dependent on #90. Full Linux verification at f6ccc3e passes 438 workspace tests and all ten system fixtures, including the real enrollment CLI fixture, plus strict quality, optimized build and package checks. Native CI has started. Pending-transaction offline disable and Windows console interruption remain explicit unfinished criteria; #10 is open.

The next #78 dependency is a persistent ManagedSessionOwner holding journal execution authority across idle periods, turns, terminal persistence and cleanup. Its issue scope is recorded at https://github.com/o-psi/voyage/issues/78#issuecomment-5552976793. Subsequent frontend integration must address persisted steering transitions, authoritative revision/usage accounting, metadata operations and truthful recovery; an ownership API alone does not complete the workflow or epic.

## Delivery checkpoint: enrollment storage merged and recovery completed locally

[PR #90](https://github.com/o-psi/voyage/pull/90) merged as 2c2d687 after both complete CI runs at 7c90033 passed every Linux/macOS/Windows job. #10 remains open. Current root documentation edits from concurrent work are preserved.

[PR #98](https://github.com/o-psi/voyage/pull/98), dependent on #97, fixes offline detach with uncertain enrollment operations. It preserves original transaction/keys, prevents recovery from silently reconnecting, and permits explicit subsequent server revocation. A tests-first regression reproduced the old rejection. At 811f201 the full Linux baseline passes 441 workspace tests, all ten real system fixtures, strict quality checks, optimized build, evaluation-definition validation and unique development package/checksum. Native CI remains pending.

[PR #99](https://github.com/o-psi/voyage/pull/99), dependent on #96, adds the persistent managed-session owner. Independent review found no blocking authority/accounting defect and prompted an actual aborted-admission race test. Twenty-three targeted runtime tests pass; final full Linux verification and native CI are in progress at 6bac8d3. Complete frontend, steering and metadata integration remains required.

PR #95 test-only b1bc9bf fixes functional socket fixtures that used deadlines below the production contract; a controlled delayed-auth regression reproduced the failure. All 465 workspace tests and 180 transport stress executions pass, with fresh native CI pending. PR #96 test-only 8b4b285 makes raw Windows failure-injection connections use the live journal's PERSIST mode, preserving the intended fault assertions. New native journal security/crash tests passed in the earlier run, but the failed overall run is retained and current CI must pass.

Parallel next work is secure terminal invitation input (#10) and a reviewed design for receipt-aware durable steering (#78). Live completion acceptance remains blocked on authentication; no broad epic is marked complete. All optimized builds and package archives remain development artifacts preparing for the first product release.


## Current verification policy and delivery checkpoint

The operator removed required macOS and Windows CI because of cost. [PR #100](https://github.com/o-psi/voyage/pull/100) merged as 5f9126a and makes both GitHub and Forgejo quality workflows Linux-only. Existing Linux quality, system fixtures and packaging checks remain required. Prior native results above remain historical evidence; pending native checks are no longer delivery blockers. Tag packaging was not changed. Active feature worktrees contain this policy change, and obsolete native runs were cancelled.

[PR #97](https://github.com/o-psi/voyage/pull/97) merged as 5c34cca. Pending-operation detach [PR #98](https://github.com/o-psi/voyage/pull/98) now targets main and awaits Linux CI; its full local baseline passed 441 tests and ten systems. Hidden terminal enrollment [PR #101](https://github.com/o-psi/voyage/pull/101) targets main with a complete Linux baseline: 438 tests, eleven systems including actual PTY cancellation/restoration and enrollment recovery, optimized build, evaluation definitions and package checks. These remain partial #10 work.

The managed owner [PR #99](https://github.com/o-psi/voyage/pull/99) passed 543 tests, including a two-CPU run. Subsequent review found stale admission time could accept a request that expired while waiting for ownership. The `fix/managed-admission-clock` follow-up samples time inside the admission transaction, preserves existing duplicate receipts despite clock failure, and has a failing-before/fixed regression plus passing targeted and workspace checks; release/system verification is in progress. Scope: https://github.com/o-psi/voyage/issues/78#issuecomment-5553109884.

Two observed inherited fixture races are fixed locally throughout the completion/session stack: plain-mode handoff now waits for the TUI's actual finished frame, and functional child shutdown uses production deadlines while explicit timeout tests retain short deadlines. Focused stress validation and fresh Linux CI remain to be completed before claiming the updated stack verified.

Transport [PR #95](https://github.com/o-psi/voyage/pull/95) integrated the enrollment CLI. Its refreshed Linux suite exposed an enrollment lock cleanup failure. A deterministic retained-descriptor regression confirms Unix flock remains held after the client drops; an explicit unlock guard is being verified. Failed evidence is retained; this check is not considered passing. Scope: https://github.com/o-psi/voyage/issues/10#issuecomment-5553196582.

Independent #78 work continues on persistent local attribution identity and durable managed steering. Local identity review found and reproduced a related error-path lock cleanup issue, now fixed with its final Linux baseline running. Steering passed a first full 563-test/15-system baseline; historical receipt-ID reservation and malformed-record bounds are being completed before final delivery. Neither identity nor steering confers execution authority or completes the frontend integration.

Live completion acceptance for #88 remains blocked by the configured provider's HTTP 401 and the pending login-renewal request. No broad epic is closed. Unrelated concurrent root documentation edits remain preserved.


## Delivery checkpoint: verified Linux stack and local managed commands

[PR #98](https://github.com/o-psi/voyage/pull/98) merged as 145c933. [PR #103](https://github.com/o-psi/voyage/pull/103) merged into the managed-owner feature branch as 8c7f998 after independent review and both Linux CI runs passed. Its tree exactly matches the verified admission-clock tree: 547 tests, 15 systems, release and packaging. This corrects the known owner-admission defect; #99 remains draft for its completion dependency.

Current Linux CI passes for #88 (367105d), #92 (38512b8), #94 (7f553f2), #96 (d646013), #99 before clock integration (5ba574d), #102 (958a915), and #103 (bdc9fb2). No macOS/Windows gate remains. The handoff fixture passed 20 two-CPU repetitions using the freshly verified admission-clock binary. An earlier stress attempt used a different `outcomes-root` TUI build and is retained as invalid branch-verification evidence, not a regression in the current stack.

[PR #104](https://github.com/o-psi/voyage/pull/104) adds durable steering, now based on the owner branch after #103 integration. Its published tree matches the tested tree: 567 tests, 33 runtime tests under two CPUs, 15 systems, strict quality, optimized build, evaluation definitions and package/checksum all pass. Historical receipt identities are reserved against new authority/command IDs without rewriting copied canonical history. Linux CI is running.

[PR #101](https://github.com/o-psi/voyage/pull/101), now cf34f6b with merged pending detach, passes all 441 tests and 11 system fixtures including actual hidden terminal input/recovery. [PR #95](https://github.com/o-psi/voyage/pull/95), now 15eeb70, passes all 475 tests and ten systems after the explicit enrollment-lock fix and pending-detach integration. Both complete Linux baselines include release/evaluation/package checks, and current Linux CI is running. Earlier failed evidence is preserved.

The next usable #78 slice is private local managed CLI create/list/submit/cancel plus explicit recovery. Backend work owns schema5 cancellation intents and bounded metadata catalogue; frontend work owns configured local policy, foreground execution, output, and cross-process cancellation. Cancellation acknowledgement means requested, not stopped; durable terminal transactions must win over stale in-memory outcomes, and terminal-looking events must wait for that result. Session create is explicit/create-only; no automatic transfer or remote sharing is introduced. An independent review is checking cancellation publication and cleanup races. Remote protocol exposure remains dependent on current sharing/authorization fences.


Dependency maintenance is tracked in [#105](https://github.com/o-psi/voyage/issues/105): resolve the confirmed transitive lru soundness advisory through the maintained Ratatui dependency path. Preserve pre-update dependency evidence; acceptance requires no affected locked lru version plus full Linux/TUI/system/release/package verification. This is independent of the managed-session coordinator work.


## Delivery checkpoint: dependency advisory and observed cleanup

[PR #101](https://github.com/o-psi/voyage/pull/101) merged as 73f18e1 after its 441-test/11-system baseline and both Linux CI runs passed. [PR #95](https://github.com/o-psi/voyage/pull/95) merged as 9bb3f86 after 475 tests, 11 system fixtures and both Linux CI runs. These provide enrollment and authenticated transport foundations; production routing and broad #9/#10/#77 acceptance remain open.

[PR #106](https://github.com/o-psi/voyage/pull/106) merged as e3d4d1b after the full integrated 475-test/11-system Linux baseline and both CI runs. Ratatui 0.30.2 resolves lru 0.18.4; real PTY tests found and now cover the upstream fullscreen cursor-query compatibility change. GitHub reports #105 closed and Dependabot alert 1 fixed.

The steering fixture follow-up in #104 (07d8734) waits for the worker before probing the actual OS execution lease. Fifty two-CPU repetitions passed, each executing the intended test. Earlier failed probes injected SQLite contention and remain recorded as invalid synchronization attempts. Current CI is running.

Managed catalogue/cancellation/cleanup obligations passed 579 tests and 15 system fixtures. The terminal cleanup helper passed 606 workspace tests plus an additional privacy regression, all 16 current Linux system fixtures and the release/package baseline. Focused draft PR publication and integration are in progress. Managed CLI integration retains root/subagent resource managers, checks durable cancellation before publishing terminal outcomes, and preserves cleanup blockers when observation is uncertain.

Managed shell cleanup scope and acceptance plan: https://github.com/o-psi/voyage/issues/78#issuecomment-5553399036. A separate adapter preserves local command policy and retains unreaped Linux session leaders through bounded cleanup; independent regression tests cover output, cancellation, background commands, limits and admission races. This is application-level observation, not containment of deliberately detached sessions (#62). Full integrated CLI verification remains pending. The completion live-provider HTTP 401 blocker is unchanged.


## Delivery checkpoint: focused cleanup PRs

[PR #104](https://github.com/o-psi/voyage/pull/104) now passes both Linux CI runs at 07d8734. [PR #107](https://github.com/o-psi/voyage/pull/107) publishes only the terminal helper against steering; both Linux CI runs pass on its actual focused head 07aa346 (581 workspace tests and 15 stacked-base fixtures). The larger earlier integration-tree counts remain historical evidence, not the focused-head count.

[PR #108](https://github.com/o-psi/voyage/pull/108) adds managed shell ownership against #107 at d349f60. Fifteen independent tests and 596 workspace tests pass with strict quality checks; release/system/evaluation/package verification and CI are running. An isolated red run reproduced both oversized-deadline defects, including an actual command effect before worker panic. The fixed implementation rejects an unrepresentable execution deadline before spawning and handles oversized shutdown waits conservatively. Scope remains observed Linux session membership, not OS containment.

[PR #109](https://github.com/o-psi/voyage/pull/109) publishes catalogue/cancellation/cleanup obligations against steering at 25b5bfc after its 579-test/15-system baseline and final targeted lease-fixture verification. CI is pending. An explicit reconciliation follow-up addresses cancelled tool calls whose outcomes are unknown: guarded operator action, exact revision and run selection, local attribution, completed cleanup, immutable evidence and no tool replay. Ordinary recovery must not silently invent a result.

The managed CLI integration now includes current main's dependency fix and root/child shell managers. Real-process fixtures pass for native transports, exact retries, signals/crashes, provisional output, output backpressure and terminal/background shell cleanup. Final integrated acceptance remains pending reconciliation and the full baseline. Independent issue #80 audit is beginning against refreshed issue history and current main. No broad epic is closed; the live-provider HTTP 401 blocker remains.


## Delivery checkpoint: managed CLI and production presence

[PR #111](https://github.com/o-psi/voyage/pull/111) merged as 5a1dc6e. Questions answers now preserve their typed envelope while redacting escaped field values exactly once. Regressions reproduced schema corruption, escaped-secret disclosure and double redaction. All 481 workspace tests, eleven real system fixtures, strict Linux quality, release, evaluation definitions, packaging/checksums and both current-head Linux CI runs passed. Full #80 remains open for its live-provider acceptance.

Current Linux CI passes for #88 at 7d76db8, #108 at 680c561, #109 at 25b5bfc, #110 at f040b28 and #112 at 8b0782a. The inherited shutdown fixture keeps its deliberate 200ms failure assertion, then uses the production five-second deadline after releasing persistence. Fifty two-CPU repetitions and 596 workspace tests passed on the integrated shell tree; no production deadline was weakened.

[PR #110](https://github.com/o-psi/voyage/pull/110) adds explicit guarded unknown-tool reconciliation after terminal execution and observed or explicitly attested cleanup. It preserves canonical history, never replays tools, binds immutable receipts to the original actor/run/revision and rejects malformed or out-of-range evidence. The production baseline passed 592 tests and fifteen system fixtures; final independent concurrency coverage brings the count to 593, with strict quality and both Linux CI runs passing.

[PR #112](https://github.com/o-psi/voyage/pull/112) exposes local managed create/list/submit/cancel/recover/upgrade commands. Root and child terminal/shell resources retain cleanup obligations until observation completes; durable cancellation wins over provisional success. Exact retries observe existing work without resending, and ambiguous tool outcomes require explicit reconciliation. The full baseline passed 690 tests both normally and under two CPUs, eighteen system fixtures, release and packaging. Final integrated ancestry adds one independent test: 691 tests and both Linux CI runs pass. Effectful MCP and the optional Codex bridge remain rejected before admission until their cleanup adapters exist. These PRs remain draft for parent completion/live acceptance, not native-platform testing.

Production presence under [#9](https://github.com/o-psi/voyage/issues/9#issuecomment-5553519620) and #10 is frozen at b115877 in the isolated attachment-presence worktree, including merged #111. It mounts authenticated heartbeat-only Vessel connections, adds an explicit foreground Helm command and bounded operator presence metadata, and exposes no session or execution dispatcher. Tests reproduced a direct lease-expiry defect and cover revocation, generation isolation and authentication/shutdown races. The actual-binary fixture passed enrollment, heartbeats, restart, signals, blocked/broken output, stalled authentication, exclusive identity, revocation and offline detach. The full Linux baseline is running; no final readiness claim yet.

Independent work continues on durable inert consent records (#79) and explicit local-provider presets/probes/setup (#66). The latter has passed a tiny tool-free local Ollama probe; it does not resolve the separate subscription-provider HTTP 401 hold. No broad epic is closed, no native development test is required, and unrelated root edits remain preserved.


Production presence is now published in [PR #113](https://github.com/o-psi/voyage/pull/113) at d9abf3a. Its complete Linux baseline passes 488 workspace tests, twelve system fixtures, strict quality, locked release, eleven evaluation definitions and unique packaging/checksums. The final commit updates guides only; their relative links and diffs are verified. Current Linux CI is running. Issue evidence: https://github.com/o-psi/voyage/issues/9#issuecomment-5553677048.

The full 83-issue open/closed inventory was refreshed and read again from `.local-git/issues-presence-followup.jsonl`, including previously truncated batches. Durable consent review found no concrete blocker and added pending-witness/retry combinations; its full baseline is running. A bounded local llama.cpp completion evaluation is being prepared against the verified managed CLI runtime. Its provider-neutral results will be kept separate from failed subscription authentication and full cutover requirements.


## Delivery checkpoint: presence merged and local live findings

[PR #113](https://github.com/o-psi/voyage/pull/113) merged as 8abe55f after its full Linux baseline and both current-head CI runs passed. This completes the explicit heartbeat-only presence slice; broad attachment issues remain open.

[PR #114](https://github.com/o-psi/voyage/pull/114) publishes inert durable consent against #102 at 1750fe1. Its full preintegration baseline passed 624 tests and seventeen systems; current-main integration passes 631 tests, strict quality, release, four affected systems and fresh packaging, retaining the other thirteen prior results. Independent recovery review passed; current CI and parent integration remain pending.

Bounded local llama.cpp evidence under [#83](https://github.com/o-psi/voyage/issues/83#issuecomment-5553723591) ran eleven requests in 214 seconds with 24,990 reported input and 821 output tokens. Clean/no-plan completed in one request with matching canonical/ledger evidence. The verification task produced the correct 42-versus-99 report but failed todo accounting; reconciliation then exposed a compatible Chat template rejecting consecutive leading system messages. Helm correctly sealed interruption. Four failed server responses omitted usage; this does not prove zero server work. All owned processes were reaped. Full completion acceptance and separate subscription authentication remain open.

[PR #115](https://github.com/o-psi/voyage/pull/115), head 157f23c, addresses that serializer compatibility defect without changing canonical history or role authority. Three tests reproduce and fix it; the full 491-test/twelve-system Linux baseline passes, with CI pending. [PR #116](https://github.com/o-psi/voyage/pull/116), head cded5f1, publishes explicit local-provider presets/discovery/setup. Full preintegration baseline passed 488 tests and twelve systems; final main integration passes 495 tests normally and under two CPUs, strict quality, release, affected fixtures and packaging. Tiny tool-free Ollama/llama.cpp probes pass; they do not establish full gate/model behavior. Independent final discovery/auth review and CI are pending.

Repository onboarding under [#69](https://github.com/o-psi/voyage/issues/69#issuecomment-5553702369) is implemented in isolated `feat/repo-onboarding`. It produces bounded deterministic reviewable drafts, reports unverified commands and preserves existing guidance. Fourteen targeted tests and the initial real CLI lifecycle fixture pass. Independent review reproduced staging-path substitution and in-place mutation; anonymous Linux descriptor publication fixes both. Other platforms currently retain inspect/preview/diff but fail direct publication before effects until an equivalent adapter exists. Full baseline is pending, and portable-publication research is active. No issue is closed for this partial status.


## Delivery checkpoint: reviewed onboarding and publication

[PR #115](https://github.com/o-psi/voyage/pull/115) merged as cfd5ace after both Linux CI runs passed. [PR #114](https://github.com/o-psi/voyage/pull/114) now passes both Linux CI runs at 1750fe1 and remains draft for its parent dependency.

Independent review reproduced unvalidated bytes replacing named staging during local-provider setup. [PR #116](https://github.com/o-psi/voyage/pull/116) now uses retained anonymous descriptor publication at 16f644f. Its corrected full baseline passes 502 tests normally and under two CPUs plus thirteen systems; subsequent Chat integration passes 505 tests in both modes, strict quality, release, affected fixtures and fresh packaging. Current CI is running. Direct setup publication remains explicitly Linux-only.

[PR #117](https://github.com/o-psi/voyage/pull/117) publishes repository onboarding at 730b53b against main. The full final Linux baseline passes 511 workspace tests, thirteen system fixtures, fmt/strict Clippy, locked release, eleven evaluation definitions and unique packaging/checksums. It uses exactly the same reviewed publication helper as #116. Final independent CLI review and CI are pending; #69 remains open for its documented publication limitation. Evidence is retained under `.local-git/evidence/repo-onboarding/final/`.

The completion-stack integration includes #112 and current main. An initial full workspace run exposed test-fixture SQLite contention in direct revocation; the isolated test passes. A bounded Busy-only fixture retry preserves the operation identity and assertions; full verification is rerunning before any new bounded local-model evaluation. Earlier failing evidence is retained.

Policy-profile work under [#70](https://github.com/o-psi/voyage/issues/70) is scoped first to a resolver/provenance foundation with a fixed protected Linux ceiling, conservative escalation metadata and security tests. It must not claim runtime enforcement until existing builders consume the result. No root-owned machine configuration is mutated for tests. All work retains Linux-only development gates.


[PR #116](https://github.com/o-psi/voyage/pull/116) merged as 4884d868 after both Linux CI runs passed. [Issue #66](https://github.com/o-psi/voyage/issues/66#issuecomment-5553900624) records delivery and the remaining Linux-only direct-publication limitation.

Final onboarding review reproduced two integration defects: uppercase guidance could shadow existing lowercase instructions, and accepted guidance could exceed the runtime loader limit. PR #117 now contains red/green fixes at 893cde1: both active filenames respect either existing spelling and the shared 64 KiB loader bound, while sidecar bounds remain unchanged. A directory lock serializes cooperating active-guidance writers; arbitrary external concurrent edits still require coordination. Eighteen targeted tests, strict quality, locked release and expanded actual-CLI coverage pass. The full final baseline and corrected-head CI are running; old known-defective CI runs were cancelled.

Completion integration passes 707 tests normally and under two CPUs, but actual cancellation fixtures intermittently retain cleanup obligations or fail to observe a durable partial checkpoint. These are failing acceptance checks under investigation, not evidence of successful completion. Private diagnostic instrumentation is temporary and must be removed before final verification. No new live run has started.

Policy foundation 7c5bff0 passes seventeen targeted tests and 508 workspace tests; full release/system baseline continues before main integration and draft publication. Independent security review is active. The next runtime slice must distinguish administrator environment-name restrictions from profile inheritance settings so explicit operator environment overrides are preserved unless the protected ceiling excludes them.


## Delivery checkpoint: onboarding merged and deterministic contention coverage

PR #117 merged b108491; issue69 delivery is recorded at https://github.com/o-psi/voyage/issues/69#issuecomment-5553965497 . PR #119 at e4f09f6 fixes attachment fixture setup without changing production authorization: prepare both enrollments before live capacity assertions, retain bounded stable-UUID revocation retry, and explicitly test database-unavailable fencing without resurrection. All43 Vessel tests,150 exact two-CPU runs,524 current-main workspace tests,strict quality and real presence passed; independent review passed and CI is running. The original policy-PR CI failure is retained.

The managed watcher now retries only DatabaseBusy inside its existing five-second read deadline. Independent review also caught stop arriving during retries; deterministic tests require clean stop between completed reads without abandoning an in-flight blocking read. A separate deterministic Journal test proves Busy COMMIT rollback leaves all durable rows unchanged and later single append creates one event/sequence increment. These do not introduce automatic mutation replay.

Raw SQLite queries in the partial-output and child-provider fixtures collided with checkpoint commits. Captured errors prove DatabaseBusy, not unexplained model failure. Fixture checks move to durable output acknowledgement / initial root dispatch respectively, retaining all canonical, cleanup and terminal assertions. Full integrated verification and bounded live evaluation remain pending; temporary diagnostic instrumentation must not ship.


## Delivery checkpoint: policy foundation merged and bounded live contract findings

PR #118 merged e3e652ed after current-head CI passed. Its runtime follow-up is implemented and independently reviewed in isolation: before-effects ceiling resolution, reused-agent preflight, parent-restricted child/worktree authority, exact Git-command approval and preserved original Config. A reviewed command-policy regression reproduced actual Git invocation before denial and is fixed. The actual namespace CLI/TUI fixture passes without host `/etc` mutation. Preserving redaction of environment values removed by a ceiling is a further targeted fix before the full baseline. Existing non-Linux ordinary runtime behavior is retained; the new ceiling feature is Linux-only.

[PR #120](https://github.com/o-psi/voyage/pull/120),2ddff52, stacks on #112 and publishes managed Busy-only polling, stop-between-read handling, deterministic COMMIT rollback coverage and corrected fixture synchronization. Production3950fef passes713 tests normally and under two CPUs,all19 systems,strict quality,release,12 evaluation definitions and package/checksums. Independent review passed. Earlier failures are retained; runtime diagnostic instrumentation is removed.

The one authorized local verification follow-up finished after178.36seconds/10requests,allHTTP200,with41,678 reported input and941 output tokens. Both owned processes were reaped. It did not pass: six invalid completion reads,zero accounting/evidence entries,one unresolved obligation,then context preflight refusal and durable Interrupted. A correct99-versus42 artifact does not substitute for accounting or prove unsupported claims about independent counting. This resolves the observed transport-template failure only; full completion acceptance remains open. Evidence reviewed directly in `.local-git/evidence/completion83-integration/live/{analysis,cleanup}.json`; issue83comment5554048792.

The captured invalid calls expose a real tool-contract mismatch: advertised completion schema requires only action and permits all union fields, but strict action decoders require/reject different fields. A separately scoped schema/description parity fix is next, preserving the argument protocol and every ownership,revision,fingerprint,evidence and strict-rejection guard. No further live calls are currently authorized to children.


## Delivery checkpoint: managed integration and action contracts

[PR #120](https://github.com/o-psi/voyage/pull/120) passed both Linux CI runs and merged into the draft #112 feature branch as 41d38b1. This did not merge managed execution to main. Parent #112 remains draft for broader completion acceptance.

[PR #121](https://github.com/o-psi/voyage/pull/121), a5f42ba, supplies exact per-action completion schemas, valid examples and independent schema/decoder boundary tests without changing runtime authority or decoding. Full Linux verification passes 716 workspace tests normally and under two CPUs, all nineteen system fixtures, strict quality, locked release, twelve evaluation definitions and package/checksums. Independent review passed; current-head CI is pending.

The same failed live trace motivates a separate todo contract follow-up under [#83](https://github.com/o-psi/voyage/issues/83#issuecomment-5554146220). Tests reproduce status accepting irrelevant text in the advertised schema, and the clear_completed unit decoder ignoring an extra ID. The latter conflicts with historical #48 and can archive more items than the caller specified. Explicit decoder hardening will reject extra fields before store mutation; error guidance must name the available block action instead of internal set_blockers. No further live evaluation has started.

Policy runtime verification at 429dfea passed 557 tests and all fifteen system fixtures. Subsequent independent review found old hydrated worktree leases could invoke Git outside newly restricted roots; 37f39cc adds per-operation root checks and preserves denied cleanup evidence. A further TUI boundary fix checks current policy before saving canonical input and retains the dispatch check. Final baseline will be refreshed after these changes; earlier baseline evidence is retained separately.

Saved workflow work under [#67](https://github.com/o-psi/voyage/issues/67#issuecomment-5554109838) now includes typed TOML discovery, exact repository digest trust, bounded single-pass rendering, advisory-only recommendations and ordinary checkpointed execution. It integrates actual #112 plus current main and passes targeted real CLI failure/cancellation, metadata, denial and trust coverage. Independent source review found no blocker. Secret declarations are inspectable but rejected for preview/run; TUI discovery and transient secret binding remain unfinished, so #67 stays open. Final integration depends on the policy runtime delivery and a refreshed full Linux baseline.


PR #121 passed both Linux CI runs and merged into draft #112 as 68f5c9a; [delivery evidence](https://github.com/o-psi/voyage/issues/83#issuecomment-5554178130). Main is unchanged by this feature-branch merge. Parent #112 at its prior 41d38b1 also passed both Linux CI runs; new parent-head CI remains separate.

Policy TUI preflight is frozen at 5cc5704 after red/green actual PTY reproduction: changed ceiling refuses provider/model requests and durable user input, preserves the Unicode composer across resize, and restores terminal modes. Root independently reviewed this and the old-lease fix. Final full Linux baseline is running.

Todo schema review removed examples that matched the live evaluation's filenames/expected numbers before delivery; production examples now use unrelated illustrative shapes. This prevents the contract correction from embedding benchmark answers. Targeted contract verification is in progress before final baseline.


## Operator-requested PR closure batch

Operator-requested PR delivery has closed eight additional PRs by merge, with results observed on GitHub:

- #114 -> #102: c93d52ea6655e443b1448ae4ceccd866692d877a
- #108 -> #107: 767a5bcb8d91915f78ad3b2f0f93d76c95bc6bad
- #110 -> #109: a9413737021653220bae877612808d094268a073
- #92 -> #88: b994f48b0b4e7a6f7cf6520db9e748bdfcc79b2a
- #94 -> #88: d3d535dfaac115474055b1d386e3ae0c006f4dba
- #96 -> #88: 9131c91140df34e3ccbdce2ba39077fa60c0c952
- #99 -> #88: ec718314b50f84ce35ec0af1886ef7ffb730fc79
- #104 -> #88: b536b014c77a941299d8178d34763ba3763fc7bc

Each original reviewed head had successful Linux CI and recorded acceptance-focused baseline evidence. The first three merge trees exactly match their tested child trees. The five merges into #88 retain tested production code plus the previously verified owned-shutdown fixture correction; the complete resulting gate test file matches the passing #112/#121 baseline. Current parent checks remain separate.

#112 now targets accepted catalogue parent #109. A final local-stack integration is being prepared to combine current main, consent, cleanup, schema corrections and accepted ancestry before a single final Linux baseline. No main merge or epic completion is claimed by these feature-parent deliveries.

Todo PR #122 at9149502 passes723 tests normally and under two CPUs, all19 systems, strict quality, locked release,12 eval definitions and packaging/checksums. Current Linux CI is pending. Docs-only655eb6d corrects stale completion acceptance lists, preserves exact evidence and distinguishes local helper delivery from broader live/remote/subscription acceptance.


## Corrected destination: deliver the combined local features to main

The operator clarified that feature-parent PR closure is insufficient: the intended destination is main. PR #88 now carries the combined reviewed local stack at2275e4e124a2267835e9347f2f38c358a48cd13e, with runtime frozen at31453fade3e8278e2a6232ad284cc86639e59c00. The last commit changes only acceptance prose to distinguish main delivery of completed local functionality from open remote/live/subscription work. Scope: https://github.com/o-psi/voyage/issues/78#issuecomment-5554238354 .

Independent integration review found no concrete blocker. Initial fmt/strict Clippy and provider34/consent29/schema10/CLI25 targeted tests pass. Full workspace tests pass; final two-CPU/release/all21 system fixtures/eval/package baseline and current-head Linux CI are running. Main merge will follow those checks. Superseded #88 branch CI runs were cancelled to avoid redundant cost; only2275e4e is the intended delivery head.

The new policy runtime feature is independent and no longer blocks this completed local stack. Its nested-child authority correction29e2876 passed independent review and targeted tests, with a separate full baseline underway. It will integrate actual main after the local stack lands, including explicit review of managed builder and child authority. No native macOS/Windows gate or false epic closure is introduced.


## Main delivery observed

PR #88 merged to main as995a9373358fc1635610eb0c1c8261184853a77f; fetched origin/main exactly matches reviewed2275e4e. Final runtime31453fa passed787 workspace tests normally and under two CPUs, all21 system fixtures, strict quality, locked release,12 evaluation definitions and packaging/checksums. Both final CI runs passed:33987190445 and33987192439.

Verified every remaining old PR head (102,107,109,112,122) is an ancestor of main. Those redundant tracking PRs were closed with explicit incorporation comments; no code or branch was deleted. GitHub now reports zero open PRs from that stack. Issue81 is closed after independent seven-criterion acceptance review;82/83 retain their explicit remaining remote/live criteria.

The policy and saved-workflow features now resume integration against actual main for separate main-targeting delivery. They no longer block delivery of the completed local stack.


## Workflow main delivery and next concrete gaps

PR #123 merged to main as c41268f5fe8a31c57ede3ef458dbcf7a3a63e177 after799 workspace tests normally and under two CPUs, all22 systems, strict quality, locked release,12 evaluation definitions, packaging/checksum and both final CI runs passed. The nonsecret workflow CLI uses the shared checkpointed executor;67 retains secret/TUI/input-form acceptance. Delivery: https://github.com/o-psi/voyage/issues/67#issuecomment-5554402498 .

Independent acceptance audit closed66 (all explicit preset/setup criteria, including two preserved real-runtime probes, met). Its documented Linux-only direct publication is a feature limitation, not an invented universal-platform acceptance requirement.69 remains open for a substantive reproduced gap: existing instructions requiring pnpm can still yield npm candidates without conflict warnings. A bounded guidance-aware correction is tests-first in an isolated worktree; arbitrary prose will not execute or gain authority.

Policy PR #124 integrated the managed main runtime and passed808-test/22-system baseline, but actual two-CPU CI exposed a fixture needing two concurrent workers while retaining an active parent. The fix explicitly grants two fixture slots with unchanged five-second deadlines/security assertions;20 exact two-CPU repetitions passed. Earlier poison-diagnostic fixture scheduling was separately corrected with exact poison/incomplete assertions retained. Failed evidence is preserved. The policy branch is integrating actual workflow main before its final baseline and CI; no main delivery yet.

The one bounded post-schema local-model evaluation timed out after300.007seconds/5requests. Four completed responses reported24,430 input/448 output tokens; fifth stream incomplete with missing usage. An artifact contained supplied99/42 values and correct hashes, but malformed todo evidence id/text objects were rejected; zero evidence/accounting/completion calls. Harness SIGTERM exited Helm-15; server0; both reaped. Raw Session remains Provisional and ledger Open, preserved honestly as termination/recovery evidence rather than claimed sealed interruption. Evidence .local-git/evidence/completion83-main-live/analysis.json; results https://github.com/o-psi/voyage/issues/83#issuecomment-5554402413 . No further live is currently planned.


## Policy enforcement delivered to main

PR #124 is merged into main as 8dfc9f05f00035e2f5bbd4a3aa1ad315a10cae24 (observed 2026-09-05T20:10:32Z). Final head 3f2f823 passed the complete Linux baseline with two-CPU affinity: 820 workspace tests, all 23 system fixtures including actual fixed-path policy isolation without a skip, formatting, strict Clippy, locked release, 12 eval definitions, and unique packaging/checksums. Both final CI runs passed: https://github.com/o-psi/voyage/actions/runs/33988666193 and https://github.com/o-psi/voyage/actions/runs/33988668420 . This delivers runtime enforcement of the administrator ceiling across ordinary, managed, workflow and child execution. Named profile lifecycle and interactive selection remain unfinished; #70 stays open. No live-provider or native macOS/Windows pass is claimed.

Onboarding #69 candidate7edd048 adds conservative bounded command evidence and omits commands from ambiguous quoted/commented/fenced Markdown contexts. Thirty targeted onboarding tests and strict Clippy pass; actual CLI and full baseline follow integration with current main. Independent review found no remaining context-guard blocker.

Remote lifecycle under #77/#9/#78/#79 is implementing an explicit foreground dedicated managed session, authenticated submit/watch/cancel/reconnect, transactional public event projection and durable command receipts. Scope: https://github.com/o-psi/voyage/issues/77#issuecomment-5554450723 . Tests-first implementation and independent authority review are underway; no execution delivery or passing baseline is claimed.


## PR 129 review and browser deferral

The operator explicitly requested review and merge of [PR #129](https://github.com/o-psi/voyage/pull/129) to main. Refreshed all 83 open/closed issue titles, bodies and states. The dedicated remote-session scope remains [#77](https://github.com/o-psi/voyage/issues/77#issuecomment-5554450723), with #9/#78/#79 retaining broader unfinished criteria. Review found the old-schema simulation must remove schema-7 tables and foreground shutdown must return failure when cleanup is unconfirmed. Both corrections require regression evidence and a fresh final Linux baseline/CI before merge.

The operator deferred all browser console work. The isolated vessel-session-console experiment is preserved unmerged; no JavaScript or Livewire console is included in PR129. Secret-workflow and policy-default work continue independently in isolated branches.


## Remote session delivery to main

PR #129 is merged into main as `fd5295ed48b21186fbd2f7845c93bb0eab7dc181` (observed 2026-09-05T21:36:54Z): https://github.com/o-psi/voyage/pull/129 . Fetched main exactly matches tested head `5593b1e` at tree `fcce1f9a9c48261de21a5322a90aacfe29ee82c8`.

Delivered: dedicated foreground remote session execution with authenticated list/inspect/submit/replay/cancel, durable exact-retry receipts, current local policy and connection authority, redacted transactional public events, and explicit local crash recovery/cleanup attestation. Review corrected the old-schema fixture and false-success shutdown exit; real failure regressions retain truthful Cancelled/unconfirmed state and block later admission.

Verified Linux: 894 workspace tests normally and with actual two-CPU affinity; all 26 system fixtures; formatting, strict Clippy, locked release, 12 eval definitions, unique packaging/checksum. All 34 local baseline stages passed. Both final CI runs passed: https://github.com/o-psi/voyage/actions/runs/33993043815 and https://github.com/o-psi/voyage/actions/runs/33993045729 . Definition validation is not live-provider evaluation; native macOS/Windows and paid/live calls were not run.

This is actual main delivery, not closure of the broader issues. Full session lifecycle/Create, private-session consent/import and retention, remote approvals, services, notifications and deployed production evidence remain open. Browser console work is explicitly deferred by the operator; none is included in this PR.

## Primary workspace reconciliation verification

Reconciliation merge `5e3dca6` preserves local delivery-history commits and the
operator's documentation/CI edits while incorporating published `fd5295e`.
Independent review confirmed runtime, dependency, CI and user-guide content match
that published tree; the delivery plan is the only difference.

Fresh Linux verification passed formatting, strict workspace Clippy, all 894
workspace tests (none ignored), the locked optimized workspace build, all 26
workflow system fixtures, and 12 evaluation definitions. Unique package
`voyage-reconcile-5e3dca6-20260905-linux-x86_64.tar.gz` passed its checksum.
Local evidence is under `.local-git/evidence/workspace-reconcile-20260905/`.
These are offline checks; no live-provider or native macOS/Windows result is claimed.
Other development worktrees remain intact, with unfinished scopes unmerged.
