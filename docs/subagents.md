# Parallel subagents

Helm subagents are child workers owned by a parent agent. They are useful
when several independent tasks can proceed at once; they are not background
terminals and do not silently inherit unlimited authority.

These are local child agents, including when their owning Helm runs on a remote
host. A child ID is not a participant Helm identity. Every Helm session is a
[voyage](voyages.md); planned cross-Helm coordination extends voyages across
explicitly scoped machines without requiring a project or worktree. That
delegation transport is separate from the current local `subagent` tool. Each
executing Helm must enforce its own authority.

## Lifecycle

Every child has a stable ID, parent ID, task, status, timestamps, policy, budget,
optional worktree, progress history, and final result. A child moves through
queued, running, waiting, and one terminal state: completed, failed, cancelled,
or interrupted. Historical timed-out records remain readable. A restarted Helm marks work it cannot reattach as interrupted
instead of pretending it is still running.

The parent can start a child, inspect the tree, wait for one or more children,
send a message, assign a follow-up task after completion, or cancel outstanding
work. Results are attributed to their child and returned to the parent in a
structured form. Live events contain bounded text previews; `status` and `wait`
provide the full stored result. Messages and lifecycle events carry monotonically
increasing sequence numbers so consumers can recover without guessing order.

## Concurrency and authority

By default, actively running subagents use half the logical processors reported by
Rust's `available_parallelism`, rounded down with a minimum of one. This reflects
processors available to the process, which may differ from the machine's physical
core count; if detection fails, the default is one. A positive
`subagent_max_concurrency` explicitly overrides the default. Excess work waits in
a visible queue. A subagent calling `wait` or `wait_many` releases its execution
slot and reacquires one before continuing, so waiting parents do not prevent
queued children from running. A full message mailbox also releases the sender’s
slot while delivery is backpressured, then reacquires it before continuing.

Subagent tasks have no elapsed-time, child-count, nesting-depth, or retained-record
cutoff. The shell-command timeout does not set a deadline for the whole assignment.
Provider response-token settings and local tool output, timeout, and authority
controls still apply. Child permissions are no broader than the intersection of
its requested policy and its parent policy. An unattended child cannot answer an
approval prompt and cannot use the TUI keyboard as approval.

Memory management uses bounded event previews, backpressure for message delivery,
and disk archives for completed records. It does not currently page live model
transcripts to disk or adjust admission based on measured memory pressure. Large
active conversations, queued tasks, and retained worktree records can still grow
memory use.

Cancellation signals inherited tokens and stops the Helm-owned executor future.
Cancelling a parent also cancels its unfinished descendants. A child
failure is recorded and reported without cancelling healthy siblings.

## Git worktrees

Coding children may request an isolated Git worktree. Helm creates it beneath a
dedicated data directory using validated branch and path names, records ownership,
and never discards an existing dirty worktree. Integration must detect overlapping
changes and report conflicts to the parent. Cleanup is explicit and refuses dirty
or unowned paths; Helm never uses destructive Git recovery to make cleanup pass.

General work does not require Git and can run in a policy-scoped directory without
a worktree.

## TUI supervision

The agent-tree view shows hierarchy, task, status, elapsed time, worktree, and
recent progress. From that view the operator can inspect a child, send a message,
assign follow-up work, cancel it, or return to the conversation. The view remains
responsive while children run and does not interfere with direct terminal attach.

## Completion evidence

Required regression coverage includes CPU-derived concurrency and explicit overrides;
work beyond the former two-minute deadline, 64-record capacity, and eight-child
budget; nested waits with one execution slot; cancellation while queued or waiting
to resume; and policy denial. Storage checks cover completed children archived
under live parents, retained worktree evidence, large event previews with full
results preserved, archive failure/retry, and restart recovery. Provider fixtures
must exercise actual nested tool calls and shell-timeout independence. The parent
conversation must remain responsive through every scenario. These requirements
are a test plan, not a claim that every check has run.

