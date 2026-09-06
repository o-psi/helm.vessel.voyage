# Questions

Helm's native `questions` tool lets the model request a multiple-choice clarification
in the full-screen TUI. It is independent of the model provider and works in all
access modes, including read-only. It does **not** grant command/file permissions or
replace a security approval.

## Operator experience

The question replaces the input at the bottom, growing upward to fit its wrapped
content while reserving at least a third of the available body for conversation.
Oversized content scrolls within that space. It takes keyboard focus even when
tool activity is hidden or another picker is open. Other pickers are temporarily
hidden; their state and the unsent chat draft return after the question ends.
Mouse-wheel conversation scrolling is paused while answering; use PageUp/PageDown
for question content. Very small terminals show as much as fits; resize for the
full question and controls.

- **Up / Down / Tab**: select one offered answer or **Other / custom answer**.
- **Enter**: submit the selected option; on Other, submit nonblank custom text.
- **Other**: type or paste text. Left/Right, Home/End, Backspace and Delete edit it.
  Custom text is retained while moving between choices.
- **Esc / Ctrl+C**: cancel this question without inventing an answer. The model
  receives `cancelled` and may continue; this is not a grant of authority.
- **PageUp / PageDown**: scroll long question content. Selection changes scroll
  back to the selected choice. Resize reflows Unicode text.

**Answers are sent to the model and saved as tool results in the session. Do not
enter passwords, tokens, or other secrets.** This differs from direct human PTY
input, which stays outside model-visible records. Known configured secrets receive
redaction once at the tool registry boundary on selected/custom answer text before
final JSON encoding, including values
with quotes, backslashes, or Unicode. Fixed status/index metadata stays intact even
when a configured value matches a schema word; malformed or unknown response fields
are rejected without echoing their content. This is not a guarantee for arbitrary text.
Question/custom input is single-line; pasted control characters are removed.

## Tool contract

```json
{
  "question": "Which output format should I use?",
  "options": ["JSON", "Markdown", "CSV"]
}
```

`question` and `options` are required; unknown fields are rejected. Provide 2–8
distinct, nonblank options. Helm adds custom input automatically; the model cannot
disable it. The question is limited to 2048 UTF-8 bytes, each option to 256 bytes,
and custom answers to 4096 bytes. Control characters are rejected in tool arguments.
JSON Schema string limits are complemented by these runtime byte limits.

Successful tool results have one of these shapes:

```json
{"status":"selected","index":1,"answer":"Markdown"}
{"status":"custom","answer":"A short plain-text summary"}
{"status":"cancelled"}
{"status":"unavailable"}
```

Selected indices are **zero-based** and bind to the exact offered option. The model
must not infer a choice from cancellation or unavailability. Ask only when a user
choice is useful; do not use this tool to solicit secrets or bypass approvals.

## Availability and lifecycle

The current interactive implementation is **full-screen TUI only**. Plain chat,
one-shot runs and standard child-agent frontends return
`unavailable` without reading stdin. Library frontends can opt in by implementing
`Approver::ask_question`; its default is unavailable. The shared frontend trait is
an integration convenience, not shared approval semantics.

Direct PTY attachment rejects incoming questions rather than stealing attached
keystrokes. Overlapping questions or approvals return unavailable instead of
replacing the active dialog. A disconnected frontend also returns unavailable.
Questions obey `command_timeout_secs`, the existing tool-context timeout; expiry
returns a tool timeout error and removes the stale dialog. Run cancellation drops
the pending request and stale responses are ignored. The dialog is in-memory only:
restart restores completed tool history, never a live question or invented answer.

Question results use ordinary tool history; there is no Vessel question-routing
contract. With planned [multi-Helm participation](voyages.md), a question may
originate on a coordinating or participant Helm away from the operator interface.
Forwarding it
to an authorized interface, with clear origin and bounded response lifetime, remains
unfinished; the current local dialog does not supply that behavior.

## Verification

Deterministic Rust coverage includes validation and byte boundaries, selected/custom
results, unavailable frontends, read-only operation, no approval dispatch,
timeout/cancellation, invalid frontend replies, known-secret redaction, provider-loop
continuation, session round trips, keyboard/paste editing, draft preservation,
concurrent/stale requests, PTY isolation, closed channels, and Unicode/control-safe
rendering at resized/narrow viewports. The `questions-unavailable-no-invented-answer`
evaluation scenario checks model behavior in unattended mode when run with a live
provider; manifest validation alone does not execute it.

The Linux CI workflows also run the real PTY/native Responses fixture:

```sh
HELM_BIN=target/release/helm python3 tests/system/questions.py
```

It verifies selected/custom/cancelled/timeout/plain-mode flows, including escaped
known-secret redaction in a custom answer, through native HTTP
transport, real keyboard and bracketed paste routing, resize, model continuation,
clean TUI exit, and saved tool history. This script uses POSIX PTYs; Windows behavior
is covered only by the portable Rust fixtures unless separately exercised manually.

Tracking issue: [#80](https://github.com/o-psi/voyage/issues/80).
