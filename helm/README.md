# Helm

Helm is a Rust LLM harness for general work through a terminal. It is intentionally
not coding-specific: the runtime can inspect and transform files, run commands,
administer scoped systems, and maintain a durable working conversation.

## Capabilities

- Provider-neutral agent loop with OpenAI-compatible and Anthropic adapters
- Native tool calling across repeated model/tool turns
- File reading/writing, SHA-guarded atomic patching, directory traversal, and search
- One-shot shell execution plus persistent PTY processes with incremental I/O and resize
- MCP stdio server discovery with collision-resistant namespaced tools
- Typed provider failures, transient retry/backoff, and cancellation-aware execution
- Workspace confinement with explicit extra read/write roots and symlink-aware checks
- Configurable approvals, deny list, command timeout, and output limits
- Interactive chat, one-shot tasks, session resume/listing, and token accounting
- `helm --voyage` short-lived pairing and authenticated outbound task worker
- Atomic JSON session persistence under the platform data directory
- Library interfaces for custom providers, event sinks, approvers, and tools

Helm treats safety as a runtime boundary, not a prompt convention. Filesystem tools
resolve paths against allowed roots; shell tools classify consequential commands,
request approval according to policy, and kill timed-out child processes.

## Build and configure

```sh
cargo build --release
mkdir -p ~/.config/helm
cp config.example.toml ~/.config/helm/config.toml
export OPENAI_API_KEY=...
```

For Anthropic, set `provider = "anthropic"`, select a Claude model, and export
`ANTHROPIC_API_KEY`. An OpenAI-compatible local service can be selected with
`base_url`; its API key variable may contain any value if the service ignores auth.

Inspect the resolved configuration without contacting a provider:

```sh
helm config
```

## Use

```sh
# Interactive work in the current directory
helm chat

# One-shot work and an explicit workspace
helm run "inventory the log files and summarize unusual failures"
helm --workspace /srv/example --approval always run "inspect service health"

# Find and continue durable sessions
helm sessions
helm run --resume 0198... "continue, but export the findings as markdown"
```

Inside chat, `/session` shows identity and token use, `/clear` resets conversation
history, and `/exit` saves and leaves. Tool progress goes to stderr, leaving assistant
text on stdout for straightforward scripting.

Pair the same Helm runtime with a Vessel without exposing an inbound port:

```sh
helm --voyage https://vessel.example.com --name workstation
# prints voyage:v1:XXXXXXXXXX; claim it in Vessel
```

## Architecture

The crate is split around stable boundaries:

- `provider`: translates the internal message/tool protocol to remote APIs
- `agent`: owns the bounded model → tool → model state machine
- `tools`: capability registry and execution context
- `policy`: validates filesystem and process actions before execution
- `session`: durable, provider-independent conversation records
- `config`: TOML defaults plus CLI overrides and environment-held credentials

This keeps the CLI replaceable and lets future frontends or automation embed Helm as
a library. Provider payloads do not leak into saved sessions.

## Security model

The active workspace is the default read/write boundary. Add other roots explicitly.
`on-risk` asks before replacing files and before commands that appear mutating or
privileged. `always` asks for every shell command and write. `never` suppresses asks,
but the command deny list and filesystem roots remain enforced.

Vessel queues work and Helm retrieves it over authenticated outbound requests. A Helm
can therefore operate behind NAT or a firewall without being publicly reachable.

This is capability control, not an OS sandbox. Shell commands inherit the user's OS
permissions and can access resources available to that account. For hostile prompts
or untrusted data, run Helm in a container or restricted service account as well.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The terminal frontend is currently the functional shell beneath the planned full-screen
TUI. Runtime facilities for cancellation, managed terminals, MCP tools, and conflict-safe
patches are exposed through the library so that frontend can represent them without
reimplementing execution policy.
