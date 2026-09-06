# Direct interactive terminal attachment

Helm has two intentionally separate input planes. Agent prompts enter the model and
session transcript. Attached-terminal input goes directly to one selected PTY and
must never enter prompts, saved sessions, approvals, tracing, audit events, or logs.

## Full-screen workflow

Press `Ctrl+T` to list all terminals, use the arrow keys to select one, and press
`Enter` to attach. Keyboard input, bracketed paste, control characters, navigation
keys, function keys, and resize events are forwarded to that terminal. This supports
interactive shells, SSH, `sudo`, REPLs, pagers, and alternate-screen programs.

Detach with **Ctrl+T** (or **Ctrl+]**). Detaching, opening another view, or switching sessions never
terminates the terminal or its child process. Termination is a separate, explicit
runtime operation. Saved-voyage navigation stays in the same Helm process. Voyages
in the same canonical workspace share its terminal manager; another workspace has
its own manager and cannot list or write the first workspace's terminals. Returning
to a workspace restores its live processes and shell state. Helm acquires the target
voyage's execution lock and builds its authorized runtime before switching; a busy
or invalid target leaves the current voyage and terminals available. Policy freshness
is checked again before dispatching a run from a retained runtime.

Quitting Helm or handing off to a configuration command drains every retained
workspace, including inactive terminals. This retention applies only within a live
Helm process, not across restarts. The header always displays the detach chord while attached.
If an agent requests approval while a terminal is attached, Helm returns
`unavailable` immediately; terminal keystrokes can never approve an agent action.

## Plain chat workflow

In plain chat, use `/terminals` to list this workspace's live terminals, then
`/terminal ID_OR_EXACT_NAME` to attach. An ambiguous name requires the listed ID.
Listing an empty workspace does not construct a provider or launch a shell. Saved
terminal metadata cannot reattach a process after Helm restarts.

Attachment requires local TTY input and output and current writable local policy.
The private screen reserves its first row for the detach hint. Ctrl+C goes to the
inner process; Ctrl+T or Ctrl+] returns to Helm without terminating it. Bytes after
the detach chord belong to the next Helm prompt, including a partial UTF-8 prompt
that you finish typing after detach. No private input before the chord enters that
prompt. Pending input is bounded to 64 KiB and never submitted on overflow or invalid
UTF-8. Ctrl+U clears an unsubmitted draft; Backspace and left/right navigation edit
it. An inner process exit or unavailable viewport also returns to Helm. Unread
input is discarded on an unexpected attachment exit because it still belongs to
the private PTY. Only an explicit detach chord transfers queued input to Helm.
If a private write fails after that chord was received, Helm retains the suffix
and reports that preceding private input delivery may be incomplete.

The plain frontend uses the current workspace manager. `/new` creates another
voyage in that same workspace and preserves its terminals. It does not recover
another workspace's processes or turn serialized IDs into live attachment authority.
Quitting Helm requests bounded terminal cleanup; an unobserved cleanup is reported
explicitly. During attachment, plain approval requests return unavailable and
asynchronous frontend notifications do not write over the private screen.

This attaches to a PTY owned by the current Helm. Running SSH inside that PTY
does not attach the interface to a remote Helm or select a coordinating Helm.
Those are separate planned [remote voyage interface](voyages.md) operations. Detaching
this view preserves the live local process; exiting its owning Helm cannot promise
that the process survives.

## Privacy after human attachment

Attaching makes that terminal private for the remainder of its lifetime, before
the first human keystroke. Helm discards its unread model capture and suppresses
all later captured output, including terminal echo, application repeats and output
arriving after detach. The human screen keeps updating. Detach, reattach and voyage
navigation do not resume model capture. Direct human writes through the runtime
adapter establish the same boundary even without a prior attach call.

Model reads report that capture is unavailable; model writes to the private terminal
are denied. Pending model input stops its remaining suffix when privacy is activated;
bytes already accepted by the PTY cannot be recalled. Model cancellation, termination
and cleanup operations retain their existing authority. Unattached terminals keep
normal model input/output. Start a new terminal when the model needs to observe and
interact with terminal output again.

Output already disclosed before attachment cannot be retracted, and Helm does not
rewrite earlier session history. This is a capture boundary inside Helm, not an OS
sandbox or a restriction on unrelated filesystem tools. Human screen snapshots must
never be copied into model messages, sessions or diagnostics.

## Runtime integration contract

The built-in runtime uses `vt100` to expose an emulated cell grid, SGR attributes,
cursor position, wrapping, erase operations, and alternate-screen behavior instead
of raw escape sequences. Agent unread transcript retention is bounded separately;
when history is evicted, `TerminalSnapshot.dropped_unread_bytes` increases and the
TUI displays the gap. Privacy omissions are recorded separately in
`TerminalSnapshot.privacy`: unread bytes discarded at attachment and subsequent
output bytes suppressed. The attached header and model-read result identify private
capture instead of presenting withheld output as an empty, complete transcript.
Emulated human screen state and its revision continue updating.

The UI consumes the `InteractiveTerminals` trait in `helm::terminal`; it never owns a
singleton process. One manager can expose any number of local, SSH, container, or
privilege-elevated terminals.

The built-in PTY writer uses one dedicated worker per terminal. Input requests are
limited to 64 KiB, with at most eight queued requests per terminal and a 250 ms
delivery deadline. A full queue or oversized request is rejected without enqueueing
bytes. Cancellation or a delivery timeout stops the remaining suffix; bytes already
accepted by the PTY cannot be rolled back. An incomplete or unconfirmed result must
not be retried automatically. Cursor-report replies use the same bounded queue and
never block the output reader. The process map is released before awaiting input,
so a non-reading child cannot block another terminal's list, resize, or shutdown.
Cleanup observes the input worker as well as the child and output reader.

Runtime implementations must:

- keep terminal IDs stable for the life of each terminal;
- provide a current emulated screen as styled `TerminalCell` rows, including cursor,
  title, state, and monotonically increasing revision;
- parse ANSI/VT output before publishing snapshots so cursor movement, erasure,
  colors, alternate screens, and full-screen apps render correctly;
- establish the permanent privacy boundary through `InteractiveTerminals::attach`
  before exposing the human screen or accepting direct human input;
- accept input as opaque bytes without UTF-8 assumptions or instrumentation;
- accept dimensions as columns then rows and signal the underlying PTY promptly;
- publish coalescible added/changed/removed events without blocking PTY readers;
- distinguish detach from terminate and retain output/state while no viewer exists;
- bound retained scrollback and event queues;
- never include direct input/output in model, session, approval, tracing, audit, or
  error payloads. Operational events may contain terminal ID and byte counts only.

`NoInteractiveTerminals` keeps Helm usable before a manager is installed and makes an
empty picker explicit. Deterministic UI tests use fake multi-terminal implementations;
the production adapter should add PTY tests for shell echo, resize, SSH, pager and
alternate-screen behavior.
