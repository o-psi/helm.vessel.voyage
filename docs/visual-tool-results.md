# Native visual tool results

Tool images (including MCP and shared-browser artifacts) remain canonical
`Role::Tool` messages containing ordered `ToolOutput` and the original
`tool_call_id`. No authored User message is created to carry them.

## Adapter contract

- Native OpenAI Responses: ordered `input_text` / `input_image` blocks in
  `function_call_output.output`, with the original `call_id`.
- Native Anthropic: ordered text / base64 image blocks nested inside `tool_result`,
  with original `tool_use_id` and `is_error`. The enclosing wire role is `user`,
  as required by Anthropic's tool protocol; the canonical role is still Tool.
- OpenAI Chat: explicitly refuses visual tool outputs. Its tool-message content
  schema permits text, not image blocks. Ordinary User image inputs are unchanged.
- Native ChatGPT OAuth: uses the same Responses function-output encoding. The
  official OpenAI client protocol defines image-bearing `FunctionCallOutputBody`
  content arrays explicitly (source below). This is native wire compatibility,
  not an external Codex bridge or another agent loop. Live account/model acceptance
  remains unverified; normal model image capability checks still apply.

Supported adapters refuse visual output whose original call is absent. They
never downgrade it to metadata or relabel an orphan as User image input. Other
non-image tool content retains its deterministic text representation in the
ordered block list. Structured content follows the ordered content blocks.

## Authority and limits

Immediately before provider dispatch, the agent clones its history and projects a
newest suffix of image-bearing messages, sharing a budget across User attachments
and Tool images: four occurrences, two MiB of raw image data. An image-bearing
message is retained whole or omitted whole. The newest image result must fit or
dispatch fails explicitly. Older images become labelled omission text, retaining
tool-call provenance. Canonical history and durable references are not modified.

Only retained Tool images are hydrated from the executing session's authorized
artifact store. Exact reference metadata, session ownership, byte length and
SHA-256 are checked; real PNG/JPEG/WebP raster validation enforces dimension,
pixel, container completeness and non-animation constraints. MIME declarations
alone never authorize a raster. No remote URL is fetched. Image bytes are
in-memory, serialization-skipped data, not transcript fields. Existing branch
artifact copying and resume reference checks retain the original references.

Native dispatch independently validates request-wide image counts and bytes,
per-message content limits, selected model image capability and the complete
encoded JSON limit (eight MiB). Context budgeting includes a conservative image
cost for Tool artifacts, which do not carry decoded dimensions. Provider error
redaction treats tool-image requests as image-bearing requests too.

## Protocol evidence and verification boundary

Official generated SDK schemas read on 2026-09-10 (no paid provider requests):

- [OpenAI Responses function output](https://github.com/openai/openai-python/blob/603b81f9e228d95032059169cb3a0e7dc12ba93f/src/openai/types/responses/response_input_param.py)
- [OpenAI output block union](https://github.com/openai/openai-python/blob/603b81f9e228d95032059169cb3a0e7dc12ba93f/src/openai/types/responses/response_function_call_output_item_list_param.py)
- [OpenAI image input block](https://github.com/openai/openai-python/blob/603b81f9e228d95032059169cb3a0e7dc12ba93f/src/openai/types/responses/response_input_image_content_param.py)
- [OpenAI Chat tool message](https://github.com/openai/openai-python/blob/603b81f9e228d95032059169cb3a0e7dc12ba93f/src/openai/types/chat/chat_completion_tool_message_param.py)
- [Anthropic tool result](https://github.com/anthropics/anthropic-sdk-python/blob/eb21a4352015686c30f5759e8c2f02d70f5371e2/src/anthropic/types/tool_result_block_param.py)

- [Official OpenAI client function-output body and image schema](https://github.com/openai/codex/blob/8e2afc09126c0cea4c282725fe68af43adad73d7/codex-rs/protocol/src/models.rs)

These establish documented request shapes, not observed cloud acceptance or image
reasoning quality. Scoped synthetic checks are supplied for encoding, provenance, refusal,
raster integrity, authorization, projection and reference retention. Run them
only in the parent-coordinated Cargo slot, using the shared target directory:

```sh
CARGO_TARGET_DIR=/home/psi/voyage/target cargo check -p voyage --locked
CARGO_TARGET_DIR=/home/psi/voyage/target cargo test -p voyage --locked --lib provider::multimodal::tests
CARGO_TARGET_DIR=/home/psi/voyage/target cargo test -p voyage --locked --lib provider::anthropic::visual_tool_checks
CARGO_TARGET_DIR=/home/psi/voyage/target cargo test -p voyage --locked --lib model::visual::checks
```

These commands are scoped synthetic checks, not broad suite recreation. Native
macOS/Windows private artifact storage is not established by Linux checks.
