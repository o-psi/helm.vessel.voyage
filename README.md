# Voyage

Voyage is a system for general-purpose LLM work across local and remote machines.

**Voyage has not had a product release. We are developing its first version.**
The target is a release-quality product that delivers practical, tested value:
complete user workflows, reliable behavior, security, and usable documentation.
An MVP or vertical slice does not satisfy that delivery standard. Current source
builds are development builds; implemented capabilities and passing individual
checks do not establish overall release readiness.

- **Helm** is the Rust TUI and agent runtime for local work.
- **Vessel** is the management plane, currently retaining health/readiness and an
  authenticated status UI while its connection model is rebuilt.
- **voyage-protocol** holds shared management-plane response types; the replacement
  attachment wire contract is not implemented yet.

- **voyage-storage** supplies native private enrollment storage on Windows, shared
  by Helm and Vessel without adding filesystem concerns to the wire protocol.

## Run locally

```sh
cargo build --workspace
cargo run -p helm -- chat
cargo run -p vessel -- --bind 127.0.0.1:9480
```

**Helm-to-Vessel connectivity is temporarily unavailable.** Legacy pairing and
HTTP task workers have been removed as an intentional clean break. Local Helm
functionality remains available. See [retirement and data preservation](docs/vessel-connectivity-retirement.md)
before updating existing development installations. No saved sessions or legacy enrollment/database
data are automatically deleted or reused.

See [Helm's README](helm/README.md) for provider, policy, session, and CLI details.
Helm's native providers do not require Codex; see [provider architecture and
migration](docs/providers.md) for transport choices and subscription-versus-API
billing boundaries.

For self-hosted source control and issue management, see [the local Forgejo guide](docs/local-git.md).
The first-version work breakdown is tracked in [the delivery map](docs/roadmap.md).

First-release and replacement readiness require verification evidence. The repository
contains cross-platform CI and future release workflows, a representative
[evaluation suite](eval/README.md), the [release procedure](docs/releasing.md), and the
operator [cutover and rollback runbook](docs/cutover.md). These prepare for release;
their presence, Cargo's `--release` build profile, and locally generated archives do
not mean a product version has been released.

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
CLI commands; the legacy path has been removed, not retained as a fallback. Delivery is
tracked in [#77](https://github.com/o-psi/voyage/issues/77).
