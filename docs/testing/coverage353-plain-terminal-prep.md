# Owned plain-terminal family preparation (#353)

This extends the ordinary family identified in
[the reachability map](coverage353-reachability-map.md). At audited source
`c15ef79b6568616fc7c0d24350ef3defdd97ee67`, `helm/src/plain_terminal.rs`
has 278 uncovered / 469 measured lines. Its pure helper tests qualify formatting,
selection, byte boundaries and cancellation wrappers; production `pending_prompt`
and `attach` loops have not executed in that report.

The source-only addition uses the current Helm library libtest executable as an
explicitly selected Linux child, attached to its own PTY and session. Each child
requires the actual parent PID/session witness, clears inherited environment,
uses fixture-only HOME/XDG/workspace and retains only the LLVM profile destination
plus required PATH/TERM and fixture markers. No human terminal, provider, Vessel,
voyage process, login, credential or production service is accessed.

## Acceptance and failure assertions

One parent test owns 22 bounded child scenarios:

| Actual production boundary | Assertions |
| --- | --- |
| Buffered prompt | CRLF consumes one separator; the next buffered line stays queued. |
| Raw prompt editing | Actual PTY UTF-8 bytes, left/home/end/delete/backspace and ignored control input produce the exact submitted text. No direct `PendingInput` mutation simulates these keys. |
| Prompt input budget | A full buffer refuses the additional byte and renders the limit state; actual Ctrl-U restores capacity, then submits one exact new line. |
| Prompt cancellation/EOF | Actual Ctrl-C retains unsent bytes; empty Ctrl-D returns without a line. |
| Human terminal detach | One exact terminal ID, exclusive attachment ownership, raw mode, alternate screen and the initial snapshot precede actual opaque input. The detach chord remains local; the suffix returns to Helm and only the preceding bytes reach the fixture manager once. |
| Failed delivery | A terminal write failure returns authored failure; failure coincident with detach keeps the suffix and marks `delivery_failed`, without replay. |
| Native resize | Actual kernel resize is observed as 40×11 content geometry; tiny 2×2 geometry refuses and detaches before additional direct input. |
| Cancellation/authority | Actual input triggers the fixture's cancellation/revocation witness; the loop refuses before any second input effect. |
| Event stream | Closed channel refuses; Changed and lagged channel paths refresh the snapshot, observe terminal exit and retire the attachment. |
| Remote adapter failure | Attach, resize and snapshot errors unwind the production screen/input guards; no direct input is delivered. |
| Terminal exit/read-only | A terminal exit returns positive metadata with no pending bytes; read-only policy refuses before attachment and raw-mode entry. |
| Failed-attempt isolation | Production `discard_failed_attempt` clears the explicitly owned pending tail and restores native input. |
| Native output errors | Closed owned stdout refuses output admission/write. An owned pipe with no reader consumption admits nonblocking output, fills and reaches the unchanged 250ms production stall bound; original fd flags return after guard drop. No production timeout is shortened. |

Every child checks original `ICANON`/`ECHO` and stdout file flags, released plain
attachment ownership and a final completion marker. The parent checks matching
PTY flags, successful bounded child reaping, alternate-screen retirement where
entered, no config/account/runtime state creation and bounded reader retirement.
All fixture process handles are owned before waits and forcibly retired on fixture
failure; that fallback is not asserted as successful runtime cleanup.

The terminal manager is a counted implementation of the existing
`InteractiveTerminals` presentation contract. It creates no runtime executor and
never calls `list` or chooses another terminal. Its controlled failures and state
changes do not establish OS shell execution, actual remote terminal transport or
model-capture privacy beyond the explicit assertions. Real direct input here is
synthetic fixture data, never human input.

## Verification state and next action

Only source review, rustfmt and `git diff --check` were performed during
preparation. **No Cargo command, compilation, test execution or coverage export
was performed, and no passing result is claimed.** This does not establish 100%
coverage or close #353. Arbitrary fd/kernel failure, every filesystem durability
boundary and native macOS/Windows still require their own evidence; no source is
excluded from the denominator.

After coordinated integration, run the exact existing Helm library target/filter
with `umask 077`, in the coordinator's bounded resource unit:

```sh
cargo llvm-cov --workspace --no-clean --json --summary-only --output-path target/verification-v103/plain-focused-summary.json --locked -j 1 -- plain_terminal::native_tests::owned_linux_plain_prompt_and_attachment_loops_restore_input_and_output
```

The workspace selection preserves the coordinator's instrumentation flags. A
focused summary with earlier/foreign profiles is not a publishable coverage
measurement. Inspect actual failure causes, keep all substantive input/receipt/
cleanup assertions, then run strict applicable checks and independent final full
workspace coverage after final source changes. Count Cargo parent results once;
retain the 22 child libtest results separately. Audit current object/source/profile
identity before publishing `coverage/latest.json`. The full reachable-production
objective and all remaining gaps stay open.
