# Contextual answer actions and changed-file navigation

Continuation of [#272](https://github.com/o-psi/voyage/issues/272), after the
[main-UI finish-up](ux-272-closure-readiness.md). This implements a specific
remaining result-inspection journey, not the complete redesign.

## Interaction

Saved, non-interrupted assistant answers have **Copy** and **Workspace changes**
actions in the transcript. Click the relevant action, or use **Alt+M** to focus
successive visible answer actions, **Alt+C** to copy the selected answer and
**Alt+D** to inspect workspace changes. Keyboard selection is highlighted and
retains the selected answer through status/footer reflow. Copy opens the existing
explicit clipboard-disclosure review and reads the full selected canonical message,
not simply the newest reply. Tool/provisional/interrupted output does not acquire
answer actions. Selection/revision/incarnation changes refuse stale activation.

Workspace changes means the **current executing workspace**, including unrelated
and pre-existing edits. It is not a claim that the selected answer caused those
changes. The inspector exposes separate status, staged, unstaged, untracked,
directory and file scopes. After a successful status read, changed paths become
selectable: **Tab** focuses the file list, arrows select, **Enter** or a click reads
the file, **u** shows its unstaged diff, **s** its staged diff and **a** reloads all
changed paths. Escape returns to the retained conversation/draft.

The inspector calls the executing Voyage's advertised tools through the normal
durable operator path. It never reads a remote path locally. Selected Git paths
are shell-quoted and Git uses literal pathspecs; options, substitutions, globs and
pathspec magic cannot become instructions. Status uses porcelain v1 with rename
detection disabled so each row names one path. Git C-quoted UTF-8 filenames decode;
control characters, traversal, absolute/platform paths and invalid UTF-8 are not
made clickable. Up to 256 valid changed paths are exposed from bounded successful
shell output; omitted/unrepresentable paths remain in textual diagnostics, not
invented navigation. Deleted/unreadable/binary files can refuse text reads; refusal
is displayed, not treated as an empty result. Native file/clipboard behavior is not
established by Linux tests.

## Integration fix discovered by the real journey

An idle operator command can legitimately create a fresh owner incarnation. The
inspector previously closed when that incarnation differed from its preflight
observation, despite having an accepted exact command/run receipt. Result tracking
now follows that exact admitted run and obtains a fresh observation for the final
read. Unrelated runs do not supply results, stale copy/read jobs still refuse, and
no command is replayed. Runtime/privacy/permission boundaries are unchanged.

## Verification

- 181 Helm library tests passed. New tests cover selected historical answer versus
  latest, stale revision, interrupted-output exclusion, keyboard selection across
  status reflow, safe Git quoting/decoding, literal-path injection resistance,
  selectable file actions, stale pointer geometry and exact admitted-run result
  tracking across idle owner replacement.
- `helm/tests/result_actions.py` ran against actual Helm/Vessel/Voyage and a
  synthetic loopback provider: selected canonical clipboard bytes, changed filename
  containing a space, keyboard and mouse file reads, selected-only unstaged diff,
  unchanged workspace contents and no additional provider request. Fixture cleanup
  was observed. Evidence: ignored `target/results-ui/pty-mouse2/`.
- The main-workspace PTY and existing inspection, historical and composer journey
  cases passed with their documented partial coverage. Earlier development failures
  remain under `target/results-ui/`: a completed-run identity mismatch blocked Copy,
  and idle owner replacement closed inspection. Neither was presented as passing;
  the final paths passed after fixes.
- Formatting/Python syntax/diff checks apply. Strict Clippy continues to report
  the pre-existing 20 Helm warnings; no new warning in the answer-action or file
  navigation implementation was reported. Not a clean Clippy pass.
- Workspace coverage passed **484 tests, 0 failed, 1 ignored**, with lines
  **39.0161%**, functions **37.1537%**, regions **37.9009%** (all increases).
  Measured after final source/test edits and published in
  `coverage/latest.json`, selecting the complete current Cargo executable set.
  Rust coverage does not replace provider/browser/native/competitor acceptance.

## Remaining scope

Contextual program/browser/task cards and full visual continuity remain distinct
from this answer/file journey. #274's intermittent browser/image diagnosis and
#273/#255's residual acceptance remain open. #272 is not marked fully complete
because these actions are implemented. Managed installation and normal-command
verification are recorded separately after actual completion; an already-running
Helm client still needs reopening to load a new executable.

## Observed managed delivery

Installed release `9cf87cc5d0f387aca90f12803aa069ae687b6875ff32623abd87c7557c7cd16c`
completed with exit 0. Normal `helm` resolves there and matches tested client SHA-256
`db083ddd28a3cacdc8546fdb0184132ac2829215a9e8416085d97543d3eb0b89`.
The installed binaries passed the full result-action PTY journey with observed
fixture cleanup. Supervisor active/running and installer authenticated readiness/
retained-owner checks completed. Journal pending is null; previous release retained.
Reopen Helm normally to load it. Evidence: `target/results-ui/install-result.txt`
and `target/results-ui/installed-pty/` (ignored).
