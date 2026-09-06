# Helm TUI module boundaries

The local frontend exposes `helm::tui::{bridge, run, CliRequest, TuiExit}` and
re-exports `UiEvent`, `UiBridge`, `ApprovalRequest` and `QuestionRequest`.
The modules under `helm/src/tui/` are private implementation details.

The event-loop coordinator is an in-process UI responsibility, distinct from the
[coordinating Helm role](voyages.md). Every session shown by the TUI is a voyage.
Planned remote interface support must let an interface Helm view and steer a
voyage whose coordinator runs elsewhere. Local panels, response channels and
configuration drafts do not implement distributed voyage coordination, remote
interface reconnection or coordinator handoff.

## Ownership

Paths below are relative to `helm/src/`. Source and test links open the development
branch on GitHub; those files and the build commands require a source checkout.
The other guide links remain available in full release archives.

| Module | Responsibility |
| --- | --- |
| [tui_runtime.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui_runtime.rs) | Binary frontend host: builds and retains workspace runtimes, acquires target session ownership during navigation, and coordinates shutdown before actual exit or CLI handoff |
| [tui.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui.rs) | `App`, event loop, run admission/cancellation, steering, title jobs, input ownership and cross-feature coordination |
| [tui/bridge.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/bridge.rs) | Provider-neutral event channel, checkpoint messages, discovery results and approval/question response channels |
| [tui/checkpoint.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/checkpoint.rs) | Run-bound canonical/partial/accepted checkpoint requests; UI-thread persistence before acknowledgement; usage and prefix validation |
| [tui/lifecycle.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/lifecycle.rs) | Outer terminal acquisition/restoration, bracketed paste, mouse capture, keyboard enhancement flags and termination signals |
| [tui/composer.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/composer.rs) | UTF-8 editing, cursor positioning and session-local prompt recall |
| [tui/recent.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/recent.rs) | Responsive recent-conversation sidebar/drawer, selection, mouse navigation and guarded session switching |
| [tui/palette.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/palette.rs) | Shared slash-command metadata, contextual suggestions and palette rendering |
| [tui/questions.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/questions.rs) | Question-dialog state, bounded custom input and safe rendering |
| [tui/models.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/models.rs) | Model discovery requests, filtering, manual selection and picker rendering |
| [tui/supervisor.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/supervisor.rs) | Agent-panel state, inspection, messages/follow-ups, cancellation requests and rendering |
| [tui/todos.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/todos.rs) | Todo-panel state, editing, store actions and rendering |
| [tui/terminals.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/terminals.rs) | Terminal-panel state, direct PTY input, terminal-screen rendering and inventory refresh |
| [tui/workflows.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/workflows.rs) | Workflow discovery/picker, validated forms, transient secret inputs, preview/trust and final preparation; hands prepared work to the common run path |
| [tui/voyage_setup.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/voyage_setup.rs) | Saved configuration-draft library, local setup identity, transient Vessel discovery credentials, save/conflict handling and form coordination |
| [tui/voyages.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/voyages.rs) | Configuration-draft form stages: name, purpose, Helm selection, coordinator selection and review; no execution |
| [tui/tool_output.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tool_output.rs) | Tool-specific headings, bounded multiline previews, full details and terminal-safe wrapping |
| [tui/conversation.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/conversation.rs) | Transcript/activity presentation, Markdown theme and viewport anchoring |
| [tui/render.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/render.rs) | Frame composition, conversation/question layout, overlay precedence, approvals and help; delegates recent-session rendering |
| [tui/commands.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/commands.rs) | Session operations, panel entrypoints and explicit session-preserving CLI handoffs |
| [tui/text.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/text.rs) | Shared terminal-safe text and layout helpers |

