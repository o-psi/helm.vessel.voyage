# BEB reachability review for #353

This read-only audit uses clean measured source
`beb0e01be386ae896af74c2ea97f423b24974b13`, its corrected detailed JSON and LCOV.
The [durable checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5950354073)
records the findings. Nineteen priority files have **145 sampled function
witnesses**, exact mapped spans/zero-line ranges, callers, authority guards and
source hashes. This is **not a complete call graph, per-line reachability proof,
reachable count or promised coverage gain**.

The full measurement remains 106,617/134,951 lines; 28,334 aggregate gap units
and 26,187 distinct zero addresses are different accounting units. Neither is
automatically a count of reachable unasserted production lines. All existing
scope, exclusions and source remain measured. The **100% reachable-production
objective remains open**. Deferred or untested code is not called unreachable.

The ignored manifest is
`target/verification-v103/beb-reachability-audit/beb-top-reachability-compact.json`,
SHA256 `820da870bd4507baaeb4f222ed7a6fc156601c1e10a002f264e27bfdee657f47`.
Every selected source file matched its measured bytes, and JSON file-segment
counters matched retained LCOV. The computation used no LLVM export, Cargo,
profile merge or native effect. Refresh after the next measured delivery;
new source cohorts are not attributed to this old measurement.

## Priority source and authority map

Counts are **aggregate gap units / distinct zero addresses** at BEB. Each row
samples representative guards; it does not classify every address in the file.
The [release plan](../releases-v1.0.3.md) explicitly parks positive Root/system,
execution identities, adoption, native macOS/Windows and Tax-Axis work for1.1.
Their code stays in the denominator. No positive1.1 implementation is resumed.

