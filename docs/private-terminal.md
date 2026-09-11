# Private terminal fidelity

Helm renders an emulated terminal owned by the independent **Voyage** process.
Vessel authenticates and routes the human-only operations. Neither Helm nor
Vessel starts a substitute PTY. These operations are not conversation messages,
journalled commands, or effects that reconnect may replay.

## Access, including plain users

In the TUI, press **F3**, choose a running named program, then Enter. **Ctrl+]**
returns to the saved conversation draft; Ctrl+C goes to the child. Direct input
makes that terminal permanently private: unread model capture is discarded, later
output (including echo/repeats after detach) remains unavailable to the model, and
model writes are refused. Already disclosed output cannot be retracted. Start a
new terminal for model-observed work. This boundary is not an OS sandbox.

The supported plain-user route is a dedicated TTY attachment, not historical
inline `/terminal` commands in plain chat:

```sh
helm connect list
helm connect request SESSION_UUID '{"op":"controls","run_id":null,"section":"terminals"}'
helm connect terminal SESSION_UUID --run RUN_UUID --terminal TERMINAL_UUID
```

Use the inventory's `run_id` and the running entry's `id`. Local commands use the
local Vessel; place `--directory PATH` or the existing scoped `--access-file PATH`
option after `connect` to select another authorized route. The terminal command
requires exactly one selected Vessel and real input/output TTYs. Listing metadata
does not attach, execute a tool or discover providers. The command validates the
current process incarnation and the runtime validates the exact owning run and
terminal; saved metadata never revives a process. Plain chat does not implement
`/terminals` or `/terminal`; that is an explicit scope decision, not absent access.

Detaching does not cancel the run or child. Root PTYs close before the Voyage
cleanly suspends; they are not promised across turns or restarts. The local UI has
one event owner during attachment. Multiple independently authorized human clients
are **not** given an exclusive attachment-client lease: input is serialized through
the runtime's bounded writer, under the same run/incarnation and policy fences.
An attachment is a permanent model-input cutoff, not an exclusive human-client
lock. Do not use simultaneous human clients when exclusive keyboard control is
required.

## Screen and input contract

A version-1 structured `screen` accompanies the legacy monochrome `rows` in the
private response. Cells carry default/indexed/RGB foreground and background,
bold/dim/italic/underline/reverse, explicit width (1/2) and an empty width-0 wide
continuation. Voyage's existing vt100 parser supplies the screen; it does not
supply dim attributes, which remain false. Cursor visibility and position are
separate; only Ratatui's backend cursor is rendered. Unsupported cursor shapes
are not fabricated. Combining text stays in its leading cell. Helm refuses unsafe
control/bidi cells and substitutes a visible replacement when its grapheme/width
table disagrees with the peer, retaining coordinates instead of shifting cells.
Actual font and emoji rendering still depend on the outer terminal.

Helm checks terminal/run identity, monotonic revision and requested dimensions.
Older-revision and pre-resize structured screens are ignored. Legacy row-only peers
retain sanitized monochrome output and conventional input modes; they do not gain
styling or application mode fidelity. Permanent privacy and exact incarnation
validation remain required. Final passive `HELM_COLOR` adaptation still applies.

Supported input includes Unicode, Enter, Backspace, Tab/BackTab, Escape, ASCII
control combinations, arrows/Home/End, Insert/Delete/PageUp/PageDown and F1–F12.
Navigation/function modifiers use xterm Shift/Alt/Ctrl parameters. Application
cursor mode changes unmodified navigation to SS3. Application keypad sequences
are used only when Crossterm identifies a key as keypad input; ordinary terminals
that do not distinguish keypad keys retain their ordinary characters. No guessing
based on numeric characters. Mouse/focus forwarding, Kitty keyboard negotiation,
F13+ and arbitrary terminal extensions are not implemented.

Child-enabled bracketed paste receives one framed operation with `ESC[200~` and
`ESC[201~`; disabled children receive plain pasted bytes. Escape characters inside
bracketed paste are refused rather than permitting injected closing delimiters.
The frame limit is 64 KiB including delimiters (65,524 payload bytes when enabled).
Modes are the last observed child state; polling is not an atomic mode/input
transaction with the application. Ctrl+] is local, including Crossterm's legacy
Ctrl+5 alias and raw control representation. Ctrl+T is not a connected detach key.

## Bounds and failure handling

- Viewport: at most 240 columns × 120 child rows (four outer rows are chrome).
- Cell text: at most 64 UTF-8 bytes, with validated width/continuations and no
  executable control characters. Structured serialization: at most 1 MiB.
  Dense styles or long combining cells can reach the byte limit before the maximum
  viewport; the attachment refuses that screen rather than silently stripping it.
- Observation: at most five scheduled snapshots per second, one queued screen;
  no escape-stream bypass. Each full response includes the legacy rows. This is
  bounded full-frame polling, not a delta/compressed terminal transport.
- Helm input: 64 queued operations, each at most 64 KiB; adjacent writes coalesce
  only within the frame limit. Resize remains ordered with writes. Saturation
  stops attachment; queued/in-flight bytes may be partial and are not replayed.
- Voyage's input worker separately bounds queues, native write chunks and deadlines.
  Human attach waits for the model writer's cutoff before acknowledging it; failure
  to confirm quiescence refuses attachment, never reopens capture.
- On detach, error or child exit, Helm cancels and observes its writer/observer,
  drains complete queued events and native input, then restores raw mode, alternate
  screen, paste mode and cursor. Nothing is automatically submitted as a prompt.
  Crossterm does not expose a reset for a partially decoded paste/UTF-8 sequence:
  therefore a TUI attachment that ends **without an explicit detach chord** stops
  that Helm interface rather than resuming its composer with ambiguous input.
  This includes refused attachment, child exit, transport/error and signal paths.
  The dedicated terminal CLI exits and has no composer to expose buffered input.
  Explicit Ctrl+] authorizes the transition to Helm input; do not continue typing
  private data after it. Complete queued suffix events are discarded; an incomplete
  post-chord sequence may finish on later deliberate Helm input. No private input
  preceding the consumed chord is handed off. Unconfirmed native disposal or
  restoration is always fatal to that interface.
- Unix console writes have a 250 ms deadline per frame/restoration batch. Native Windows
  synchronous console-output boundedness is not established. Process kill/SIGKILL,
  terminal disappearance and OS failure cannot guarantee restoration. Panic guards
  attempt restoration but do not prove that an unavailable terminal accepted it.

## Rendering dependency decision

The published `tui-term` 0.3.4 manifest and Screen/Cell/cursor source were inspected.
It supports a custom adapter with defaults disabled; its default adapter adds
vt100 0.16.2, unlike Voyage's 0.15 parser. Its cursor overlay would require disabling
one cursor path. For this bounded protocol, direct Ratatui buffer cells retain the
required width checks, clipping and existing backend cursor with less adapter
machinery. No `tui-term`, second parser or experimental process controller is added.
This is a source/dependency decision, not a comparative performance claim.

## Evidence scope

Focused Rust checks cover encoding, styled/wide/combining cells, bounds and stale
observations; the Linux executable check is `helm/tests/private_terminal.py`.
Actual commands, results and measurements for this delivery are recorded below
when run. Native macOS/Windows, deployed WSS networks, real SSH/sudo accounts,
terminal multiplexers and live-provider certification are not inferred from local
Linux/offline checks. Human testing/product acceptance is not required for this
resolution pass.
