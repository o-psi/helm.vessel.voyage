# Development

Voyage is a Rust 2024 workspace in first-release development. Read the
[architecture](architecture.md) for the intended component boundaries and
[current state](current-state.md) before making claims about available behavior.

## Source map

Runtime execution belongs to `voyage/`. Helm modules contain protocol clients and
operator interfaces. Paths in backticks require the source checkout.

| Path | Current responsibility |
| --- | --- |
| `helm/src/main.rs`, `helm/src/cli.rs`, `helm/src/diagnostics.rs` | CLI dispatch, grammar and execution-host diagnostics clients |
| `helm/src/process_client/` | Connected CLI, local/SSH/grant transports, lifecycle/courier clients and multiplexer |
| `helm/src/markdown.rs`, `helm/src/onboarding/` | Rendering and repository onboarding |
| `voyage/src/main.rs`, `voyage/src/server/` | Independent runtime entrypoint, private transport and session command dispatch |
| `voyage/src/agent.rs`, `voyage/src/agent/`, `voyage/src/context.rs` | Provider-neutral loop, cancellation, context and completion integration |
| `voyage/src/provider/`, `voyage/src/tools/`, `voyage/src/config.rs` | Provider transports, local tools and execution configuration |
| `voyage/src/session/`, `voyage/src/session.rs` | Legacy JSON sessions, identity fences and checkpoints |
| `voyage/src/attachment/journal/`, `voyage/src/attachment/runtime.rs` | Canonical managed journals, exact admission, steering and cleanup |
| `voyage/src/build/`, `voyage/src/execution.rs` | Runtime construction and admitted execution, per-process resource root |
| `voyage/src/terminal.rs`, `voyage/src/subagent/`, `voyage/src/todo.rs`, `voyage/src/completion/` | Resources, task state and completion accounting |
| `voyage/src/policy_profile/`, `voyage/src/runtime_policy.rs` | Local policy, administrator ceilings and profile transitions |
| `voyage/src/github/`, `voyage/src/workflow/`, `voyage/src/extensions/`, `voyage/src/inference/` | Execution services and shared legacy operator workflows |
| `helm/src/managed.rs`, `helm/src/remote_worker.rs` | Thin supervised managed and outbound-worker clients |
| `vessel/src/process/` | Linux launch, private registry, routing and conservative stop/restart |
| `vessel/src/main.rs` and HTTP/transport modules | Management, enrollment, scoped process gateway and compatibility relay |
| `crates/voyage-protocol/src/process/` | Versioned bounded Helm–Vessel–voyage messages |
| `crates/voyage-storage/src/` | Native private-storage primitives |
| `installer/src/service/` | Linux private installation and explicit service lifecycle |
| `installer/src/install/` | Verified release manifests, atomic installation and rollback |
| `installer/src/cli.rs`, `flow.rs`, `ui.rs` | Installer arguments, review/apply planning and interactive UI |

`helm/src/lib.rs` re-exports shared configuration/presentation types; executable
session construction and canonical mutation remain in voyage. New code is split
by transport, lifecycle, authorization, storage and resource responsibility. Keep
these boundaries when extending the command surface.

## Build and inspect

Use stable Rust with rustfmt and Clippy and the native compiler/linker prerequisites.

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --all-targets --all-features --locked
./target/debug/helm --help
./target/debug/vessel --help
./target/debug/voyage --help
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

The previous automated suites and evaluation scenarios were removed. The targeted
concurrent-voyage regression now runs through the [quality gates](quality.md);
broader suite recreation remains separate. Do not claim a passing build establishes
runtime behavior, security, platform portability or provider quality. Record actual
behavioral validation and its scope precisely.

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

`voyage/prompts/system.md` is embedded runtime input, not a product design guide.
`helm/config.example.toml` is an executable configuration example. Change either
only when its runtime semantics are intended to change; a docs reorganization must
not inject planned capabilities into model instructions.

Update `scripts/release-documents.txt` whenever packaged guide paths change.
Generated manpages and completions come from actual CLI definitions. Keep all
relative guide links usable in both a checkout and a full extracted archive.
