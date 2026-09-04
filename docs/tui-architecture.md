# Helm TUI module boundaries

The public entrypoints remain `helm::tui::{bridge, run, CliRequest, TuiExit}`.
The event/response types (`UiEvent`, `UiBridge`, `ApprovalRequest`, and
`QuestionRequest`) retain their existing paths through re-exports. The modules
under `helm/src/tui/` are private implementation details, not new public APIs.

## Ownership

| Module | Responsibility |
| --- | --- |
| `tui.rs` | Application composition, event loop, run cancellation, input precedence and cross-feature coordination |
| `bridge.rs` | Provider-neutral event channel and approval/question response channels |
| `lifecycle.rs` | Outer terminal acquisition/restoration and termination signals |
| `composer.rs` | UTF-8 editing, cursor positioning, session-local prompt recall and draft restoration |
| `palette.rs` | Shared slash-command metadata, contextual suggestions and palette rendering |
| `questions.rs` | Question-dialog state, bounded custom input and safe rendering |
| `models.rs` | Model-panel state, discovery requests, selection and rendering |
| `supervisor.rs` | Agent-panel state, navigation, messages/follow-ups, cancellation requests and rendering |
| `todos.rs` | Todo-panel state, editing, store actions and rendering |
| `terminals.rs` | Terminal-panel state, direct PTY input, screen rendering and inventory refresh |
| `conversation.rs` | Transcript/activity presentation, Markdown theme and viewport anchoring |
| `render.rs` | Frame composition, overlay precedence, session picker, approvals and help |
| `commands.rs` | Session operations and explicit session-preserving CLI handoffs |
| `text.rs` | Small shared text and layout helpers |

`App` composes `TodoPanel`, `SupervisorPanel`, `TerminalPanel`, `ModelPanel` and
`PaletteState`. Panel handlers/renderers receive their own state, not the whole
application. A status-message output is passed separately when needed. Model
selection explicitly receives the session and runtime it must update.

Completion reads a `PaletteContext`: input text, running/dismissed state,
selection, model catalog, saved sessions and workspace. It cannot mutate the
session, execute a tool or change another panel. Filesystem suggestions retain
the existing bounded directory-browsing behavior. Composer/history and question
editing also operate independently of `App`.

Cross-feature operations intentionally remain at the application boundary:
commands can change sessions or request a relaunch; conversation presentation
reads canonical and streaming history; frame composition decides which view is
visible. Moving those operations does not change persistence, provider, tool or
approval policy.

## Routing invariants

The main keyboard router retains its existing order: questions, approvals,
attached PTY, shortcut help, model panel, supervisor panel, todo panel, terminal
picker, session picker, global shortcuts, command palette, then chat editing.
Paste, mouse and resize routing stay in the coordinator too. Frame composition
keeps its existing explicit overlay order; this refactor does not redefine modal
behavior.

- Direct attached-terminal input never becomes chat or model-visible input.
- Dialog answers do not edit underlying drafts or panels.
- Panel navigation does not submit a model turn.
- Run and terminal guards preserve cancellation and terminal restoration.
- CLI handoffs still save the session before requesting relaunch.

New features should add state and behavior to their owning module, then wire
routing at the coordinator. Do not import `App` into an independent panel merely
to avoid specifying its dependencies, or introduce wildcard imports between
production feature modules.

## Regression checks

`helm/src/tui/tests.rs` holds shared fixtures; its `tests/` children group coverage
by commands, composer, conversation, palette, questions, supervisor, terminals,
todos and cross-module routing. All 48 tests from the original single-file TUI
were retained during extraction. Additional tests exercise the real central
keyboard router across overlapping views, completion without an application, and
todo editing without conversation state.

Run from the repository root:

```sh
cargo test -p helm --lib tui::
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release --locked
HELM_BIN=target/release/helm python3 tests/system/questions.py
HELM_BIN=target/release/helm python3 tests/system/native_provider_no_codex.py
```

The questions fixture exercises the real TUI through a PTY with an offline native
provider, including resize, selected/custom answers, cancellation, expiry,
continuation and persistence. Unit/render fixtures and this Linux PTY test do not
substitute for macOS/Windows CI or live operator acceptance. There is no wire or
session-format migration for this module-only refactor.

Tracking: [god-file audit and TUI cleanup #84](https://github.com/o-psi/voyage/issues/84).
