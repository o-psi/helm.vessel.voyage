# Request context limits

Helm preflights each agent request, including tool follow-ups and automatic title
requests, before calling the provider. `context_window` is the effective token
ceiling, including the `max_tokens` output reserve. The default is a finite
65,536-token fallback. Configure a lower value for a smaller model; do not assume
model discovery establishes a context limit. An adapter can supply a smaller
known model limit, which takes precedence. Switching models rechecks the limit on
the next request. The same configured ceiling applies to child agents.

```toml
context_window = 32768
max_tokens = 4096
```

Use `/set context_window 32768` or `--set context_window=32768` to change the
configuration. The output reserve must be positive and smaller than the window.
`helm config` exposes both values. Setting a larger value is an operator assertion
about the endpoint's capacity, not discovery or proof that the model supports it.

Current adapters do not expose a local tokenizer. Helm conservatively counts one
token per serialized UTF-8 byte across the model, messages, continuation metadata,
tool definitions, sampling settings, and output settings. It adds 4,096 tokens of
framing headroom, 256 per message and tool, and the output reserve. This estimator
intentionally overcounts ordinary text; it is not measured provider usage and
cannot establish a universal token bound for arbitrary custom tokenizers or hidden
server instructions. The optional Codex compatibility bridge retains its own
external runtime; Helm cannot measure that runtime's hidden context. Provider
context errors remain actionable failures, never successful completion.

When the estimate exceeds the ceiling, Helm repeatedly removes the oldest complete
user turns from a request copy and includes an omission notice as runtime context.
System guidance, the latest user turn, and all of that turn's tool calls/results
remain together. A large active tool loop, prompt, schema, or instruction block
that cannot fit fails locally with `ContextError`; it is not truncated and no
oversized request is dispatched according to Helm's estimate. Reduce input or
tool output, start a new turn, or correct the configured ceiling before retrying.
This introduces no model-turn limit and does not change tool permissions.

Automatic reduction never changes canonical conversation history or inserts a
synthetic human message. Saved sessions and checkpoints retain the original text;
resume deterministically rebuilds the bounded request projection. Explicit manual
`/compact` remains a separate operation. Request estimates, ceilings, and omitted
message counts appear in terminal status/diagnostics without logging message
content. Cancellation still prevents dispatch; preflight failures are not retried.

Offline tests cover request accounting, exact boundaries, whole-turn reduction,
tool-group integrity, initial/follow-up rejection, model changes, cancellation,
and canonical save/resume. Linux checks cannot establish macOS/Windows or live
provider behavior; those results must be recorded separately in the tracking issue
[#64](https://github.com/o-psi/voyage/issues/64).

A context rejection carries the run's canonical prompt, completed assistant/tool
messages, continuation metadata and usage back to its frontend. Save-enabled
one-shot runs and plain/full-screen chat persist that recovery snapshot and report
an error; they do not count the rejected run as completed. `--no-save` still skips
persistence. A later explicit resume retains this evidence and rebuilds a projection
without automatically replaying tool execution. Full-screen recovery also retains
steering accepted after the rejected snapshot and marks that input not applied.
Runtime system guidance is excluded from the saved snapshot; continuation metadata
remains local session data.
