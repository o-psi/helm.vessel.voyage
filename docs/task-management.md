# Task management

Helm maintains a durable task list for each workspace. The operator and model use the same store,
so plans remain inspectable across turns and restarts instead of living only in chat history.

The model-facing `todo` tool supports concise structured operations:

- `create`, `list`, and `edit` define and inspect work.
- `status` moves an item through its lifecycle; dependency checks prevent dishonest completion.
  Set the status to `pending` to reopen completed, cancelled, or blocked work.
- `block` records a concrete blocker.
- `dependencies` incrementally adds and removes prerequisites and rejects cycles.
- `assign` associates an item with a Helm subagent or clears the assignment.
- `note` appends general context, `progress` appends a short checkpoint, and `evidence` appends a
  verifiable result or artifact reference.
- `reorder` changes presentation/execution order without changing identity.
- `remove`, `archive`, and `clear_completed` provide explicit cleanup operations.

Task IDs are stable UUIDs. Mutations are serialized and written atomically. Call `list` after a
failed mutation rather than guessing current state. Completing an item should follow the real work
and include evidence where verification matters. Archiving preserves history; removal is intended
for erroneous or unwanted records.

Task data is scoped to the active workspace and stored below Helm's data directory. Workspace
paths are represented by a stable key, so opening another project cannot expose or alter this
project's plan.
