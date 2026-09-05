# Remaining GitHub delivery plan

User mandate: comprehensively resolve remaining issues, including epics. Inventory refreshed from all GitHub pages: 82 issues; initial working tree clean.

## Active work

- Delivered #64 context budgets, #86 steering, and #59 readable tool output follow-up in [PR #87](https://github.com/o-psi/voyage/pull/87), merged as 05310bf. All three issues are closed.
- [#81](https://github.com/o-psi/voyage/issues/81): run ownership, coordinated stores and frontend references implemented; integration verification continues with #82.
- [#82](https://github.com/o-psi/voyage/issues/82): bounded reconciliation, durable seals and CLI checkpoint acknowledgements implemented in an isolated checkout. TUI save acknowledgements and observed descendant cleanup are being completed.
- [#83](https://github.com/o-psi/voyage/issues/83): three native-provider fixtures and adversarial evaluation scenarios under verification. Live-provider evidence remains blocked by the previously observed configured-provider HTTP401; definition validation is not live execution.
- Recovered committed work after temporary-directory loss. Active checkouts and evidence now live under `.local-git/` on durable disk. Missing uncommitted callbacks/fixtures are being reconstructed and rechecked.

## Dependencies and delivery order

1. Verify existing local features (#86, #80, #59), implement P0 request safety (#64).
2. Run ownership/readiness (#81), reconciliation (#82), end-to-end verification (#83), resilient execution (#5).
3. Outbound protocol (#9), enrollment (#10), sharing (#79), coordinated sessions (#78), UI (#14), approvals (#21/#72); then attachment epic (#77).
4. Isolation (#62), per-agent limits (#37), policy profiles (#70), budgets (#71).
5. Extensions (#63/#75), MCP lifecycle (#65), local providers (#66), workflows (#67/#76), GitHub (#68), onboarding (#69), images (#74), voice (#73).
6. Packaging (#18), measured dogfood (#22), replacement epic (#19).

Each feature needs issue discussion review/update before edits, acceptance-driven tests, baseline Linux gates and packaging, and honest platform/provider evidence. Epics remain open until all exit criteria are met. No issue is considered delivered by this plan.

## Open inventory

- [#86: Allow steering messages during an active Helm run](https://github.com/o-psi/voyage/issues/86) — delivered in #87
- [#83: Validate and document completion-gate behavior end to end](https://github.com/o-psi/voyage/issues/83) — pending
- [#82: Gate final responses with bounded work reconciliation](https://github.com/o-psi/voyage/issues/82) — pending
- [#81: Track run-scoped completion readiness for subagents and todos](https://github.com/o-psi/voyage/issues/81) — pending
- [#80: Add native multiple-choice questions tool with custom answers](https://github.com/o-psi/voyage/issues/80) — pending
- [#79: Enforce session-sharing consent, retention and remote authorization boundaries](https://github.com/o-psi/voyage/issues/79) — pending
- [#78: Coordinate local and Vessel-managed sessions in the Helm runtime](https://github.com/o-psi/voyage/issues/78) — pending
- [#77: EPIC: Manage Helm sessions from Vessel through outbound attachment](https://github.com/o-psi/voyage/issues/77) — pending
- [#76: Add team-shared prompts, policies, and workflows](https://github.com/o-psi/voyage/issues/76) — pending
- [#75: Publish a plugin SDK and stable extension API](https://github.com/o-psi/voyage/issues/75) — pending
- [#74: Support image and screenshot attachments in multimodal turns](https://github.com/o-psi/voyage/issues/74) — pending
- [#73: Add optional voice input to Helm](https://github.com/o-psi/voyage/issues/73) — pending
- [#72: Add remote/mobile notifications and secure approval handoff](https://github.com/o-psi/voyage/issues/72) — pending
- [#71: Add resource telemetry, cost dashboards, and enforceable budgets](https://github.com/o-psi/voyage/issues/71) — pending
- [#70: Add reusable policy profiles and trust presets](https://github.com/o-psi/voyage/issues/70) — pending
- [#69: Add repo-aware onboarding and generated project guidance](https://github.com/o-psi/voyage/issues/69) — pending
- [#68: Add GitHub-native workflows for issues, pull requests, checks, and review](https://github.com/o-psi/voyage/issues/68) — pending
- [#67: Add reusable saved workflows and parameterized commands](https://github.com/o-psi/voyage/issues/67) — pending
- [#66: Add local-model and OpenAI-compatible provider presets with endpoint discovery](https://github.com/o-psi/voyage/issues/66) — pending
- [#65: Add first-class MCP marketplace, installation, and lifecycle UX](https://github.com/o-psi/voyage/issues/65) — pending
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

## Verification and blockers

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