The binary's `tui_runtime` host owns execution resources independently of one
`App` or model run. It retains an agent, event bridge/receiver, subagent runtime,
terminal manager, todo store and execution configuration for each canonical
workspace. The frontend borrows its active event receiver. A `TuiExit::Navigate`
request makes the host acquire and reload the target session and prepare its
workspace runtime before changing the active session. A failed target leaves the
current voyage available with an error. Only the active workspace's terminals are
shown; navigation does not imply that inactive workspace processes stopped.
Actual quit or a `TuiExit::Launch` configuration/CLI handoff takes the host's
shutdown path for retained runtimes. See [terminal attachment](terminal-attach.md)
and [session ownership](session-ownership.md) for the execution boundaries.

`App` composes `TodoPanel`, `SupervisorPanel`, `TerminalPanel`, `ModelPanel`,
`PaletteState`, `workflows::Panel`, `voyage_setup::Hub`, the composer/history and
optional question/checkpoint state. Feature handlers generally receive their
panel state and explicit dependencies. Cross-feature helpers such as `recent`
and `commands` receive `App` because they coordinate session state and navigation.
Model selection explicitly receives the session, agent and store it must update.

Completion reads a `PaletteContext`: input text, running/dismissed state,
selection, model catalog, saved sessions and workspace. It cannot mutate the
session, execute a tool or change another panel. Filesystem suggestions use
bounded directory browsing. Composer/history and question editing operate
independently of the application.

Ordinary prompts and prepared workflows enter the same `start_run` path. It checks
current policy and run scope, saves canonical user text and invocation metadata,
then dispatches through `checkpoint::UiCheckpoint`. Checkpoint acknowledgements
follow successful UI-thread persistence; mismatched, cancelled or closed requests
cannot acknowledge unsaved state. Workflow forms do not implement another executor
or apply advisory authority. Secret form values are transient bindings, not saved
workflow inputs. See [Saved workflows](saved-workflows.md).

## Keyboard and paste ownership

The event loop sends `Event::Key` and `Event::Paste` to `handle_input_event`.
Key presses continue through `handle_key`; non-press key events are ignored.
Both paths consult `input_owner` in this order:

1. Question.
2. Approval.
3. Attached PTY.
4. Already-open shortcut help.
5. Voyage configuration panel.
6. Workflow panel.
7. Model picker.
8. Supervisor panel.
9. Todo panel.
10. Terminal picker.
11. Recent-session selection.
12. Conversation composer.

Questions consume input without editing underlying panels or drafts. Approval
paste is ignored and cannot select a decision. Attached PTY paste is forwarded as
bytes without chat newline normalization or session recording. Help, pickers and
other nonediting modes consume paste; editable panel fields receive it exclusively.
Workflow and voyage panels enforce their own field, loading, preview and size
rules. Conversation paste uses the bounded composer and normalizes CRLF/CR to LF.

Keyboard-only controls remain distinct from text insertion. An open help view
consumes keys until F1 or Esc closes it. When help is closed, its F1 entrypoint is
checked after question, approval, attached-terminal and voyage/workflow handling,
before the model/supervisor/todo/picker handlers. With no panel owner, global
shortcuts precede slash completion and ordinary composer editing. Existing
active-run guards still control model changes, new voyages and panel entrypoints;
paste does not bypass them or submit a turn.

