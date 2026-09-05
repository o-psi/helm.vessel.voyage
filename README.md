# Voyage

Voyage is a system for general-purpose LLM work across local and remote machines.

Voyage is in development toward its first release, with a focus on complete,
reliable workflows that deliver tested value.

- **Helm** is the Rust TUI and agent runtime. It is the intended operator interface
  for local and remote work through Vessel; the unified remote interface is planned.
- **Vessel** is the management plane for remote session management. It currently
  provides health checks, an authenticated status UI, outbound Helm presence, and
  explicitly enabled dedicated remote-session HTTP operations.
- **voyage-protocol** holds shared management-plane and attachment protocol types.

- **voyage-storage** supplies native private enrollment storage on Windows, shared
  by Helm and Vessel without adding filesystem concerns to the wire protocol.

## Run locally

Try the interactive [setup preview](docs/installer-preview.md) to explore local,
remote and Vessel setup choices. All setup actions in that wizard are mocked.

```sh
cargo build --workspace
cargo run -p helm -- chat
cargo run -p vessel -- --bind 127.0.0.1:9480
```

Helm runs locally. [Authenticated attachment presence](docs/attachment-presence.md)
connects enrolled Helm machines to Vessel. [Dedicated remote sessions](docs/remote-sessions.md)
add opt-in foreground execution with authenticated list, submit, watch and cancel APIs.

See [Helm's README](helm/README.md) for provider, policy, session, and CLI details.
[Private managed sessions](docs/local-managed-sessions.md) support local CLI turns,
exact retries, cancellation, and explicit recovery without enabling remote sharing.
Helm's native providers do not require Codex; see [provider architecture](docs/providers.md) for transport choices and subscription-versus-API
billing boundaries.

For GitHub collaboration and this workspace’s Git wrapper, see [the Git guide](docs/local-git.md).
The first-version work breakdown is tracked in [the delivery map](docs/roadmap.md).

See the [evaluation suite](eval/README.md) for representative task checks and the
[release procedure](docs/releasing.md) for build and verification requirements.

Helm also supports bounded parallel child agents with explicit permissions,
supervision, result collection, persistence, and optional Git worktree isolation.
The lifecycle and safety contract is documented in [Parallel subagents](docs/subagents.md).

Helm also provides durable, dependency-aware task tracking shared by the model and operator.
See [Task management](docs/task-management.md) for the todo tool contract and workflow.

Available models are discovered from the configured account and can be switched without losing
the active session. See [Model discovery and switching](docs/model-management.md).

Helm derives its model-facing capability contract from the live registry and never persists runtime
instructions as conversation history. See [Runtime guidance](docs/runtime-guidance.md).

Assistant responses use safe, streaming-aware Markdown presentation without changing the canonical
session text. See [Markdown rendering](docs/markdown-rendering.md) for syntax, fallback, and terminal
safety behavior.

## Voyages across Helms

The agreed [voyage model](docs/voyages.md) is an open-ended session across a
user-scoped set of Helms. The interface Helm and coordinating Helm are separate
roles: coordination can run remotely while the user connects from a workstation.
Work can move among permitted participants without a required repository, component
map or permanent assignment to one machine.

The [Helm voyage setup UI](docs/helm-voyage-ui.md) provides a responsive recent-conversation sidebar and explicit machine/coordinator selection in saved drafts. Drafts do not start remote work.

The [session management design](docs/vessel-session-management.md) describes the
supporting lifecycle, authority and sharing boundaries. Multi-Helm coordination,
coordinator handoff and the Helm management interface remain planned. Current
`helm remote-worker` provides one dedicated foreground session with HTTP operations.
Delivery is tracked in [#77](https://github.com/o-psi/voyage/issues/77).
