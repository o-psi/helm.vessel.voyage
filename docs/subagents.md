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
waits in a visible queue. Each child receives explicit model-turn, token, wall
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