| File; counts | Representative caller, function and exact guard | Classification and remaining ordinary surface |
| --- | --- | --- |
| `installer/src/system_install/adoption.rs`; 1623 /1597 | `adopt-user` → `run`1515 → directory/root64 → [root_host358](../../installer/src/system_install.rs#L358): real/effectiveUID0, SETUID/SETGID capabilities and full identity maps. [peer250](../../installer/src/system_install/adoption.rs#L250) owns the pipe and explicitly drops the user helper's identity. | Positive adoption is deferred1.1. Arg/UUID/root refusals and bounded format/hash helpers are separate ordinary candidates. No adoption success is inferred from parsing. |
| `vessel/src/process/migration.rs`; 1064 /1025 | Hidden vessel MigrationUser/Control → [user458](../../vessel/src/process/migration.rs#L458) or [control830](../../vessel/src/process/migration.rs#L830). [pipe60](../../vessel/src/process/migration.rs#L60) requires SO_PEERCRED UID0. | User helper runs under its original ordinary real/effectiveUID **and a root peer**; control requires real/effectiveUID0/full maps. Root-orchestrated positive migration is deferred. Ordinary refusal/frame/private-file candidates remain. |
| `vessel/src/process/execution_transition.rs`; 819 /795 | Connected Execution → [prepare335](../../vessel/src/process/execution_transition.rs#L335): explicit stop consent, protected current owner authority, bound source; [directory129](../../vessel/src/process/execution_transition.rs#L129) retains root-private receipts. | Positive identity transition is deferred. Consent, shape/digest/provenance and ordinary service refusal are separate candidates. Do not weaken review/receipt/target fences. |
| `vessel/src/process/admin_execution.rs`; 528 /518 | Connected Execution → [execution_operation404](../../vessel/src/process/admin_execution.rs#L404); [provision117](../../vessel/src/process/admin_execution.rs#L117) loads protected Administrator/UID0 configuration and separate namespaces. | Administrator-positive deployment is deferred. Bounded kernel-status parser, provision shape/hash and ordinary authority refusals can be reviewed independently. |
| `vessel/src/process/bound_lifecycle.rs`; 493 /470 | Bound start → [start_bound_initialized164](../../vessel/src/process/bound_lifecycle.rs#L164): euid0 before reservation, exact binding/layout; guardian owns execution separately. | Bound-positive lifecycle is deferred. Ordinary refusal and typed metadata/fault validation remain; normal ordinary Voyage lifetime is a different path. |
| `installer/src/system_install.rs`; 487 /478 | `--scope system` → [run776](../../installer/src/system_install.rs#L776); [root_host358](../../installer/src/system_install.rs#L358) enforces root/capabilities/full maps. | System-positive install/lifecycle is deferred; CLI/refusal/read-only facts are not deployment success. |
| `installer/src/remote/system.rs`; 467 /460 | Explicit remote `system` branch → [root59](../../installer/src/remote/system.rs#L59): real/effectiveUID0, remote_host, existing private system installation. | Full-file ordinary classification is unsupported. Pure UUID/record/protocol/refusal arms are distinct from deferred positive system update. |
| `installer/src/system_install/lifecycle.rs`; 453 /446 | System upgrade/rollback/uninstall → [run_reviewed388](../../installer/src/system_install/lifecycle.rs#L388); [remote_review845](../../installer/src/system_install/lifecycle.rs#L845) also calls root_host. | Positive system lifecycle remains deferred; parsing, pinned facts and refusal are not automatically privileged effects. |
| `vessel/src/process/identity_accounts.rs`; 448 /432 | [projection12](../../vessel/src/process/identity_accounts.rs#L12) checks AccountUse/Enroll/current scope; [selected47](../../vessel/src/process/identity_accounts.rs#L47) uses protected default or bound observer identity. [accounts304](../../vessel/src/process/accounts.rs#L304) chooses identity versus ordinary registry by layout. | Identity-scoped positive orchestration is deferred. Projection itself is not UID0-only; ordinary account operations are separately supported. Credential/provider-success arms need actual authority/budget/evidence. |
| `installer/src/remote/linux.rs`; 424 /416 | Default [remote dispatcher8](../../installer/src/remote.rs#L8) selects user updater; normal current bootstrap, worker and reconcile use owner-reviewed managed user installation/receipts/leases. | Supported ordinary Linux flow, with native manager, public acquisition and fault arms. Reuse delivered #401/forward cases before selecting residual worker/staging/uncertainty assertions. |
| `vessel/src/process/guardian.rs`; 351 /342 | Hidden GuardBound → [run306](../../vessel/src/process/guardian.rs#L306): euid0 before protected root/exact incarnation. [request_stop58](../../vessel/src/process/guardian.rs#L58) is only a durable request. | Deferred privileged guardian; ordinary refusal/shape/receipt candidates remain. Stop requested is never observed cleanup. |
| `voyage/src/transition_helper.rs`; 302 /287 | Hidden TransitionHelper; [execution_transition helper211](../../vessel/src/process/execution_transition.rs#L211) drops to configured identity. [identity40](../../voyage/src/transition_helper.rs#L40) requires unmixed UID/GID/full maps; [run274](../../voyage/src/transition_helper.rs#L274) uses FIFO stdio; [hold147](../../voyage/src/transition_helper.rs#L147) owns private journal/leases. | Ordinary-UID helper inside deferred orchestration. **A root-peer guard is not proved**: fd type is checked, unlike migration SO_PEERCRED. Direct owned-helper primitives/faults are not permission to implement positive1.1 transitions. |
| `vessel/src/process/database_execution_reviews.rs`; 291 /260 | Review/transition APIs → [protected19](../../vessel/src/process/database_execution_reviews.rs#L19), then current full enrolled owner/private provenance. | Explicit real/effectiveUID0 contradicts a default ordinary filename label. Deferred protected transactions; direct refusal/shape candidates remain. |
| `vessel/src/process/identity_start.rs`; 286 /274 | Bound-layout Start → [start_identity_request99](../../vessel/src/process/identity_start.rs#L99) → protected planned directory209 → bound admission. [pin_launch18](../../vessel/src/process/identity_start.rs#L18) distinguishes ordinary registrations. | Positive bound start is deferred; ordinary no-peer bypass/refusal and DTO/hash cases remain. Layout presence alone is not authority. |
| `installer/src/system_preflight.rs`; 285 /274 | [installer main29](../../installer/src/main.rs#L29) routes `system-assess` before system install; [run41](../../installer/src/system_preflight.rs#L41) emits read-only bounded assessment. | Ordinary read-only diagnostics are currently reachable, despite deferred positive activation. Do not classify all preflight lines privileged/unreachable or enable a service to obtain coverage. |
| `vessel/src/process/bound_recovery.rs`; 225 /213 | [recover_bound113](../../vessel/src/process/bound_recovery.rs#L113) → [recovery_authority55](../../vessel/src/process/bound_recovery.rs#L55): current owner History/Lifecycle, protected cleanup identity; admin changes require protected authority. | Deferred bound recovery; receipt parsing/scoped-attestation refusal remains separate. [validate_receipt17](../../vessel/src/process/bound_recovery.rs#L17) forbids execution/restart authorization claims. |
| `helm/src/process_client/admin/transfer.rs`; 189 /181 | Owner Move → [run26](../../helm/src/process_client/admin/transfer.rs#L26): no scoped access_file, absolute private journal/destination/workspace. [ordinary service930](../../vessel/src/process/service.rs#L930) routes signed transfer and advertises the capability. | Supported ordinary owner courier. Administrative filename does not mean root execution identity. Signed peer/offset/reopen/receipt/fault cases need no real host transfer or new grant. |
| `voyage/src/extensions/runtime.rs`; 164 /149 | [build/tools45](../../voyage/src/build/tools.rs#L45) → [register734](../../voyage/src/extensions/runtime.rs#L734); package provenance/current policy/private files and [owned shutdown204](../../voyage/src/extensions/runtime.rs#L204). | Supported executing-Voyage integration; registration never launches code. Actual owned broker/read/drain/refusal fixtures are distinct from mocked launch or native cleanup. |
| `voyage/src/server/authorization.rs`; 138 /126 | [authorize_parts105](../../voyage/src/server/authorization.rs#L105): UserFile ordinary registration versus protected Broker peer; [read_current259](../../voyage/src/server/authorization.rs#L259) checks exact current grant/connection/parent/account/workspace. | Supported ordinary UserFile authority mixed with deferred bound Broker-positive scope. Do not apply either classification to the entire file. |

## Corrections to unsupported classification premises

- Filename prefixes are not entry/authority proof. `remote/system` and
  `database_execution_reviews` cannot be called wholly ordinary; `admin/transfer`
  cannot be called wholly Root. `execution_identity` advertisement selected by
  layout presence is not itself a grant or proof of protected authority.
- Original-user migration and transition helper are different boundaries:
  migration has a root kernel peer; transition-helper has its own UID/namespace/
  pipe/private-journal checks. Do not invent a shared root-peer predicate.
- Ordinary system assessment is read-only. It does not complete deferred system
  deployment. Conversely, deferred deployment does not make its diagnostics
  unreachable.
- External acquisition/provider success, owned fault fixtures and native manager/
  platform evidence remain separate. No absent credential, budget or authority is
  replaced with a fabricated success. Linux coverage establishes no native
  macOS/Windows behavior or Tax-Axis adoption.

## Substantive next ordinary selection

The **181** signed courier, **149** executing-extension runtime, **126** mixed
UserFile-authority, **274** read-only preflight and **416** user-updater/fault
addresses are input counts, **not gains or proven ordinary reachable totals**.
Select complete behaviors: frozen source/destination and exact signed receipts;
sealed package/current-policy and observed owned child/read drain; complete grant
revocation/attenuation/queued dispatch; bounded diagnostic uncertainty without
effects; exact worker/manager/staging failure and retained no-replay obligations.
Reuse existing tests and native #401/account/lifecycle evidence, inspect actual
missing assertions and coordinate file ownership before implementing. Do not
repeat completed families merely to increase a percentage.

The newer observation and saved-owner/steering cohorts were independently
source-reviewed after producer inner-Snapshot SID/cursor and full SQLite
side-table oracle corrections. They remain **source-only/unrun** at this audit;
no results are attributed to BEB. Their combined focused/strict/full current-object
measurement/publication/build gate was pending at that historical audit; the
subsequent verified result is recorded below. This planning note
implements no runtime, resumes no positive1.1 work and removes no hard lines.

## Subsequent coordinated execution checkpoint

The source-only status above describes this historical BEB reachability audit,
not the later measurement. Clean `1d49636e47bd42a7aef3dc4c54741209f5790335`
subsequently passed formatting, strict Clippy and full workspace coverage:
2,860 passed, zero failed and eight ignored across fifteen parent test targets.
All 56 new observation, saved-owner/steering and public-gateway parents passed.
The exact 18-current-object audit, source totals and accounting are recorded in
[the observation result](coverage353-helm-observation-prep.md#verified-measurement-and-remaining-delivery).
Normal Main `c3f997f8ac1d1d1c821cb7a6a5b05f84f2ca795e` was subsequently published and
[run 36999095928](https://github.com/o-psi/helm.vessel.voyage/actions/runs/36999095928)
terminated SUCCESS. Exact source/BUILD identity, public checksum/digest and
safe four-binary/123-browser-asset archive verification passed; archive SHA-256
`e38f693c44e1f04984af7e71207e26c24e994bddd20aa7f347e3943a537b3626`.
[Full artifact record](coverage353-helm-observation-prep.md#published-source-and-verified-hosted-artifact),
[#353 checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951154544)
and [#375 checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/375#issuecomment-5951154114)
preserve the measured-versus-built source distinction and no-native-installation
limit. #333, #353 and #375 remain open.
No newer result is attributed to BEB; the 145 sampled spans and nineteen files
remain a partial planning sample, not a complete reachable-production graph.
All source, denominator, exclusions, native authority boundaries and the full
100% objective remain unchanged.
