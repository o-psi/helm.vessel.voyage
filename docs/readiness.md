# First-release readiness work

[#19](https://github.com/o-psi/helm.vessel.voyage/issues/19) tracks executable
product readiness, not a mandatory human adoption ceremony. The current work
instruction requires no human testing, visual sign-off, trial schedule or operator
cutover acceptance yet. Historical proposals for those activities are not blockers
for implementing and verifying the product. This does not turn unrun checks into
passes, authorize provider spending or deployment changes, or remove real gaps.

## Component and evidence boundaries

Follow the [architecture](architecture.md): Helm presents, Vessel supervises and
routes, and each independent Voyage owns its session's execution and canonical
history. Disconnecting Helm does not cancel accepted work. Human connections,
participant assignments, independent model-created voyages and owner transfer are
different authority and ownership surfaces. No retired outbound worker or Codex
bridge is required. The opt-in [local browser](local-browser.md) uses the existing
full-duplex Helm–Vessel socket; it is not a browser/mobile replacement interface.

[Current state](current-state.md) describes implemented paths and limitations.
[Quality](quality.md) describes focused checks, not universal coverage. Source
inspection establishes what a path does; an actual check establishes only its
recorded behavior, revision and environment. Completion accounting records task
outcomes, not semantic correctness. Native platform and deployed TLS claims require
actual evidence for those environments, not human sign-off or inference from Linux.

## Executable dependency map

This is a source/issue assessment at `8a2c6e1`, not new runtime verification.
Statuses belong to the linked issues and must be refreshed as deliveries arrive.
A closed historical foundation with obsolete wording is not authority to restore
an old design. An open issue is not itself proof that every listed path is absent.

| Owner | Work that still needs implementation or executable evidence | Existing foundation / source |
| --- | --- | --- |
| #10 | Credential protection/key custody and content-free current-grant lifecycle audit | [Connections](vessel-connections.md); inventory/revoke/re-pair is not retired enrollment recovery. |
| #14 / #32 | Usable operator journeys and private-terminal application/input/rendering/recovery fidelity | [Interaction inventory](ux-readiness.md); its historical human review gates do not apply to this work instruction. |
| #18 | Supported installation, update, data preservation, service and recovery behavior | [Installer](../installer/README.md); package checks do not prove service operation. |
| #21 | Exact-request remote decision/effect recovery, expiry, revocation and bounded unattended behavior | [Runtime contract](runtime-contract.md); existing durable decisions are not an absent approval engine. |
| #64 | Durable reduced working context separate from full history; active-turn reduction and bounded recovery from actual provider context rejection without tool replay | [Request preflight](../voyage/src/context.rs) removes older request prefixes; [saved compaction](../voyage/src/session.rs) removes messages and inserts an omission marker. Neither completes this contract. |
| #63 / #75 | Executable packaging/activation and versioned SDK, invocation, authority and cleanup | [Extensions](../voyage/src/extensions.rs) explicitly restrict capabilities to `model_context`; MCP and declarative packages are foundations, not an executable SDK. |
| #70 | Named-profile tool/secret/network/sandbox categories and connected selection/provenance | [Profile schema](../voyage/src/policy_profile.rs); the Access chooser is not full profile selection. |
| #71 | Actual token/monetary/resource budgets, pricing provenance and current dashboards | [Inference accounting](../voyage/src/inference); dispatch permits are not billed tokens or monetary caps. Unknown usage is not zero. |
| #77 / #79 / #72 | Integrated multi-Vessel adverse workflows, positive disclosure/retention/cache modes and authenticated notification handoff | [Process access](process-access.md); pairing, sharing, approving, assigning and transferring remain distinct. |
| #198 / #200 | Deployment-specific TLS/proxy/upstream/admin-route checks and native credential-storage protection | [Security](security.md); the fixed Linux cache issue is not proof of native macOS/Windows protection. |
| #22 | Representative useful-work correctness and failure/recovery evidence against independently checkable artifacts | [Focused workflow checks](quality.md); synthetic providers exercise mechanisms, not general model quality. Human dogfood/cutover is not required by the current instruction. |

Issues are in [o-psi/helm.vessel.voyage](https://github.com/o-psi/helm.vessel.voyage/issues).
The simultaneous #10/#14/#18/#21 owners supply their own implementation and
verification; epic maintenance must not duplicate them. #78's independent-owner,
#180's automatic-outcome, #7's MCP and #62's optional Linux isolation foundations
must be reused with their documented limits, not reopened merely because old
acceptance text predates the current architecture.

## Closure and failure handling

1. Map every remaining requirement to its delivery issue, implemented path and
   actual result. Record source/artifact identity, command or reproducible procedure,
   platform, expected and observed outcome, and evidence location. Distinguish
   implemented-but-unverified behavior from a missing implementation.
2. Verify meaningful failure paths: refusal before effects, stale authority, lost
   admission response, cancellation, interrupted persistence, unavailable owner,
   restart and observed versus unknown cleanup. Never replay uncertain effects to
   manufacture evidence. Inspect results from independent owners before claiming
   integration or closure.
3. Review useful artifacts independently of task completion flags: file contents,
   calculations, attribution, transformations and truthful partial outcomes. Record
   failed attempts and corrected results separately. Provider/account failures are
   scoped observations, not permanent universal credential failures.
4. Publish verified changes and retain real unresolved dependencies. No operator
   trial/sign-off is needed to resolve implementation scope, but neither a document
   refresh nor a successful build closes the epic while product gaps remain.
5. If an environment or authorization is unavailable, record the exact blocked
   check and supported-platform limit. Do not replace it with a human acceptance
   request, claim it passed, or silently waive the underlying requirement.

This guide adds no executable checks, changes no runtime settings and establishes
no release, cutover or provider-quality certification. Historical failures and
unrun trials in issue discussion remain history, not new prerequisites. It is a
repository planning guide linked by #19, not a change to the archive document
manifest or instructions to restore locally deleted packaging/test scripts.