## Implementation boundaries

`AgentTreeStore` persists records through a synced temporary file and atomic rename.
The runtime calls `recover_after_restart` once during startup. `AgentPolicy` validates
that child roots, tools, approvals, and budgets only narrow parent authority.
`WorktreeManager::conflicts` identifies files changed independently by sibling
branches before guarded integration. Clean worktrees may be removed, but their
branches and commits remain recoverable.

Model/tool loops have no turn-count cutoff. Legacy `max_turns`,
`max_runtime_secs`, and `max_children` fields in saved child budgets are ignored
on load and omitted on the next save. Legacy `subagent_max_agents` configuration
is also ignored on load and omitted on save; obsolete runtime overrides are
rejected rather than silently accepted. Existing explicit positive
`subagent_max_concurrency` settings continue to apply; remove the setting to use
the CPU-derived default.

Older Helm binaries that require removed budget fields cannot directly load newly
saved child trees or archive records. Before upgrading, retain a backup of the data directory if rollback
is needed; do not overwrite current task history with that backup implicitly.

## Automatic archive

Finished subagents archive automatically after completion, failure, cancellation,
and restart recovery. Historical timeout outcomes are preserved. Original
IDs, parent links, tasks, statuses, results/errors, recent progress, budgets and
worktree references remain available. Archival is a storage state, not a successful
outcome. Full transcripts and the runtime's rolling event stream are not included.

Finished non-worktree leaves archive even while their parent is active, preserving
the original parent link. Active agents and ancestors required by retained children
stay in the working set. Finished worktree records remain retained while ancestors
are active so commit and integration can proceed; they follow normal archival once
that tree settles. Archived records are omitted from the normal `list` and TUI
working tree, and the retained set has no fixed record-count capacity. A failed
archive write leaves evidence retained and reports a storage error; repair storage
to allow archival to resume. No worktree, branch, todo, or session is deleted.

The model can discover old records and retrieve them without running them:

```json
{"action":"archive","limit":20}
{"action":"archive","limit":20,"after":"UUID_FROM_NEXT_AFTER"}
{"action":"status","id":"ORIGINAL_AGENT_UUID"}
{"action":"wait","id":"ORIGINAL_AGENT_UUID"}
```

`archive` returns `agents` (bounded name/task previews, IDs, parent IDs, status and
archive timestamp) and `next_after` (null at the end). `limit` defaults to 20 and
must be 1–100. Pages sort by UUID, not completion time; restart a scan to discover
records added before its cursor. `status` adds `archived: true` for retired records
and returns the full retained record. `wait`/`wait_many` return stored outcomes
immediately. These lookups work in read-only mode and after restarting Helm.

Archives are historical evidence: `follow_up` and worktree mutations require a
retained record. To do new work based on an archived result, use a fresh `spawn`
with its original ID and relevant findings in the task; normal current authority
and budgets apply. Read-only worktree inspection may report a path that no longer
exists. Archived references do not promise that an OS process or worktree survived.

Files live beside each workspace tree as `subagents/<workspace-key>.archive/<id>.json`.
Each version-1 envelope contains `archived_at` and the original `record`. The archive
is written and synced before retirement from the working tree; retrying a partial
transaction preserves the existing identical record. Unix directories/files use
0700/0600 permissions. Reads and writes reject records above 16 MiB rather than
silently truncating evidence. Corrupt or conflicting records fail explicitly.
Lookup reads one record; listing scans filenames while retaining only one page of
IDs in memory. Storage is local to the workspace, with no automatic expiry or
archive deletion. Disk use grows with retained history. Library runtimes created
without a store retain the same archive in memory only.

Back up both the workspace tree and its sibling archive directory. Older binaries
ignore the new archive directory and cannot discover its records; preserve it when
rolling back. Records deleted by older pruning cannot be reconstructed. As with the
existing working-tree store, use one writing Helm runtime per workspace; the store's
in-process mutation lock is not multi-process session coordination (#78).
