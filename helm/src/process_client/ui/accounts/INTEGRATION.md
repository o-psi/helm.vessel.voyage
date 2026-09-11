# Helm account UI integration (#213)

User operations and qualification limits are documented in
[Named provider accounts](../../../../../docs/provider-accounts.md).
The public account/enrollment types live in `voyage_protocol::accounts` and
`voyage_protocol::vessel`. Helm uses the ordinary duplex `Client::request`, not a
second transport or HTTP/SSE fallback.

## Identity and persistence

`/account` and the composer Account control share the keyboard/mouse picker.
Confirmation stages an atomic account/model/override envelope and preserves
composer text and the admitted run's identity. New-voyage preferences are keyed by
authenticated Vessel/workspace/connection, never alias or route index. An unavailable
remembered identity cannot silently fall back.

Configured local drafts restore policy metadata using `Saved::launch_config()` and
the existing frontend launch persistence. `StartAccount` includes an owner-local
optional trusted JSON path; `ResolveStartAccount` retains that exact path. Scoped
remote creation accepts no local configuration path. Neither path resolves remote
provider credentials on Helm.

## Private enrollment lifecycle

The view owns its one-shot receiver and temporary response. Neither is an `Update`,
conversation message, ordinary event, diagnostic, or serializable draft. Durable
storage contains only the original public enrollment envelope and cancellation ID.
An enrollment-only caller can inspect its allowed connections without account labels.

`Client::connection_state()` supplies the view-owned watch. A change in
`loss_generation` invalidates the request/view ID, drops queued responses, and clears
codes even if disconnect/reconnect coalesce between ticks. Rendering checks the
current generation too. Explicit R resolves the original admission and privately
reads status; it never repeats `EnrollAccount`. Resolving an absent original fences
it as not admitted so a late start cannot launch provider authorization.

Closure, expiry, denial, cancellation, success, or socket loss clear sensitive
material. Codes must be nonempty and bounded; the verification URI must exactly
match the supported native provider flow. O is the sole explicit browser action,
passes only the fixed URI with null standard streams, and permits one bounded,
reaped launcher at a time. It bounds the launcher, not the user's browser navigation.
Unsupported platforms show manual navigation guidance.

## Focused checks

`accounts/app_tests.rs` exercises actual App input, render, reply, and persistence
paths with synthetic one-shots and watches: keyboard/mouse selection; exact pending
recovery; current/next-run separation; composer/history preservation; saved-state
scanning for private material; explicit browser action; expiry/denied reads;
repeat-cancel identity; publication winning cancellation; closure/narrow layouts;
coalesced connection loss; configured launches; and remembered/unavailable bindings.
The fixtures use temporary per-test storage and a browser-effect substitute, not
operator HOME, real providers, or a real browser. Helper tests cover aliases,
availability, host/workspace keys and legacy deserialization.

The delivery's recorded Cargo/process results and `coverage/latest.json` are the
verification evidence. Linux synthetic tests do not certify live-provider login,
deployed TLS proxies, native macOS/Windows private storage, or platform launchers.
