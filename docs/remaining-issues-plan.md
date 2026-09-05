# Remaining GitHub delivery plan

User mandate: comprehensively resolve remaining issues, including epics. Inventory refreshed from all GitHub pages: 82 issues; initial working tree clean.

## Active work

- #64: context accounting and request preflight implemented; issue scope posted; 361 workspace tests passing, release/system verification underway. Acceptance tests: full request accounting, initial/tool follow-up guard, indivisible rejection, whole-turn reduction, canonical history preservation, cancellation, provider-neutral dispatch.
- #86: audit found input bounds, delivery-state and chronology gaps; fixes and PTY regressions underway in isolated worktree.
- #80 and #59: independent implementation/evidence audit underway.

## Dependencies and delivery order

1. Verify existing local features (#86, #80, #59), implement P0 request safety (#64).
2. Run ownership/readiness (#81), reconciliation (#82), end-to-end verification (#83), resilient execution (#5).
3. Outbound protocol (#9), enrollment (#10), sharing (#79), coordinated sessions (#78), UI (#14), approvals (#21/#72); then attachment epic (#77).
4. Isolation (#62), per-agent limits (#37), policy profiles (#70), budgets (#71).
5. Extensions (#63/#75), MCP lifecycle (#65), local providers (#66), workflows (#67/#76), GitHub (#68), onboarding (#69), images (#74), voice (#73).
6. Packaging (#18), measured dogfood (#22), replacement epic (#19).

Each feature needs issue discussion review/update before edits, acceptance-driven tests, baseline Linux gates and packaging, and honest platform/provider evidence. Epics remain open until all exit criteria are met. No issue is considered delivered by this plan.

## Open inventory

- [#86: Allow steering messages during an active Helm run](https://github.com/o-psi/voyage/issues/86) — pending
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
- [#64: Enforce context-window limits and compact automatically before every provider request](https://github.com/o-psi/voyage/issues/64) — pending
- [#63: Add external extension and skill packaging, installation, and discovery](https://github.com/o-psi/voyage/issues/63) — pending
- [#62: Add OS-level sandbox adapters for Linux, macOS, and Windows](https://github.com/o-psi/voyage/issues/62) — pending
- [#59: TUI: Keep conversation user-facing and isolate activity/log diagnostics](https://github.com/o-psi/voyage/issues/59) — pending
- [#37: Enforce per-agent tools, approvals, budgets, and resource limits](https://github.com/o-psi/voyage/issues/37) — pending
- [#22: Run structured dogfood and incumbent-harness cutover](https://github.com/o-psi/voyage/issues/22) — pending
- [#21: Define attended and unattended approval semantics](https://github.com/o-psi/voyage/issues/21) — pending
- [#19: EPIC: Make Helm ready to replace the incumbent harness](https://github.com/o-psi/voyage/issues/19) — pending
- [#18: Package and operate Voyage across supported platforms](https://github.com/o-psi/voyage/issues/18) — pending
- [#14: Build Vessel fleet and operations UI](https://github.com/o-psi/voyage/issues/14) — pending
- [#10: Implement Helm identity, pairing, and worker authorization](https://github.com/o-psi/voyage/issues/10) — pending
- [#9: Version and stream the outbound Helm task protocol](https://github.com/o-psi/voyage/issues/9) — pending
- [#5: Build resilient agent execution semantics](https://github.com/o-psi/voyage/issues/5) — pending

## Verification and blockers

Context preflight: 361 Linux workspace tests passed; strict Clippy passed before the final config regression/cancellation-priority additions. Baseline regression reproduced with old release. Release build/system tests pending. GitHub reads require sandbox escalation; installed gh binary is used because the mise shim attempts an installation. Live-provider and cross-platform evidence must be separately obtained; definition validation is not live evaluation.
