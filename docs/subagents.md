# Parallel subagents

Helm subagents are bounded child workers owned by a parent agent. They are useful
when several independent tasks can proceed at once; they are not background
terminals and do not silently inherit unlimited authority.

## Lifecycle

Every child has a stable ID, parent ID, task, status, timestamps, policy, budget,
optional worktree, progress history, and final result. A child moves through
queued, running, and one terminal state: completed, failed, cancelled, timed out,
or interrupted. A restarted Helm marks work it cannot reattach as interrupted
instead of pretending it is still running.

The parent can start a child, inspect the tree, wait for one or more children,
send a message, assign a follow-up task after completion, or cancel outstanding
work. Results are attributed to their child and returned to the parent in a
bounded, structured form. Messages and lifecycle events carry monotonically
increasing sequence numbers so consumers can recover without guessing order.

## Concurrency and authority

The configured concurrency limit bounds actively running children; excess work
waits in a visible queue. Each child receives explicit token, wall
time, output, and tool limits. Child permissions are no broader than the
intersection of its requested policy and its parent policy. An unattended child
cannot answer an approval prompt and cannot use the TUI keyboard as approval.

Cancellation is cooperative first and forced only for Helm-owned execution after
the grace period. Cancelling a parent cancels its unfinished descendants. A child
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

Release validation covers bounded parallel execution, result synthesis, messaging,
cancellation, timeouts, policy denial, conflicting edits, child failure, restart
recovery, and non-Git general work. The parent conversation must remain responsive
through every scenario.

## Implementation boundaries

`AgentTreeStore` persists records through a synced temporary file and atomic rename.
The runtime calls `recover_after_restart` once during startup. `AgentPolicy` validates
that child roots, tools, approvals, and budgets only narrow parent authority.
`WorktreeManager::conflicts` identifies files changed independently by sibling
branches before guarded integration. Clean worktrees may be removed, but their
branches and commits remain recoverable.

Model/tool loops have no turn-count cutoff. Legacy `max_turns` fields in saved
child budgets are ignored on load and omitted on the next save; token, wall-time,
child-count, terminal, and authority limits remain unchanged.

Older Helm binaries require the removed field and cannot directly load newly saved
child trees. Before upgrading, retain a backup of the data directory if rollback
is needed; do not overwrite current task history with that backup implicitly.

## Automatic archive

Finished subagents archive automatically after completion, failure, cancellation,
or timeout. Helm also archives recovered terminal records on startup. Original
IDs, parent links, tasks, statuses, results/errors, recent progress, budgets and
worktree references remain available. Archival is a storage state, not a successful
outcome. Full transcripts and the runtime's rolling event stream are not included.

A finished record needed by an active parent/child tree stays in the working set
until that tree settles. Other archived records consume no `subagent_max_agents`
slots and are omitted from the normal `list` and TUI working tree. A failed archive
write leaves the record retained, logs an error, and can block spawning at capacity
until storage is repaired. No worktree, branch, todo, or session is deleted.

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
