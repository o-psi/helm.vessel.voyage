# Inference resolution verification — #182

Verified on Linux on 2026-09-08 using development binaries and synthetic,
loopback-only provider fixtures. No paid inference, real account endpoint, or
operator credential was used. These were one-shot checks, not recreation of the
removed automated suites. Live account entitlement, pricing, native macOS/Windows,
and deployed upgrades are not established by these results.

## Behavior observed

- Native ChatGPT catalog reasoning and service defaults were encoded in actual
  supervised request bodies. An advertised non-enum service ID also reached the
  request; the shared Responses encoder no longer misapplies the API tier enum to
  ChatGPT requests.
- Explicit settings changed during an active turn applied only to the next turn.
  Inherited defaults stayed frozen through four provider requests and intervening
  tool calls despite catalog changes. The provider-reported tier was independent
  of the requested tier.
- A subsequent response without tier metadata cleared the last-response
  observation. OAuth account-identity text echoed as tier metadata was discarded,
  without failing or retrying completed inference.
- Switching the synthetic executing account with the same provider/model selected
  the new account's catalog defaults. Pure resolver checks also verified endpoint,
  provider and credential-binding cache identity changes, model mismatch, and
  expired observations.
- Known-empty reasoning support rejected an explicit effort, while inheritance
  omitted it. Missing metadata remained unknown. An unavailable catalog did not
  prevent explicit transport-valid settings from reaching the fixture.
- Contradictory defaults, oversized option arrays, terminal-unsafe text and
  credential-bearing default metadata were rejected without publishing rejected
  values. Model metadata could not broaden Anthropic or compatibility adapter
  override support.
- Both OpenAI Responses and Chat Completions kept missing catalog defaults unknown,
  omitted inherited unknown fields, sent explicit `service_tier: "default"`, and
  encoded their respective reasoning fields. Unknown API tier IDs were rejected
  locally. ChatGPT's explicit `default` instead suppressed catalog selection and
  omitted the service field.
- Existing next-turn checks passed for identical command deduplication, conflicting
  payload refusal, stale/unsupported-setting preservation, tool-loop freezing,
  resumed request values, clearing overrides, and supervisor-restart persistence.
- Actual Helm PTY interaction verified automatic local-draft default discovery,
  Thinking selection, explicit Service `default` versus `inherit`, saved draft
  values, cancellation preserving composer text, and an account change during
  foreground discovery invalidating the picker before fresh metadata was shown.
  The draft interaction issued zero inference requests, and background discovery
  did not print creation diagnostics into the TUI display.
- Temporary discovery cleanup initially exposed a redundant-Stop race after a
  successful Delete. The corrected path waits for the exact deletion receipt and
  clean stopped state through read-only inspection; it does not replay uncertain
  mutations. All fixture-owned runtime cleanup was observed.

## Verification artifacts

Local evidence (not repository test entry points):

| Check | Evidence |
|---|---|
| Supervised ChatGPT resolution and response metadata | `/tmp/vdr-mk67ndji/results.json` |
| OpenAI Responses and Chat Completions wire values | `/tmp/vdr-01nnfxgj/results.json` |
| Helm PTY defaults, selection, cancellation, account switch | `/tmp/vdr-xgi_7iv6/results.json` |
| Prior next-turn/deduplication/restart behavior | `/tmp/vdr-clexu1jf/results.json` |
| Pure resolver, scope, expiry and metadata validation | `.local-git/inference-resolution-verify.log` |

Build/analysis commands used for the affected packages:

```sh
cargo check -p helm -p voyage -p vessel --locked
cargo clippy -p helm -p voyage -p vessel --locked -- -D warnings
cargo build -p helm -p voyage -p vessel --locked
```

Changed Rust files were checked with rustfmt; source/diff checks preserved unrelated
workspace deletions and concurrent work. A shared development build artifact was
found stale after concurrent work; rebuilding the affected protocol crate and
consumers restored a consistent build without clearing the build cache.

## Deliberate boundaries

Catalog support/defaults are not account entitlement or proof of delivered service.
Public API project service settings not exposed by discovery remain unknown.
Anthropic and the compatibility adapter do not acquire new generic override
encodings from metadata. Remote drafts still retain executing-host settings.
Response-tier observations are live per-turn metadata, not a persistent billing
ledger. Capability metadata may be unknown following suspension/restart until
rediscovery; explicit selections remain durable.

See [configuration](configuration.md#next-turn-inference-controls) for inheritance,
provider-specific `default`, freshness and cost semantics.
