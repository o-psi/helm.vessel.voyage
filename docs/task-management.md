# Task management

Helm maintains a durable task list for each workspace. The operator and model use the same store,
so plans remain inspectable across turns and restarts instead of living only in chat history.

The model-facing `todo` tool supports concise structured operations:

- `create`, `list`, and `edit` define and inspect work.
- `status` moves an item through its lifecycle; dependency checks require prerequisites to be
  completed before starting or completing dependent work.
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
paths are represented by a stable key, so changing the active workspace selects its own plan.
This local storage boundary does not define the scope of a planned [voyage](voyages.md).
A voyage may span several permitted Helms and workspaces without a required project
or component map. Current todo assignment targets local subagents; it does not
route work to a remote Helm or establish cross-machine completion evidence.

## Action arguments

Every action accepts only the fields listed here. IDs are actual UUID strings;
examples in the advertised schema illustrate shapes and must not be reused as
real task IDs. Unsupported, missing and irrelevant fields are rejected.

| Action | Required fields besides `action` | Optional fields |
| --- | --- | --- |
| `create` | `title` | `description`, `priority`, `order`, `assignees` |
| `list` | none | `include_archived`, `status` |
| `edit` | `id` | `title`, `description`, `priority` |
| `status` | `id`, `status` | none |
| `block` | `id` | `blockers` |
| `dependencies` | `id` | `add`, `remove` |
| `assign` | `id` | `assignees` |
| `note`, `progress`, `evidence` | `id`, `text` | `author` |
| `reorder` | `id`, `order` | none |
| `remove`, `archive` | `id` | none |
| `clear_completed` | none | none |

Creation defaults to empty description, normal priority, the next ordering
position and no assignees. Lists exclude archived items unless requested; omitted
or null status means no status filter. Editing omitted or null fields leaves
those fields unchanged. Optional create `order` and timeline `author` accept null;
other array/boolean defaults do not. Orders are signed 64-bit integers.

Use `block` with `id` and a nonempty `blockers` array to explain blocked work:

```json
{"action":"block","id":"00112233-4455-4677-8899-aabbccddeeff","blockers":["Awaiting operator approval"]}
```

Sending `blockers: []`, or omitting blockers, clears them and moves a blocked item
to pending. `status` does not accept `text` and cannot invent blocker reasons.
Use `note`, `progress` or `evidence` for the appropriate timeline entry. Evidence
in a title/progress note does not replace a structured evidence entry or
[run completion accounting](completion-tool-contract.md).

Omitted `assign.assignees` clears assignments; omitted dependency `add`/`remove`
lists are empty. `archive` accepts one completed/cancelled item. `clear_completed`
archives **all completed items**, accepts no `id`, and does not remove them. Older
versions silently ignored extra fields for this unit action; they now reject the
call before any mutation. The action-only valid shape is unchanged; no storage
migration is needed. Archival/removal cannot erase run-owned completion obligations.

The schema uses shared object properties plus one branch per action, preserving
native transport modes described in the [completion tool contract](completion-tool-contract.md).
Independent schema/decoder corpus tests cover canonical JSON argument shapes,
option/default/null behavior, malformed fields and numeric bounds. Ordinary and
streaming HTTP fixtures inspect all actions through Chat, Responses and Anthropic.
Durable regressions cover rejected bulk archival and recovery from blocked-status
errors. These fixtures do not prove successful live model reconciliation; no new
live attempt is part of this correction.
