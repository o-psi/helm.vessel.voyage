# Model discovery and switching

Helm discovers models from the configured provider or local endpoint; model names are not a
compiled-in catalog. `helm models` prints the available catalog and `helm models --json` emits
stable structured descriptors. OpenAI uses `GET /models` relative to its configured `/v1` API
base, Anthropic uses its paginated models endpoint, and native ChatGPT OAuth uses direct
subscription model discovery. The optional Codex compatibility bridge uses app-server `model/list`.
For local services, use the [explicit preset and endpoint setup workflow](local-providers.md).
Compatible discovery shares the setup bounds: 1 MiB response, 1,024 model entries, 512-byte IDs,
no credential-bearing redirects, and fixed errors that exclude server response bodies.

The configured model remains a valid manual fallback when discovery is unavailable or when an
OpenAI-compatible endpoint accepts a model that it does not advertise. Discovery is bounded by the
normal provider timeout. The interactive agent caches a successful catalog for five minutes;
`Ctrl+R` in the picker forces a refresh.

In full-screen chat, press `Ctrl+M`, type to filter, use the arrow keys, and press Enter. Tab changes
the search field into manual model-ID entry. In line-oriented chat, `/model` shows the current
model, `/models` discovers available models, and `/model MODEL` switches directly.

A switch applies to the next model turn. It does not clear conversation history, terminate PTYs,
discard the workspace todo list, or rebuild the subagent runtime. Newly spawned subagents inherit
the selected model. The session stores both its current model and timestamped change history, so a
resumed session continues with its last selection even when the configuration default differs.

These controls apply to the current local Helm session and its local subagents.
They do not remotely switch another Helm's model. With planned
[multi-Helm participation](voyages.md), interface, coordinator and participants
can be different Helms; voyage scope does not imply shared model settings or credentials.

Provider selection itself remains a startup concern because changing providers can change
credentials, protocol semantics, and model compatibility.

## Automatic session titles

`helm/utility-models.json` selects utility models at build time. Its `title` entry
currently names `gpt-5.6-luna`. Edit that JSON and rebuild Helm to change the title
model. This model is independent of the conversation model: changing the main
model does not change the title model. The request uses the existing provider and
credentials, and runs only when its cached model discovery includes the selected
utility model. Unsupported endpoints or failed discovery keep the existing name;
there is no different-model or different-provider fallback and no pricing lookup.

Automatic titles refresh after successful top-level runs 1, 2, 3, 5, 8, 13, 21,
and subsequent Fibonacci checkpoints. Tool/model iterations and steering messages
within one run do not count separately. The completed-run counter persists across
resume and compaction. A failed title attempt consumes its checkpoint rather than
retrying on every input. Sessions without a completed-run counter start at zero;
existing custom names remain manual. `/name`, `/new TITLE`, and `/branch TITLE` disable
automatic naming. A branch starts its own counter; `/clear` resets the counter and
an automatic name, while preserving a manual name.

Title generation uses a separate tool-free request with a short, redacted excerpt
of recent user/assistant text. It does not include system instructions, tool
results, or provider continuation data, and its response never enters the chat
transcript. Requests are bounded by the provider timeout or ten seconds, whichever
is shorter, with no retries. Providers that support output-token limits receive a 64-token
limit; the subscription adapter omits that unsupported parameter. Title stream
text is capped at 4 KiB and never displayed. Invalid output, errors, and cancellation retain the
previous name. Title usage is stored separately at `title_state.usage` in the
session JSON; existing conversation token totals do not include it. A cancelled
or failed request may incur provider usage that was not returned to Helm.

In full-screen chat, generation runs in the background after the conversation is
saved. Starting another prompt or command cancels pending title work; late results
cannot overwrite a newer run or a manual name. Plain chat and saved one-shot runs
save the completed conversation first, then wait for the bounded title attempt.
`run --no-save` and subagent runs do not request titles. The stable `session-ID`
name remains the fallback until a valid generated title is available.

Validation: `tests/system/session_titles.py` exercises the real CLI with an offline
provider, covering independent model requests, Fibonacci scheduling across restart,
manual names and sessions without title metadata, failures, and discovery fallback.
Unit tests additionally
cover title sanitization, redaction, timeout/cancellation, branch/clear/compaction,
and stale UI results. No shared protocol changes are involved. Live model title
quality and macOS/Windows execution need their own validation; Linux fixtures do
not establish those results. The persisted name remains readable independently of
automatic-title metadata.
