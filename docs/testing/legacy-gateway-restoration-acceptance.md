# Legacy gateway restoration and original #401 acceptance

Source-only follow-up to the
[recorded source defect](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5948674655).
Four new Rust parents (seven service-boundary scenarios) are **uncompiled and
unrun**. Rustfmt/diff/source checks do not establish execution. This is a
required current-release repair, not a new native fault or broad feature.

## Narrow production correction

Original [#401](https://github.com/o-psi/helm.vessel.voyage/issues/401) requires
supervisor restoration and every unchanged reviewed gateway independently;
pointer rollback alone is insufficient. Ordinary `restore_previous` already
retains their separate outcomes, but the later `apply_legacy` rollback branch
used `restore_activation(...)?` before `rollback_gateways(...)?`. A supervisor
configuration/readiness error skipped all gateways, repeating that defect.

The new private `restore_legacy_services` boundary retains supervisor restore,
gateway restore and actual previous executable observation outcomes, attempts
them independently, and aggregates **all** errors into an unconfirmed result.
`rollback_gateways` still refuses each independently changed definition and
continues other reviewed gateways; only unchanged reviewed units receive
start-limit reset/restart. No success or quarantine removal follows an error.

All preceding gates stay in their original order: matched old/candidate pointer,
candidate quiescence, held exact snapshot restoration and guard-backed old
pointer when publication happened, ownership release only after restoration,
and original activation-context proof. A prepublication refusal needs no
invented snapshot. Original namespace/readiness/context checks remain before
quarantine clear. No definition, namespace, enablement, credential override,
manifest, receipt, schema, ownership or no-replay protection is weakened.

## Prepared regression boundary

The existing thread-local manager/PID seam prevents real systemctl or host PID
access. Both exact unit definitions come from the production public service
preview. Tests use real private unit/receipt/quarantine/pointer files, exact
scripted manager calls and previous/candidate PID executable links. A successful
readiness arm runs only an owned shell fixture emitting `[]` for the normal Helm
catalogue read. It is not an authenticated native endpoint qualification.

Cases require:

- Original supervisor start refusal still consumes both unchanged gateways'
  exact reset/restart/readiness/PID sequence and the separate supervisor image
  observation, without losing that supervisor error.
- First gateway independently changed, restart-refused, inactive or executing
  the candidate: the second is still attempted; every error is retained. The
  operator-changed private unit bytes remain unchanged and no first-gateway
  activation is queued in the changed-definition arm.
- A candidate supervisor image cannot be hidden by the old pointer and ready
  gateways. Exact original unconfirmed receipt, quarantine and transaction bytes
  stay untouched.
- Even all service observations succeeding at this boundary does not itself
  promote an uncertain receipt or clear quarantine. The existing caller still
  has to prove original namespace and context.

These tests exercise the real **post-restoration service boundary**, not the
upstream SQLite/held-guard restoration itself. Existing held-pointer/legacy
regressions and native startup qualification remain independent. Do not claim a
new full transaction or native multi-gateway rollback from this fixture.

Pending coordinated gate: `remote::linux::tests::legacy_services`, the existing
`rollback_gateway_matrix` and legacy service/held-pointer families, then strict
workspace checks and fresh full workspace coverage/current-object audit after
all combined source edits. Commit the compact summary, publish normally and own
the actual hosted artifact before declaring this repair delivered. Source and
private failed/native records remain independently resumable.

## Independent original-acceptance audit

Audit inputs are the original issue body, maintained
[ordinary legacy runbook](ordinary-legacy-update-ct119.md),
[remote update contract](../remote-updates.md), published source and the durable
native records. It adds no host action or new test requirement.

| Obligation | Evidence and honest scope |
| --- | --- |
| Gateway origin flag/canonical HTTPS/literal-loopback validation | Delivered parser regressions and actual CT106 canonical public-origin gateway/strict TLS health. This remains an established flag, not a removed pinned argument. |
| Real CT106 recovery, original uncertainty not replayed | [CT106 record](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5945526723): new exact reviewed forward operation complete, original `2a84…` untouched, namespace/state/account/history/credential override preserved. This is forward recovery, not reconstruction of an absent original snapshot. |
| Active legacy owner refusal | [CT125](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5945860692): one held synthetic response, exact owner/run untouched, no cancel/snapshot/publication/quarantine, observed completion/cleanup. |
| Forced startup failure and actual previous reader | [CT128](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5947915793): exact old service/pointer/schema1, no quarantine, complete raw and public-v1 histories, separate per-read helper PID/start/image/pidfd exit+reaping. Failed CT124 remains failed/unconfirmed, not overwritten by this later case. No native multi-gateway rollback is claimed. |
| Helper death at pre-proof boundary | Same record's CT127: exact owned helper/updater retired; old service/schema1/full histories and per-read cleanup qualified. Original proof is absent at that boundary; requiring a fabricated proof/restore marker would be incorrect. Original uncertain receipt remains. |
| Local updater interruption/missing restore proof | [CT126](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5946815896): supported status strictly after unchanged deadline gives bounded unconfirmed outcome, old schema/pointer/quarantine retained, stopped service, no replay; original target retirement and history/lease observations are separate. This is not successful rollback or independent remote UpdateApply worker death. |
| Independent namespace/unit/enablement change | [CT129–131](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5948285504): real exact pause/change/continue, original operator changes preserved, unconfirmed receipts, retained proof/backup/quarantine and exact raw histories; no successful reconciliation/rollback claimed. Installed four executables/123 assets verified. |
| Original independent gateway restoration | **Current source gap above**. New exact regression plus source-final strict/full local coverage and actual hosted build are required before closure. Existing local gateway matrix is not a native gateway fault; CT106 forward recovery cannot be relabelled rollback. |

The changed-context negative cases are not required to become successful
rollbacks. Neither original #401 nor the runbook requires converting original
unconfirmed receipts to success, claiming a candidate never ran, installing
every later unrelated UI artifact, or collecting every historical helper ID in
all context cases. The specifically signalled fault targets/per-read helpers
have their own required retirement evidence. Aggregate account-tree equality
and ten momentary acquired/released leases must keep their actual scope; do not
invent per-file admission checks or a continuously held restore guard.

CT129's persistent `activating`/MainPID0, unavailable live image/namespace and
unknown historical helper IDs remain **unresolved observations**, not verified
readiness, never-ran or whole-cleanup facts. Preserve this owned-fixture
disposition in the [release checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/375#issuecomment-5948553818).
Read-only existing manager journal/Job/unit/environment and retained helper
records are the next safe evidence source. Do not repeat status/apply/start,
force context restoration, signal a guessed PID or manufacture historical
witnesses. This audit demonstrates no additional product defect from PID0 alone.

Full browser #333, 100% reachable coverage #353, Root/system/native macOS/Windows
scope and stable publication remain distinct; this audit closes none of them.
