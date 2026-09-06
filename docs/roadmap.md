# Voyage delivery map

Voyage is developing its first release. Helm is the program; every session is an
[open-ended voyage](voyages.md), including local chat. Planned distributed work
extends those voyages with explicitly scoped machines and coordination that can
run on another Helm. The complete agreed workflow and its verification define
delivery.

## Current foundations

Local Helm chat/tools, private managed sessions, local subagent supervision and
run-owned completion accounting are available. Enrollment and outbound presence
are available. The dedicated remote worker provides one new foreground managed
session through authenticated Vessel HTTP operations. These foundations do not
yet provide the unified Helm management interface or multi-Helm orchestration.

## Remaining connected work

| Area | Tracking | Outcome |
| --- | --- | --- |
| Multi-Helm voyage integration | [#77](https://github.com/o-psi/voyage/issues/77) | Scoped ongoing session across Helms; complete user workflow |
| Helm operator interface | [#14](https://github.com/o-psi/voyage/issues/14) | Local/remote viewing, targeting, scope and steering; browser console deferred |
| Runtime coordination | [#78](https://github.com/o-psi/voyage/issues/78) | Separate interface/coordinator/participant lifecycle and full managed sessions |
| Transport and identity | [#9](https://github.com/o-psi/voyage/issues/9), [#10](https://github.com/o-psi/voyage/issues/10) | Versioned routing, enrollment and current scoped authority |
| Privacy and decisions | [#79](https://github.com/o-psi/voyage/issues/79), [#21](https://github.com/o-psi/voyage/issues/21), [#72](https://github.com/o-psi/voyage/issues/72) | Explicit context sharing, scoped approvals and Vessel notifications |
| Operations | [#18](https://github.com/o-psi/voyage/issues/18) | Explicit service lifecycle, setup and recovery evidence |
| Readiness | [#82](https://github.com/o-psi/voyage/issues/82), [#83](https://github.com/o-psi/voyage/issues/83), [#22](https://github.com/o-psi/voyage/issues/22), [#19](https://github.com/o-psi/voyage/issues/19) | Completion correctness, representative workloads and measured dogfood |

Other capabilities retain their own open issue acceptance criteria. Consult the
complete [GitHub issue inventory](https://github.com/o-psi/voyage/issues?q=is%3Aissue)
and [delivery history](remaining-issues-plan.md); historical branch names and closed
foundation issues are not current assignments or proof of readiness.

Shared protocol changes require both Helm and Vessel compatibility tests. Scope,
context and handoff contracts precede their dependent UI/runtime implementation.
Security, recovery and verification belong to every delivery, not a final cleanup
phase. Use focused branches and GitHub PRs; keep incomplete required work explicit.

Linux quality, real component fixtures, packaging and recorded behavioral/deployment
checks remain required as applicable. Other-platform results must be observed
separately. [Cutover](cutover.md) requires representative workload and dogfood
evidence; individual merges and successful builds do not close that gate.
