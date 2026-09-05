# Completion tool argument contract

The `completion` tool reviews work owned by the current run. Its advertised JSON
Schema describes the same action-specific required and allowed keys as its strict
argument decoder. Fields from another action are errors, even if their values are
otherwise valid.

| Action | Required and allowed keys | Purpose |
| --- | --- | --- |
| `snapshot` | `action` | Obtain current revision, fingerprint and unresolved IDs |
| `read` | `action`, `kind`, `id` | Inspect one owned todo or agent result |
| `adopt` | `action`, `kind`, `id`, `revision` | Explicitly adopt eligible historical work |
| `account` | `action`, `kind`, `id`, `revision`, `fingerprint`, `disposition`, `reason` | Record a reviewed disposition against fresh state |

`kind` is `todo` or `agent`; IDs are UUID strings copied from actual records.
`revision` is an unsigned integer. For `account`, copy the exact revision and
fingerprint from a fresh snapshot after any state changes. `read` accepts neither
revision nor fingerprint, reason or disposition. Missing fields, unknown fields,
wrong types and unsupported actions/dispositions remain errors.

For example, first call `{"action":"snapshot"}`, then read an actual returned ID:

```json
{"action":"read","kind":"todo","id":"00112233-4455-4677-8899-aabbccddeeff"}
```

The UUID above is illustrative. The four schema examples demonstrate argument
shapes, not authority to reuse their sample IDs, revisions or fingerprints.
Account only after inspecting real evidence/results. Preserve pending or blocked
work and state its impact honestly. A syntactically valid reason or fingerprint
does not satisfy the runtime's evidence, freshness or ownership checks.
[The completion gate contract](completion-gate.md) defines those checks and
[validation evidence](completion-validation.md) records their coverage and limits.

## Transport and verification boundaries

The schema keeps top-level object properties visible to compatible templates and
uses four mutually exclusive `oneOf` branches to specify allowed/required keys.
It uses standard JSON Schema constructs without changing the argument protocol.
The [JSON Schema specification guide](https://json-schema.org/understanding-json-schema/reference/combining)
explains how `oneOf` and shared constraints combine.

Current native adapters forward the schema unchanged. Responses explicitly sends
`strict: false`; Chat leaves strict mode unspecified, which defaults to non-strict
according to [OpenAI's function calling documentation](https://developers.openai.com/api/docs/guides/function-calling).
Anthropic forwards it as `input_schema`, the JSON Schema parameter documented in
[Claude's tool definitions](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools).
This change does not opt any provider into strict generation or promise every
compatible server/template implements all JSON Schema features. Tool decoding and
runtime checks remain authoritative when a model generates an invalid call.

Tests use an independent Draft 2020-12 validator with format checks enabled against a
valid/invalid corpus and the strict decoder. Parity claims cover canonical JSON
UUID strings and integer representations, rather than every permissive serde UUID
spelling or integral floating-point encoding. Syntax tests intentionally accept
empty review strings that the runtime may reject semantically. Actual loopback
HTTP fixtures check ordinary and streaming requests through Chat, Responses and
Anthropic, preserving tool schemas and output limits.

The motivating local live run failed honestly with six malformed reads and no
accounting. Accurate schema guidance fixes that contract mismatch; deterministic
fixtures do not establish improved live model behavior. No new live run accompanies
this correction, and successful reconciliation/subscription acceptance remains
separate. There is no storage migration, auto-accounting or authority expansion.