While a top-level run is active, ordinary composer submission is bounded steering
rather than another concurrent run. Accepted messages are persisted before channel
delivery and enter agent history in FIFO order at a provider boundary. Failed
saves leave the draft unsent. Receipt labels persist across status changes;
restart marks queued receipts as delivery unknown, and observed failure or
cancellation marks unapplied messages accordingly. See [steering delivery and
recovery](../helm/README.md#steering-delivery-and-recovery).

## Rendering, mouse and resize

Rendering has its own explicit branch order in `render::draw`; it is not derived
from `input_owner`. An attached terminal occupies the whole frame. Without a
question, a terminal smaller than 32×10 shows the resize notice. The other early
views, when no question is present, are help (only without an approval), model
picker, supervisor, todo, voyage configuration and workflow. Supervisor, todo,
voyage and workflow views overlay any pending approval before returning.

Otherwise the frame draws the recent sidebar and conversation. A question replaces
the bottom composer and returns before drawing the command palette or pickers,
retaining the underlying draft. Without a question, the composer/palette is drawn,
then the recent-session drawer when needed, then the terminal picker, and finally
any approval. Session selection uses the sidebar in landscape and a drawer when
that sidebar is unavailable. Question/approval admission refuses conflicting
direct-terminal interaction, and normal keyboard guards prevent opening the model
picker during an active run.

Mouse handling first gives `recent::mouse` a chance to consume a hit inside the
visible sidebar/drawer. It refuses input while another modal or panel owns the
view. Recent-list wheel events move selection; a valid click requests guarded
session navigation. Outside that area, conversation wheel scrolling runs only
when no question, approval, help, picker or feature panel is open. Mouse input does
not fall through into a hidden conversation.

The event loop computes `conversation_layout` for terminal-size changes and calls
`resize_conversation` to maintain scroll bounds. Layout accounts for the responsive
sidebar and question height, reserving conversation space as questions grow.
An attached PTY receives the new columns and rows minus its status row. The outer
Ratatui terminal is resized to clear stale cells, and subsequent redraws refresh
pixel-based orientation when available. Resize does not submit input or change
execution authority.

Helm enables Crossterm's level-triggered Unix event reader so simultaneous resize
and input readiness cannot leave a paste waiting for another keystroke. The
resize/paste regression sends repeated pairs without an extra wake-up key.

## Regression checks

[Shared fixtures](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tests.rs) and the `tests/` modules cover commands,
composer, conversation, palette, questions, supervisor, terminals, todos and
cross-module routing. Additional tests live with `recent`, `checkpoint`,
`voyage_setup`, `voyages`, [workflow forms](https://github.com/o-psi/voyage/blob/main/helm/src/tui/workflows_tests.rs) and
tool-output rendering.

| Contract | Source assertions and fixtures |
| --- | --- |
| Shared key/paste ownership and hidden-draft isolation | [routing tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tests/routing.rs): `central_router_preserves_modal_and_panel_precedence`, `paste_owner_matrix_is_exclusive_during_idle_and_active_steering`, question/approval/PTY and workflow/voyage paste tests; [real PTY fixture](https://github.com/o-psi/voyage/blob/main/tests/system/paste_routing.py) |
| Question layout and exclusive input | [question tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tests/questions.rs), including underlying-view restoration and narrow rendering; [real PTY fixture](https://github.com/o-psi/voyage/blob/main/tests/system/questions.py) |
| Sidebar orientation, guarded navigation and mouse ownership | [recent tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/recent.rs); [real voyage UI fixture](https://github.com/o-psi/voyage/blob/main/tests/system/voyage_ui.py) |
| Conversation scrolling and resize bounds | [conversation tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tests/conversation.rs) and question/layout tests |
| Save-before-acknowledgement checkpoints | [checkpoint tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/checkpoint.rs), including cancellation, closed channels, failed saves and mismatched canonical state |
| Steering persistence and provider-boundary delivery | [routing/conversation tests](https://github.com/o-psi/voyage/blob/main/helm/src/tui/tests/routing.rs); [offline steering fixture](https://github.com/o-psi/voyage/blob/main/tests/system/steering.py) |

Run from the repository root:

```sh
cargo test -p helm --lib tui::
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release --locked
HELM_BIN=target/release/helm python3 tests/system/paste_routing.py
HELM_BIN=target/release/helm python3 tests/system/questions.py
HELM_BIN=target/release/helm python3 tests/system/voyage_ui.py
HELM_BIN=target/release/helm python3 tests/system/steering.py
```

These offline fixtures exercise actual terminal and provider boundaries. They do
not establish native macOS/Windows coverage or deployed/live-provider acceptance.
