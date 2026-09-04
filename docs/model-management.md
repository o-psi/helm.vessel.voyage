# Model discovery and switching

Helm discovers models from the account configured for the active provider; model names are not a
compiled-in catalog. `helm models` prints the available catalog and `helm models --json` emits
stable structured descriptors. OpenAI uses `GET /models` relative to its configured `/v1` API
base, Anthropic uses its paginated models endpoint, and the subscription transport uses Codex
app-server `model/list`.

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

Provider selection itself remains a startup concern because changing providers can change
credentials, protocol semantics, and model compatibility.
