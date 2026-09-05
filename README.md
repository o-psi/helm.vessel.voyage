# Voyage

Voyage is a system for general-purpose LLM work across local and remote machines.

Voyage is in development toward its first release, with a focus on complete,
reliable workflows that deliver tested value.

- **Helm** is the Rust TUI and agent runtime for local work.
- **Vessel** is the management plane for remote session management. It currently
  provides health checks, an authenticated status UI and optional outbound Helm presence.
- **voyage-protocol** holds shared management-plane and attachment protocol types.

- **voyage-storage** supplies native private enrollment storage on Windows, shared
  by Helm and Vessel without adding filesystem concerns to the wire protocol.

## Run locally

```sh
cargo build --workspace
cargo run -p helm -- chat
cargo run -p vessel -- --bind 127.0.0.1:9480
```

Helm runs locally. [Authenticated attachment presence](docs/attachment-presence.md)
connects enrolled Helm machines to Vessel. Remote session management remains planned.

See [Helm's README](helm/README.md) for provider, policy, session, and CLI details.
Helm's native providers do not require Codex; see [provider architecture](docs/providers.md) for transport choices and subscription-versus-API
billing boundaries.

For self-hosted source control and issue management, see [the local Forgejo guide](docs/local-git.md).
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

## Planned Vessel session management

The agreed [Vessel-managed session design](docs/vessel-session-management.md) specifies
`helm attach VESSEL_URL JOIN_KEY`, outbound interactive control, explicit sharing,
and phased service/approval support. These are planned capabilities, not current
CLI commands. Delivery is tracked in [#77](https://github.com/o-psi/voyage/issues/77).
