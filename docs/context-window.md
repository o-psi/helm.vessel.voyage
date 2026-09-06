# Optional request token limits

Helm imposes no context or output-token limit by default. Normal agent requests,
tool follow-ups and automatic title requests do not run a local token admission
check. The provider receives the full request, subject to its own capacity and
protocol. Model metadata does not silently enable a local ceiling.

`context_window = 0` disables the local context check, and `max_tokens = 0` disables
Helm's output-token cap. Both are defaults; omit them from ordinary configuration.
To remove an explicitly configured limit, use `/set context_window 0` and
`/set max_tokens 0`, or the corresponding `--set KEY=0` startup overrides. Inspect
resolved settings with `helm config`. These settings apply to resumed voyages too.

Native OpenAI Chat and Responses requests omit their optional output-limit field
when no limit was requested. The ChatGPT subscription transport does not send an
output-limit field. Anthropic requires `max_tokens`: when the operator has not
specified a value, Helm retrieves the selected model's advertised maximum output
capacity and caches it for that provider instance. A custom Anthropic-compatible
endpoint without this metadata needs an explicit supported output value; Helm does
not guess a smaller cap. An explicitly configured output limit bypasses that
metadata lookup. Provider-required fields are not an additional Helm allowance.

Child agents inherit the absence of a cap. An explicit child or parent output
limit still narrows delegated requests; a child cannot disable a parent's explicit
limit. Automatic titles use the same optional output setting, while their input
excerpt and displayed title remain bounded for their separate UI purpose.

## Deliberately limiting requests

For an operator who wants local preflight and a response allowance:

```toml
context_window = 32768
max_tokens = 4096
```

Positive `context_window` enables conservative local preflight for every initial,
follow-up and title request. When both settings are positive, the output allowance
must be smaller than the context ceiling. Output-only limits work with
`context_window = 0`; a positive context ceiling also works without a fixed output
reservation. A provider adapter may narrow an explicitly enabled context ceiling
using known model capacity. No current native adapter supplies that hook.

The optional preflight estimator counts one token per serialized UTF-8 byte, plus
4,096 framing tokens, 256 per message and tool, and only an explicitly supplied
output allowance. This overcounts ordinary text and is not provider-reported usage
or a universal tokenizer. Without an explicit context ceiling, Helm does not use
this estimator to decide whether work may proceed.

When this optional check exceeds the chosen ceiling, Helm omits the oldest whole
user turns from a request copy. System guidance and the current user turn, including
its tool-call/result groups and steering, stay together. If that indivisible input
cannot fit, the explicit limit produces a local error. Set `context_window = 0`
to remove that local restriction. Saved canonical history remains unchanged;
explicit manual `/compact` is a separate operation.

A local context rejection carries canonical prompt/tool messages, continuation
metadata and usage back to the frontend for save-enabled recovery. Resuming does
not automatically replay tool execution. Estimates and omission counts appear only
when the local budget is enabled. Cancellation and explicit inference allowances
remain independent of token settings.

## Provider capacity and remaining work

Removing Helm's limits does not make a model's context infinite. Actual provider
context rejection remains a provider error. Automatic recovery from that error,
including semantic reduction of a large active tool turn, remains planned under
[#64](https://github.com/o-psi/voyage/issues/64). Do not mistake the absence of a
local cap for delivery of that separate recovery workflow.

Offline regressions cover large uncapped initial/follow-up requests, output-field
behavior, explicit limits, cancellation, delegation, and canonical save/resume.
Provider and native-platform results must be recorded separately; Linux fixtures
do not establish live model quality or other-platform validation.
