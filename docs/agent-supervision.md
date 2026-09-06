# Agent supervision in Helm

Helm's full-screen interface has an agent-tree view for supervising concurrent work
without blocking the conversation or an attached terminal. Press `Ctrl+A` from the
conversation to open it. If a direct terminal is attached, detach with `Ctrl+T`
(`Ctrl+]` is also accepted) first; detaching leaves the process running.

The tree shows each retained agent's short ID, explicit state, elapsed time, task,
hierarchy, and latest progress. Its header and refresh status distinguish active
agents from retained terminal history. Finished terminal leaves archive automatically;
active agents and ancestors required by retained children stay retained. Completed
non-worktree leaves archive even under a live parent. Completed worktree records
stay retained while ancestors are active to permit commit and integration. There
is no fixed retained-record capacity. Models can browse them with `subagent` action
`archive` and retrieve their original IDs with `status` or `wait`; see
[archive behavior and retention](subagents.md#automatic-archive). The selected row remains visible as the tree grows. `Enter`
opens the complete agent record, including parent, worktree, recent progress,
sequenced events, result, and error. `PageUp` and `PageDown` scroll that record.
The layout falls back to Helm's resize guidance below 32 columns by 10 rows.

Controls in tree and inspection views:

- `m` composes a message for the selected agent.
- `f` composes a follow-up task. `Shift+Enter` inserts a newline and `Enter` sends.
- `c`, then `c` again, requests cancellation. Terminal agents cannot be cancelled.
- `r` refreshes the snapshot and, while inspecting, its event history.
- `Esc` returns from composer to inspection, inspection to tree, and tree to chat.

Requests run asynchronously, so a slow supervision backend does not freeze input or
rendering. Live events use bounded text previews and a bounded broadcast channel;
full stored results remain available through inspection or `status`/`wait`. On lag, Helm reports skipped
history and refreshes the tree; durable inspection remains available through the
adapter.

Actively running subagents default to half the logical processors available to the
process, rounded down with a minimum of one. An explicit positive
`subagent_max_concurrency` overrides this value. Agents blocked in `wait` or
`wait_many` release their slot and reacquire it before continuing. Entire subagent
tasks have no automatic two-minute deadline. Message/follow-up delivery also
yields a sender’s slot when a full mailbox needs its recipient to run.

This view supervises children within the current Helm runtime. The planned
[remote voyage interface](voyages.md) must also expose work on participating Helms and
identify the coordinating Helm independently of the Helm displaying the interface.
The current agent tree does not provide that remote routing or authority.

## Runtime adapter contract

The UI depends on `helm::supervision::AgentSupervisor`. Integrations provide tree
snapshots, durable sequenced history, actions, and a live event subscription. Each
snapshot includes parent, task, state, elapsed duration, optional worktree, progress,
and terminal result/error.

The concrete `helm::subagent::SubagentRuntime` exposes corresponding `tree`,
`events_after`, `send_message`, `follow_up`, `cancel`, and `subscribe` operations;
the UI preserves distinct `TimedOut`, `Interrupted`, and `Cancelled` terminal states
and events. The application installs `RuntimeAgentSupervisor`, which maps the
durable runtime into this UI contract and forwards bounded live events without
exposing task handles.

Supervisor composer text is sent only to the selected agent. It is not copied into
the main model prompt or session transcript. Adapters should apply the runtime's
redaction and audit policy to control-plane messages.
