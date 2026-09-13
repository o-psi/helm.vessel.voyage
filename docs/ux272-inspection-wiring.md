# Coding-result inspection

Tracked in [#272](https://github.com/o-psi/voyage/issues/272). This describes the
integrated source; executable evidence and limitations are recorded in the
[delivery ledger](audits/ux-272-delivery.md).

## Inspect without changing the workspace

Use `/inspect` or `/diff`, or search for those actions with F8. The panel names
the executing host, voyage and workspace. Up/Down chooses a scope; Enter submits
one explicit runtime operator request. Esc closes observation, not the voyage.
PageUp/PageDown reads long output. `p` edits a workspace-relative UTF-8 path for
the file and directory scopes. `c` opens the separate response-copy review.

Scopes are deliberately separate:

- Status: index/worktree changes and ordinary untracked paths.
- Unstaged: worktree against index.
- Staged: index against HEAD.
- Untracked: paths, not file-content diffs.
- Directory/file: existing executing-voyage filesystem tools and authorized roots.

Git diff includes unrelated existing changes; it is not a last-turn attribution.
Ignored files and untracked contents are excluded from diffs, binary files show
markers rather than binary patches, and submodules use short summaries. External
diff/textconv and filesystem-monitor hooks are disabled. No rollback, write,
filesystem copy or Helm-side remote-workspace fallback is provided. Missing Git,
missing repository, denied policy or absent output never means a clean tree.

The live tool inventory must advertise the requested tool. Reads run through the
same durable operator admission, executing-host policy, approvals, cancellation
and output limits as other tools. The panel binds its result to the exact command
receipt, admitted run and that run's canonical message range. It never picks an
unrelated latest tool result. Unknown admission retains the original command
identity; observation timeout does not replay or cancel the command.

Inventory, copy and result reads are bounded and fenced by route generation,
voyage incarnation and revision. Closing/switching cancels owned observation tasks
and retains their join handles for observed cleanup. A stale panel requires an
explicit reopen rather than silently targeting newer state.

## Copy canonical text

`/copy` selects the newest complete, nonempty assistant response in the loaded
snapshot, excluding interrupted responses and tool-role output. The review names
host, voyage, message and revision. Enter explicitly authorizes disclosure to
this terminal's clipboard; Esc cancels. Other local applications may read that
clipboard. The complete revision-bound public canonical message is fetched before
copy; terminal styling and projected/truncated transcript text are not copied.

The bounded reader rejects malformed UTF-8-byte continuations and changing
revision/identity. OSC52 encodes canonical bytes as base64 so content cannot become
terminal instructions. A matching fresh explicit request is rechecked immediately
before writing. The UI says **Copy requested**, not clipboard verified: OSC52 has
no portable success acknowledgment. `/export PATH` is a separate file fallback.

## Limits

- File tools may render binary/non-UTF text lossily. Git quotes non-UTF filenames;
  its quoted spelling is not an exact UTF-8 navigation path.
- Results and clipboard responses are bounded; use canonical history/export when
  display limits are exceeded. A withheld or incomplete result is not success.
- No live clipboard acknowledgment, remote production-host, native macOS/Windows
  or paid-provider certification follows from the focused Linux tests.

Implementation: `helm/src/process_client/ui/inspection.rs`,
`inspection_bridge.rs`, and `helm/src/process_client/export.rs`. Focused tests
cover dirty Git scopes/binary markers, registry refusal, exact admitted result
association, stale-copy fencing, canonical encoding and owned read cleanup.
