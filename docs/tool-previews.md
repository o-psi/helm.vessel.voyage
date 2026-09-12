# Live tool-call previews

Helm displays provider tool-call generation separately from execution. Voyage owns
bounded display checkpoints; Vessel carries the public snapshot through the
existing authenticated transport. A preview is never a tool admission, executable
JSON, assistant message, or permission to replay an action.

## Presentation and retention

A card appears while arguments are generating. Three decoded argument lines are
shown by default; **double-click** expands up to 128 lines. Unicode wrapping and
terminal-control sanitization use the transcript renderer. Existing scroll anchors
and expansion keys use provider call IDs when available, with attempt/index fallback.
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

**Decision for this delivery:** adopt early tool generation cards; do not yet adopt
reasoning display. Existing native `ProviderDelta` has text and tool calls, not a
typed displayable reasoning channel. Responses retains private continuation items;
Anthropic signed/opaque thinking requires protocol-specific handling. Chat-compatible
models do not offer a uniform display channel. No provider is claimed to display
reasoning in Helm today, and existing Thinking/inference settings only configure
generation. Private replay records must not be surfaced as a shortcut.

A future implementation should label **Reasoning summary** separately from
**Provider-exposed thinking**, stream only explicitly supplied content, use collapsed
blocks by default and after completion, offer a discoverable independent toggle
(Ctrl+T is already Activity in Helm), and retain sanitized bounded display records
separately from private signatures/encrypted continuation. Unsupported providers
should show no fabricated block. That adoption requires its own typed channel,
privacy/retention tests and provider-support evidence.

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
