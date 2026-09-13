# Live tool-call previews

Helm displays provider tool-call generation separately from execution. Voyage owns
bounded display checkpoints; Vessel carries the public snapshot through the
existing authenticated transport. A preview is never a tool admission, executable
JSON, assistant message, or permission to replay an action.

## Presentation and retention

A card appears while arguments are generating. Three decoded argument lines are
shown by default; **double-click** expands up to 128 lines. Keyboard-only: use
**Ctrl+Shift+Up/Down** to select a disclosure and **Ctrl+Space** to toggle it. Unicode wrapping and
terminal-control sanitization use the transcript renderer. Existing scroll anchors
and expansion keys use stable attempt/index identity during generation.
The preview label always says **not executed** and **not final JSON**. Arguments
are decoded for readability, not repaired for execution. A completed canonical
assistant checkpoint atomically replaces the previews with ordinary call cards.
Pending decisions appear as awaiting approval; other active calls retain the
existing Working label. Tool results retain their existing terminal classifications.

Each attempt retains at most 32 previews, 16 KiB per argument preview, 128 bytes
per name and 1024 bytes per call identity. Additional calls beyond this display
budget are not previewed; this does not limit or alter the provider's final response.
Arguments beyond the byte/line budget carry an explicit marker. Final call details
remain available through ordinary transcript expansion/history loading.

Only privacy-filtered display material is persisted in the run journal before
snapshot publication. Configured secrets are withheld across chunk boundaries,
including JSON string escapes. During generation, argument text is shown only for
`shell`, `apply_patch`, `write_file`, `read_file`, `search_files`, and
`list_directory`. Other tools (including `process`, browser, workflows and unknown
external tools) show an arguments-withheld marker. This deliberately avoids parsing
partial JSON as a trustworthy privacy boundary for private input/environment fields.
No opaque provider state is included.

Interrupted/error/cancelled generation remains a clearly unfinished run preview;
reconnect does not execute it or assert that a dead process survived. Public export
of previews is limited to run snapshots, not canonical conversation messages or
provider request reconstruction. Clear/import cleanup clears previews alongside
partial text. Old runs without the optional field load as empty; older Helm clients
ignore it. Content-free checkpoint events invalidate snapshots rather than persisting
an unbounded delta history.

## Pi review and reasoning decision

Reviewed upstream Pi commit `71dca871` in
[earendil-works/pi](https://github.com/earendil-works/pi/tree/71dca871):
`packages/coding-agent/src/modes/interactive/components/assistant-message.ts`,
`packages/coding-agent/src/modes/interactive/interactive-mode.ts`, and
`packages/ai/src/api/openai-responses-shared.ts`.
Pi starts tool components at message updates and updates by call ID before execution.
Its thinking blocks are separate, italic and clickable; Ctrl+T toggles visibility,
whereas Shift+Tab changes generation effort. Thinking is visible by default.
Responses reasoning summaries enter its generic thinking channel.

## Typed reasoning disclosure (U05 follow-up)

The live display now accepts only explicitly typed provider disclosures:
OpenAI Responses (including the native ChatGPT OAuth Responses adapter) emits
`response.reasoning_summary_text.delta` as **Reasoning summary**; Anthropic emits
`thinking_delta.thinking` as **Provider-exposed thinking**. Chat-compatible
`reasoning_content`, encrypted Responses items, Anthropic signatures and redacted
thinking blocks are not displayed. A provider/model that supplies no supported
public disclosure has no block; Helm does not infer hidden reasoning.

Blocks are collapsed initially. Their header offers double-click or
**Ctrl+Shift+Up/Down**, then **Ctrl+Space**, to show/hide; this changes display
only, never the provider's effort setting. A canonical answer
checkpoint marks the associated disclosure finalized and collapses it again. A
cancelled, failed or interrupted generation retains an **unfinished disclosure**,
not an answer or successful tool action. Provisional disclosure text is separate
from canonical assistant text, provider request reconstruction and private replay.

Voyage redacts configured secrets across chunk boundaries before checkpointing or
transport. Each run retains at most 32 display blocks, each capped at 16 KiB on a
Unicode boundary; oldest blocks can be evicted. Signatures/opaque continuation are
never used as prose. The bounded records persist in the run journal and reconnect
through its current run snapshot. They are not canonical conversation history:
older-run browsing/export does not currently expose these records. Clear/import
removes them with other provisional display data. Default deserialization accepts
older records with no disclosures.

Provisional tool keys now always use attempt/index, not a potentially fragmented
provider ID. The UI records bounded correlation and transfers expansion/reading
anchors only when an exact canonical call ID exists. Final canonical calls still
replace, never execute, previews; malformed JSON remains display-only.

## Evidence and remaining acceptance

Focused Rust tests exercise escape/chunk-boundary privacy, Unicode, interleaving,
byte/call bounds, interrupted provider propagation, journal reopen and atomic
canonical reconciliation, plus narrow/normal rendering and expansion.
An offline Linux fixture with real Vessel-supervised Voyage and Helm PTYs at
100×32 and 40×32 observed an incrementally generated `write_file` before any file
existed, double-click expansion, detach/reconnect, one final canonical call and its
single completed file. No paid/live provider was used.

Issue [#255](https://github.com/o-psi/helm.vessel.voyage/issues/255) remains open:
separately hosted remote Vessel interaction, the full malformed/cancellation/error
interaction matrix, fragmented-ID scroll reconciliation, and all-tool argument
preview parity are not established by these checks. Do not interpret local transport
or rendering tests as remote/native-platform certification.
