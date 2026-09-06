# Development

Voyage is a Rust 2024 workspace in first-release development. Read the
[architecture](architecture.md) for the intended component boundaries and
[current state](current-state.md) before making claims about available behavior.

## Source map

These paths describe the existing checkout, not a completed separation into three
runtime programs. Paths in backticks require the source checkout.

| Path | Current responsibility |
| --- | --- |
| `helm/src/main.rs` | Helm CLI, provider/tool construction and plain execution |
| `helm/src/tui_runtime.rs`, `helm/src/tui/` | In-process workspace runtimes, full-screen UI, input and checkpoints |
| `helm/src/agent/`, `helm/src/agent.rs` | Provider-neutral loop, cancellation, context and completion integration |
| `helm/src/provider/`, `helm/src/tools/` | Model transports and locally authorized tools |
| `helm/src/session/`, `helm/src/session.rs` | Ordinary JSON sessions, stable fences and checkpoint persistence |
| `helm/src/attachment/journal/`, `helm/src/attachment/runtime.rs` | Managed journal, exact admission, steering and cleanup obligations |
| `helm/src/managed.rs`, `helm/src/remote_worker.rs` | Foreground managed execution and dedicated remote worker |
| `helm/src/attachment/` | Enrollment, presence, sharing, explicit transfer and protocol adapters |
| `helm/src/terminal.rs`, `helm/src/subagent/`, `helm/src/todo.rs`, `helm/src/completion/` | Owned resources, task state and completion ledger |
| `helm/src/policy_profile/`, `helm/src/runtime_policy.rs` | Policy resolution, administrator ceiling and profile transitions |
| `helm/src/github/`, `helm/src/workflow/`, `helm/src/extensions/`, `helm/src/inference/` | Supporting operator workflows and accounting |
| `vessel/src/main.rs` and sibling HTTP/transport modules | Management service, authenticated API and relay |
| `crates/voyage-protocol/src/` | Shared wire types, validation and feature contracts |
| `crates/voyage-storage/src/` | Native private-storage primitives, including Windows security |
| `installer/src/` | Setup preview; provisioning is mocked |

There is currently no standalone `voyage` package. Follow the
[implementation sequence](implementation.md) when extracting that runtime.

## Build and inspect

Use stable Rust with rustfmt and Clippy and the native compiler/linker prerequisites.

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --all-targets --all-features --locked
./target/debug/helm --help
./target/debug/vessel --help
```

Model calls require separately configured credentials and authorization. Help,
configuration inspection and `helm doctor` do not require dispatching a model.
Keep native provider integration independent of the optional compatibility bridge.

## Working on a change

Follow [AGENTS.md](../AGENTS.md) and [Git workflow](local-git.md). Inspect status and
diffs first, consult relevant open and closed GitHub issues, and record scope and
acceptance before implementation. Preserve concurrent edits and unfinished work.
Update both transport ends and document negotiated compatibility when changing wire
contracts. A library primitive is not a completed operator workflow.

Automated tests and evaluation scenarios are absent. Do not claim a passing build
establishes runtime behavior, security, platform portability or provider quality.
Use [quality validation](quality.md) for the remaining gates, and record any actual
manual or separately authorized behavioral validation precisely.

Documentation-only changes need source/command checks, local link and anchor
validation, release-manifest verification and a clean diff. Do not introduce new
tests merely to validate a documentation replacement.

## Documentation ownership

`docs/architecture.md` is canonical for the target model. `docs/runtime-contract.md`
defines lifecycle and authority requirements. `docs/current-state.md` describes
implemented behavior, and operations/configuration guides document usable commands.
Keep design and current behavior explicit rather than scattering progress reports
through feature-specific documents. Git history and GitHub issues retain prior
implementation decisions and evidence.

`helm/prompts/system.md` is embedded runtime input, not a product design guide.
`helm/config.example.toml` is an executable configuration example. Change either
only when its runtime semantics are intended to change; a docs reorganization must
not inject planned capabilities into model instructions.

Update `scripts/release-documents.txt` whenever packaged guide paths change.
Generated manpages and completions come from actual CLI definitions. Keep all
relative guide links usable in both a checkout and a full extracted archive.
