# Worktree cleanup audit

Tracking: [#46](https://github.com/o-psi/voyage/issues/46), with implementation
follow-ups [#67](https://github.com/o-psi/voyage/issues/67) and
[#70](https://github.com/o-psi/voyage/issues/70).

The starting primary workspace and GitHub main matched `9b6d0d0`. The inventory
contained 77 worktree registrations (including the primary), 15 missing paths,
and 38 local branch tips with commits outside main history. Every original branch
and detached head was preserved in a verified all-refs bundle before changes.
The browser-console tracked patch and untracked module were also backed up.

Dedicated review agents inspected all 15 existing worktrees with unmerged commits,
plus the retained steering branch from a missing checkout. Exact ancestry established
that 46 other existing secondary worktrees were already merged and clean. Fourteen
other missing registrations were ancestors or had entirely patch-equivalent commits.
Additional local branch-only tips were compared by patch identity, identical trees,
merged successor commits and current implementation. An unmerged commit ID alone
was not treated as missing functionality.

## Integration decisions

- Private policy defaults: integrated actual `feat/policy-defaults` ancestry from
  PR #131. The operator-private global/project defaults, exact workspace activation,
  freshness and escalation checks extend named profiles. TUI switching remains open.
- Workflow secrets: integrated actual `feat/workflow-secrets` ancestry from PR #130.
  Explicit transient bindings reach policy-controlled one-shot shell environments;
  output is suppressed before capture. Plain-mode missing-input prompting remains open.
- Added a combined real-provider-fixture regression: restrictive defaults deny private
  effects, explicit activation permits exact secret consumption, and a changed defaults
  revision after model dispatch prevents release of a new private environment.
- Browser console: archived as an incomplete deferred experiment. Its one committed
  test depends on an untracked module and has no production route/frontend integration.
  Six state tests pass with the preserved module; this is not a delivered UI. Helm
  remains the requested operator interface for local and remote Vessel management.
- Other reviewed work: incorporated, patch-equivalent, or superseded by safer current
  implementations. No old task routes, weaker persistence/authority checks or deliberate
  failing-regression snapshots were restored.

## Original unmerged branch dispositions

| Branch | Disposition |
| --- | --- |
| `agents/agents-loader-7aba3587ba1d4a3fb86805c09c359fb3` | Archived: incorporated or superseded by reviewed main code |
| `agents/inline-question-docs-4008d93ee2a649fc944987922ee10f2b` | Archived: incorporated or superseded by reviewed main code |
| `agents/local-session-ownership-a0d302ab6b3c4399a54d6ef62d106316` | Archived: incorporated or superseded by reviewed main code |
| `backup/local-features-before-rebase` | Archived: incorporated or superseded by reviewed main code |
| `feat/completion-runtime` | Archived: incorporated or superseded by reviewed main code |
| `feat/completion-seal` | Archived: incorporated or superseded by reviewed main code |
| `feat/completion-wiring` | Archived: incorporated or superseded by reviewed main code |
| `feat/managed-shell` | Archived: incorporated or superseded by reviewed main code |
| `feat/observable-terminal-shutdown` | Archived: incorporated or superseded by reviewed main code |
| `feat/policy-defaults` | Integrated with full ancestry |
| `feat/vessel-attachment-transport` | Archived: incorporated or superseded by reviewed main code |
| `feat/vessel-session-console` | Archived: incomplete deferred browser experiment |
| `feat/workflow-secrets` | Integrated with full ancestry |
| `feature/completion-outcome-ui` | Archived: incorporated or superseded by reviewed main code |
| `fix/checkpoint-cancellation` | Archived: incorporated or superseded by reviewed main code |
| `fix/completion-handoff` | Archived: incorporated or superseded by reviewed main code |
| `fix/completion-shutdown-windows` | Archived: incorporated or superseded by reviewed main code |
| `fix/context-recovery-64` | Archived: incorporated or superseded by reviewed main code |
| `fix/context-window-fixture` | Archived: incorporated or superseded by reviewed main code |
| `fix/gate-cleanup-observation` | Archived: incorporated or superseded by reviewed main code |
| `fix/gate-subagent-fixtures` | Archived: incorporated or superseded by reviewed main code |
| `fix/root-final-gate-82` | Archived: incorporated or superseded by reviewed main code |
| `fix/session-checkpoint` | Archived: incorporated or superseded by reviewed main code |
| `fix/steering-86` | Archived: incorporated or superseded by reviewed main code |
| `fix/storage-portability` | Archived: incorporated or superseded by reviewed main code |
| `fix/tool-output-resize-fixture` | Archived: incorporated or superseded by reviewed main code |
| `fix/tui-checkpoint` | Archived: incorporated or superseded by reviewed main code |
| `test/attachment-client-sockets` | Archived: incorporated or superseded by reviewed main code |
| `test/completion-gate` | Archived: incorporated or superseded by reviewed main code |
| `test/completion-runtime-e2e` | Archived: incorporated or superseded by reviewed main code |
| `test/journal-commit-busy` | Archived: incorporated or superseded by reviewed main code |
| `test/rebuilt-completion-gate` | Archived: incorporated or superseded by reviewed main code |
| `title-generation-core` | Archived: incorporated or superseded by reviewed main code |
| `title-generation-system` | Archived: incorporated or superseded by reviewed main code |
| `work/helm-runtime` | Archived: incorporated or superseded by reviewed main code |
| `work/helm-serve` | Archived: incorporated or superseded by reviewed main code |
| `work/helm-ui` | Archived: incorporated or superseded by reviewed main code |
| `work/vessel` | Archived: incorporated or superseded by reviewed main code |

## Preservation and verification

The local evidence directory is `.local-git/evidence/worktree-cleanup-20260905/`.
It holds the original inventories, verified `all-refs-before.bundle`, per-worktree
reviews, patch-equivalence mappings, dirty browser patch/untracked archive and
combined verification logs. Branch tips are retained under
`refs/archive/worktree-cleanup-20260905/heads/`; detached heads have separate archive
refs. Retired worktree release artifacts and build directories are relocated into
that evidence directory rather than discarded. The final cleanup manifest records
original paths and preservation destinations.

To inspect preserved branch names:

```sh
./scripts/local-git for-each-ref refs/archive/worktree-cleanup-20260905/
```

To restore an archived branch under a new name, use normal Git, for example:

```sh
./scripts/local-git branch review-console refs/archive/worktree-cleanup-20260905/heads/feat/vessel-session-console
```

The console's uncommitted files remain in its separate dirty patch and untracked
archive; restoring the branch alone does not restore those files.

Combined Linux validation and final publication/installation status are recorded
below after observation. No live-provider, macOS or Windows validation is implied.

## Verified result

The combined runtime at `f41d7a4` passed all 939 workspace tests both normally and
under actual two-CPU affinity (zero failures or ignored tests), strict formatting
and Clippy, a locked optimized workspace build, all 28 system fixtures, and 12
evaluation definitions. The new defaults/secret-binding interaction case passed.
The fixed-path Linux policy-ceiling fixture ran and passed. Unique package
`voyage-cleanup-f41d7a4-20260905-linux-x86_64.tar.gz` passed its SHA-256 check.
These checks use offline provider fixtures; they are not live-model evaluations.

Cleanup retired 63 existing secondary worktrees (including the two temporary
integration checkouts), pruned 15 missing registrations, and archived/retired 100
local branch names. The primary workspace and its `main` branch remain. Original
release artifacts and ignored build directories were preserved by relocation;
the browser experiment's dirty patch and module remain recoverable separately.
