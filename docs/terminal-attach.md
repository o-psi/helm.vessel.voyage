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
runtime operation. The header always displays the detach chord while attached.
If an agent requests approval while a terminal is attached, Helm returns
`unavailable` immediately; terminal keystrokes can never approve an agent action.

For screen readers or terminals which cannot run Helm's full-screen UI, the runtime
adapter should expose a raw/plain attach command using `PlainDetachFilter`. It copies
PTY bytes directly between stdio and the selected terminal, reserves Ctrl+T and Ctrl+] locally,
and restores the outer terminal on exit or signal.

## Runtime integration contract

The built-in runtime uses `vt100` to expose an emulated cell grid, SGR attributes,
cursor position, wrapping, erase operations, and alternate-screen behavior instead
of raw escape sequences. Agent unread transcript retention is bounded separately;
when history is evicted, `TerminalSnapshot.dropped_unread_bytes` increases and the
TUI displays the gap. Emulated screen state remains intact.

The UI consumes the `InteractiveTerminals` trait in `helm::terminal`; it never owns a
singleton process. One manager can expose any number of local, SSH, container, or
privilege-elevated terminals.

Runtime implementations must:

- keep terminal IDs stable for the life of each terminal;
- provide a current emulated screen as styled `TerminalCell` rows, including cursor,
  title, state, and monotonically increasing revision;
- parse ANSI/VT output before publishing snapshots so cursor movement, erasure,
  colors, alternate screens, and full-screen apps render correctly;
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
