# Local and compatible providers

Use `helm local-provider` to configure Ollama, LM Studio, vLLM, llama.cpp, or a
custom OpenAI-compatible service. Helm runs its own native agent and tools; these
presets do not start Codex, install models, or change local tool permissions.

```sh
helm local-provider presets
helm local-provider probe ollama --model qwen3.5:2b
helm local-provider setup ollama --model qwen3.5:2b --output ./helm-local.toml
helm --config ./helm-local.toml doctor
helm --config ./helm-local.toml models
helm --config ./helm-local.toml run "Explain the files in this directory"
```

The output config is created only after validation. An existing destination,
including a symlink, is never replaced. If output is lost or publication reports
an error, inspect the destination before retrying. Setup creates an ordinary
Helm config with default local policy; review the file before using it for work.
Existing configs are not edited or imported implicitly.

| Preset | Default API base | Default transport |
|---|---|---|
| `ollama` | `http://127.0.0.1:11434/v1` | `openai-chat` |
| `lm-studio` | `http://127.0.0.1:1234/v1` | `openai-chat` |
| `vllm` | `http://127.0.0.1:8000/v1` | `openai-chat` |
| `llama-cpp` | `http://127.0.0.1:8080/v1` | `openai-chat` |
| `custom` | Required `--endpoint` | `openai-chat` |

Every preset accepts `--endpoint` for a different explicitly selected API base.
Include the API prefix, usually `/v1`, rather than a completion route. URLs must
use HTTPS or numeric loopback HTTP; credentials, queries and fragments are
rejected. Use `127.0.0.1` or `[::1]` for local HTTP. Chat is the shared default;
select `--transport responses` only when the service supports that protocol.
Local Chat presets set `chat_use_max_tokens = true`, using the widely supported
`max_tokens` limit in both validation and runtime requests. Use
`--chat-max-completion-tokens` for a Chat endpoint requiring the newer field.
Existing configs keep their `max_completion_tokens` default.

## Credentials and validation

Omitting `--api-key-env` explicitly selects `api_key_required = false`. Helm does
not read or send the ambient OpenAI key in this mode. For a protected endpoint,
name its dedicated environment variable:

```sh
helm local-provider setup custom --endpoint https://models.example.com/v1 \
  --api-key-env LOCAL_MODEL_API_KEY --model my-model --output ./helm-model.toml
```

Set that variable through your normal secret-management workflow; its value is
never stored in the generated config. Existing configs retain required API-key
authentication by default. Changing provider restores that requirement. No-auth
is available only for an explicitly configured native compatible endpoint.
ChatGPT OAuth subscriptions and public API billing remain separate; these
commands never borrow subscription credentials or select a compatibility bridge.

A probe requests `/models`, then sends a fixed tool-free compatibility request
capped at 16 output tokens. It may load a model and consume endpoint capacity.
`--timeout-secs` sets a total 1–120 second deadline (default 15). Responses are
limited to 1 MiB, catalogs to 1,024 entries, and IDs to 512 bytes. Redirects are
not followed, and setup/scan requests bypass environment proxies. Model output
is discarded; only bounded model metadata and the validation result are shown.

Supply `--model` to choose a model explicitly. Otherwise the first sorted model
ID is selected. A manual ID works when `/models` returns 404, 405 or 501, and can
select a model not advertised by a compatible service. Empty catalogs require a
manual ID. Authentication rejection, connection failure/timeout, incompatible
protocol responses and malformed model lists produce distinct diagnostics and
prevent config creation. A successful probe validates the selected request
format; it does not establish model quality, tool-calling support, every stream
feature, or future availability.

`helm config` and `helm doctor` show resolved transport, endpoint and credential
source, never the credential value. Their discovery state is `not_probed`:
neither command performs inference or implicit network discovery. Use an explicit
probe to check current availability. Saved config contains no stale claim of
permanent compatibility. Existing `helm models` and the interactive model picker
use the resulting provider configuration and the same bounded compatible catalog parser, and manual model switching remains
available.

## Explicit loopback discovery

```sh
helm local-provider scan
```

This command alone checks the four numeric IPv4 loopback candidates in the table,
using only `GET /models`, no credentials and no redirects or proxies. Each check
has a two-second bound. It never scans other addresses/ports, starts a service,
loads a model through inference, or saves configuration. A reported catalog does
not validate inference; follow up with `probe` or `setup`. Presets, normal startup,
config inspection and doctor never trigger this scan.

## Compatibility evidence

The maintained endpoint references are [Ollama's compatibility guide](https://docs.ollama.com/api/openai-compatibility),
[LM Studio's compatibility endpoints](https://lmstudio.ai/docs/developer/openai-compat),
[vLLM's server documentation](https://docs.vllm.ai/en/latest/serving/openai_compatible_server/),
and [llama.cpp's server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).
Runtime versions and model capabilities vary; the explicit probe checks your
selected endpoint rather than inferring compatibility from a product name.

`tests/system/local_provider.py` exercises all presets and both native transports
through real HTTP and CLI processes, with authentication, bounds, failures,
manual fallback, concurrent create-only publication and interruption coverage.
Platform and actual-runtime smoke results are recorded separately in the tracking
[issue #66](https://github.com/o-psi/voyage/issues/66); simulated service fixtures
do not establish compatibility with an untested runtime version.
