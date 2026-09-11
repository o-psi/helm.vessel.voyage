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
This is a **decoded-event/wire admission bound**, not a bound on all outer-input
memory. Crossterm 0.29 retains an unterminated bracketed paste in its internal
parser before yielding `Event::Paste`; it exposes neither a size limit nor a reset
for that buffer. Bounded pre-decode input remains a real #32 implementation gap.
No allocation-stress result or total input-memory bound is claimed. Closing this
gap requires a bounded reader/parser boundary, not a larger post-decode check.
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
Actual commands, results and measurements for this delivery follow below. Native macOS/Windows, deployed WSS networks, real SSH/sudo accounts,
terminal multiplexers and live-provider certification are not inferred from local
Linux/offline checks. Human testing/product acceptance is not required for this
resolution pass.


### Verified Linux delivery source: `7fd619f`

The measured source includes the current main/#14/account/approval integration and
the narrow post-attachment `CleanupFailure` UI hook; workflow/operator hooks remain
in place. Native providers in the fixture are explicit anonymous loopback
`openai-chat`, not an implicit ChatGPT account. Fixture directories have short
private paths to stay within native Unix socket path limits.

| Check | Observed result |
|---|---|
| Affected Helm/Voyage/Vessel/protocol `cargo check --locked` | Passed after correcting use of a private Ratatui writer API |
| Helm and Voyage library tests | 88 + 125 passed, including native panic restoration and saturated writer-cutoff interleavings |
| Protocol terminal tests | 2 passed; final workspace run includes them |
| Locked development Helm/Vessel/Voyage build | Passed |
| `python3 helm/tests/private_terminal.py --bin-dir "$CARGO_TARGET_DIR/debug"` | 13 assertion groups passed against real independently supervised processes |
| Final workspace default-feature coverage | 227 passed, 0 failed, 1 ignored live-provider test |
| Workspace formatting and diff checks | Passed |
| Strict affected-package all-target Clippy | **Failed** on inherited account/provider/UI warnings; completed warning-mode analysis reports no new terminal-code diagnostics |

The executable journey observes F3 discovery/Enter attachment and the standalone
plain-user CLI route; bold/RGB cells and cursor visibility/position; application
and normal arrows/paste; resize; synthetic private echo and delayed repeats;
Ctrl+] queued-tail handling; direct model-read privacy gaps and model-write denial;
live read-only policy refusal; SIGTERM/SIGHUP and socket-loss restoration; same
independent owner/child survival across Vessel restart; and incomplete private
paste on child exit stopping the TUI instead of resuming its composer. It observes
a nonempty live root-terminal obligation, then `cleanup.phase = observed`, empty
pending/resources, child exit and Voyage suspension. Canary absence is checked in
provider requests, canonical snapshots/state and diagnostics. The small outer CSI
probe is not a full terminal emulator. The runtime panic test uses a separate real
PTY subprocess and checks termios plus alternate-screen/paste/cursor transitions.

Some runtime terminal refusals conservatively carry `outcome_unknown`; the fixture
records that rather than labelling them durable definite receipts. No operation is
replayed. The intentionally disconnected one-shot run follower exits with a
connection error while the independent accepted run finishes; this is not hidden
as successful continuous following.

Initial compiler and fixture failures remain in ignored local evidence: private
Ratatui writer access; an obsolete nested wire-envelope assumption; an overly
strict definite-refusal assertion; and expecting the deliberately disconnected run
follower to exit successfully. They were corrected without increasing timeouts.
The protocol's unused-unit lint was also fixed. Passing earlier subsets are not
substituted for the final expanded journey.

Coverage is published in [the compact measurement](../coverage/latest.json).
The default shared-target report contained stale objects and missing historical
source paths and is preserved **but not used** for totals. The corrected report
retains all 11 current workspace executables (10 executed test binaries), validates
406 source paths, and finds no duplicate logical paths, foreign reused source or
mismatched functions. Source bytes remained unchanged during measurement. Detailed
HTML/JSON/logs, copied instrumented executables/profdata and binary hashes remain
under ignored `target/coverage-report/issue32*`; private synthetic PTY evidence is
retained separately. No test diagnostics or machine credentials are published.

#32 remains open specifically for bounded pre-decode outer input. Human product
signoff is not a blocker. Native platforms, actual SSH/sudo accounts, deployed WSS
and terminal-multiplexer certification remain unclaimed limitations, not fabricated
passing checks.


### Observed costs (Linux development build, one sample per size)

`cargo test -p helm --locked --lib measured_bounded_screen_cost -- --nocapture`
measures the plain default-cell allocation/render path. It is not a release-build
benchmark, worst-case Unicode/style bound, total heap measurement or throughput
promise. Cell-header bytes exclude text allocations and vector/allocator overhead.

| Child viewport | Typed JSON bytes | Cell-header bytes + text | Serialize / decode | Buffer allocation / render |
|---|---:|---:|---:|---:|
| 80 × 24 | 44,328 | 76,800 + text | 1.687 / 1.815 ms | 0.079 / 0.746 ms |
| 240 × 120 | 662,762 | 1,152,000 + text | 24.194 / 24.998 ms | 0.868 / 10.582 ms |

The final real loopback private-snapshot measurements (including runtime snapshot,
serialization and HTTP response handling, not just rendering) were:
- 80 × 24: 17.385 ms; 46,664 bytes when reserializing the returned session reply.
- 240 × 120: 268.509 ms; 692,266 bytes when reserializing the returned session reply.

These are full snapshots, not deltas. A 240 × 120 snapshot can take longer than the
200 ms observation schedule; missed ticks are skipped, and five Hz is a maximum
scheduled rate rather than a guaranteed rate. Dense styles/long cells may hit the
1 MiB typed-screen limit earlier. The pre-decode outer-paste memory gap described
above is not covered by these bounded screen measurements.
