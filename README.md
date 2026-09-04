# Voyage

Voyage is a system for general-purpose LLM work across local and remote machines.

- **Helm** is the Rust TUI and agent runtime. `helm --voyage` pairs it with a Vessel;
  Helm then maintains an outbound worker connection while preserving local policy.
- **Vessel** is the management plane. It claims pairing strings, tracks liveness,
  queues work, and dispatches it over connections initiated by Helms.
- **voyage-protocol** owns the versioned wire types shared by both binaries.

```text
operator ── Helm TUI ── local tools
    │
    └── Vessel ◀── outbound paired Helm worker(s) ── scoped tools/workspaces
```

## Run locally

```sh
cargo build --workspace

# management plane
cargo run -p vessel -- --bind 127.0.0.1:9480

# In another terminal: print a one-time pairing string and wait
OPENAI_API_KEY=... cargo run -p helm -- --voyage --name workstation

# Give the printed string to Vessel
cargo run -p vessel -- pair voyage:v1:XXXXXXXXXX
```

Helm does not listen on a public port. Pairing codes expire after ten minutes. Once
claimed, Helm authenticates outbound heartbeats, pulls queued tasks, executes them
under local policy, and posts results to Vessel.

See [Helm's README](helm/README.md) for provider, policy, session, and CLI details.
Helm's native providers do not require Codex; see [provider architecture and
migration](docs/providers.md) for transport choices and subscription-versus-API
billing boundaries.

For self-hosted source control and issue management, see [the local Forgejo guide](docs/local-git.md).
The comprehensive 0.1 work breakdown is tracked in [the delivery map](docs/roadmap.md).

Release and replacement readiness are executable, not informal. The repository ships
cross-platform CI/release workflows, a representative [evaluation suite](eval/README.md),
the [release procedure](docs/releasing.md), and the operator [cutover and rollback
runbook](docs/cutover.md).

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
