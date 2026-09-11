# Documentation

The architecture documents define the intended product. Current-operation guides
only describe the binaries in the source tree. Commands for unimplemented features
are not presented as usable commands.

| Read this | To understand |
| --- | --- |
| [Architecture](architecture.md) | Helm, Vessel and the one-session-per-voyage-process boundary |
| [Runtime contract](runtime-contract.md) | Ownership, persistence, decisions, reconnect and shutdown |
| [Implementation](implementation.md) | Work needed to deliver the target, in dependency order |
| [Current state](current-state.md) | What the existing code implements and what remains unfinished |
| [Configuration](configuration.md) | Current provider, policy and storage configuration |
| [Named provider accounts](provider-accounts.md) | Helm account selection/private device sign-in, host-only credentials, migration and authority |
| [Provider accounts plan](provider-accounts-plan.md) | Design decisions and acceptance matrix for named native accounts |
| [Operations](operations.md) | Connected HTTP(S) voyages and legacy chat/managed/worker procedures |
| [Vessel connections](vessel-connections.md) | In-app connection manager, remembered Vessels, pairing, workspaces and access recovery |
| [Connection verification](vessel-connections-verification.md) | Observed Linux UI, protocol, recovery, transport and deployment checks |
| [Vessel coordination](vessel-coordination.md) | Native model inspection, steering, independent voyage creation and follow-up |
| [Local shared browser](local-browser.md) | Local companion, full-duplex routing, privacy, file disclosure and recovery |
| [Browser verification](local-browser-verification.md) | Actual synthetic TUI/runtime/browser journeys and remaining qualification limits |
| [Visual tool results](visual-tool-results.md) | Provider image encodings, tool provenance and bounded artifact projection |
| [Duplex transport](duplex-transport.md) | One authenticated socket for Helm commands, events and local capabilities |
| [Images and screenshots](multimodal-implementation.md) | Attaching images, private storage, provider support and limits |
| [Composer previews](ui-previews.md) | Opt-in inline thumbnails, terminal fallbacks and decoding bounds |
| [UI components](ui-components.md) | Semantic styles, scoped forms, Unicode editing and library decisions |
| [Helm UI framework](helm-ui-framework.md) | Deep architecture audit and Helm-specific replacement design; not implemented |
| [Process access](process-access.md) | Scoped grants, participant execution, signed owner transfer and recovery |
| [Security](security.md) | Authority, credentials, disclosure, terminal safety and first-release audit |
| [Development](development.md) | Source layout, build commands and contribution workflow |
| [UX readiness](ux-readiness.md) | Interaction inventory, known gaps and manual product acceptance gate |
| [Design lessons](design-lessons.md) | Consolidated design principles from 13 terminal apps, source evidence and Helm design guidance |
| [Quality](quality.md) | The remaining non-test checks and limits of their evidence |
| [Releasing](releasing.md) | Archive contents, packaging and publication boundaries |
| [Local Git](local-git.md) | GitHub and this workspace's Git wrapper |

The component entrypoints are [Helm](../helm/README.md) and
[Vessel](../vessel/README.md), [voyage runtime](../voyage/README.md) and
[installer](../installer/README.md). [Evaluation status](../eval/README.md) records the
absence of the previous automated suite. Source code is authoritative for current
behavior; the architecture contract governs future component boundaries.
