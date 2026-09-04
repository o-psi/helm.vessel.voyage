# Agent supervision in Helm

Helm's full-screen interface has an agent-tree view for supervising concurrent work
without blocking the conversation or an attached terminal. Press `Ctrl+A` from the
conversation to open it. If a direct terminal is attached, detach with `Ctrl+T`
(`Ctrl+]` is also accepted) first; detaching leaves the process running.

The tree shows every agent's short ID, explicit state, elapsed time, task, hierarchy,
and latest progress. The selected row remains visible as the tree grows. `Enter`
opens the complete agent record, including parent, worktree, recent progress,
sequenced events, result, and error. `PageUp` and `PageDown` scroll that record.
The layout falls back to Helm's resize guidance below 32 columns by 10 rows.

Controls in tree and inspection views:

- `m` composes a message for the selected agent.
- `f` composes a follow-up task. `Alt+Enter` inserts a newline and `Enter` sends.
- `c`, then `c` again, requests cancellation. Terminal agents cannot be cancelled.
- `r` refreshes the snapshot and, while inspecting, its event history.
- `Esc` returns from composer to inspection, inspection to tree, and tree to chat.

Requests run asynchronously, so a slow supervision backend does not freeze input or
rendering. Live events use a bounded broadcast channel. On lag, Helm reports skipped
history and refreshes the tree; durable inspection remains available through the
adapter.

## Runtime adapter contract

The UI depends on `helm::supervision::AgentSupervisor`. Integrations provide tree
snapshots, durable sequenced history, actions, and a live event subscription. Each
snapshot includes parent, task, state, elapsed duration, optional worktree, progress,
and terminal result/error.

The concrete `helm::subagent::SubagentRuntime` exposes corresponding `tree`,
`events_after`, `send_message`, `follow_up`, `cancel`, and `subscribe` operations;
the UI has an explicit `TimedOut` status and event. The application currently installs
`NoAgentSupervisor`, so the view is empty until that runtime adapter is connected.

Supervisor composer text is sent only to the selected agent. It is not copied into
the main model prompt or session transcript. Adapters should apply the runtime's
redaction and audit policy to control-plane messages.
