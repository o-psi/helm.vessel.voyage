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
| [Operations](operations.md) | Current chat, managed-session and remote-worker procedures |
| [Security](security.md) | Authority, credentials, disclosure and terminal safety |
| [Development](development.md) | Source layout, build commands and contribution workflow |
| [Quality](quality.md) | The remaining non-test checks and limits of their evidence |
| [Releasing](releasing.md) | Archive contents, packaging and publication boundaries |
| [Local Git](local-git.md) | GitHub and this workspace's Git wrapper |

The component entrypoints are [Helm](../helm/README.md) and
[Vessel](../vessel/README.md). [Evaluation status](../eval/README.md) records the
absence of the previous automated suite. Source code is authoritative for current
behavior; the architecture contract governs future component boundaries.
