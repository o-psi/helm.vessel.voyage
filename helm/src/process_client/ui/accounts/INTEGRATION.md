# Helm account UI integration — source handoff for #213

This worktree owns Helm source only. It has not run Cargo, tests, or coverage:
the browser coordinator reserved the shared target. `rustfmt` parse/format and
Git diff checks are source checks, not compilation or runtime verification.

## Existing contract used

All host operations use the existing `Client::request(VesselCommand)` interface.
There is no transport fallback or copied socket implementation. Model discovery
uses `AccountModels` on the execution host; Helm does not resolve account keys or
OAuth token stores. Next-run changes persist exact `SetAccountInference` envelopes
through the existing pending command machinery. Account creates retain the exact
`StartAccount` envelope and recover with `ResolveStartAccount`, never Start replay.

`/account` and the composer Account control share one picker. Account/model/override
confirmation preserves existing overrides unless explicitly edited/reset. The
private alias/device view consumes input before clipboard/composer/history paths.
Its sensitive result uses a view-owned one-shot, not `Update`, ordinary event queues,
conversation, draft serialization, or diagnostics. Only safe enrollment intent and
cancel identities persist, keyed by authenticated host + workspace; remembered
bindings additionally key provider connection. A file lock serializes preference
and enrollment intent changes across Helm instances. Uncertain enrollment remains
retained, not converted to success or automatically restarted.

## Contract blockers that must be integrated before complete acceptance

1. **Configured local creation:** `StartAccount`/`ResolveStartAccount` have no
   configuration/policy envelope. A configured local draft must not silently lose
   tool, root, approval, resource, system, or policy-profile settings when creating
   its voyage. Helm currently refuses that first-send combination and preserves the
   draft/configuration. The parent should add an owner-local optional config path
   (or equivalent complete trusted configuration contract) to both command shapes,
   include it in exact identity comparison, and preserve remote host authority.
   Then replace this explicit guard and extend the matching source tests. Do not
   “fix” it by validating/resolving host credentials on Helm or discarding policy.
2. **Enrollment-only catalogue:** the current `Accounts` request requires account-use.
   An enrollment-only principal cannot discover its allowed device connections.
   The host must supply an enrollment-scoped connection catalogue without revealing
   unauthorized account metadata. Helm currently reports denied/unsupported access,
   never broadens authority.
3. **Socket loss notification:** this source only uses current route availability.
   Once the browser owner's `connection_state()` watch arrives, wire socket loss to
   immediate private-material clearing (including loss/reconnect between UI ticks).
   Reopening must privately read the original enrollment under current authority;
   a socket reconnect must not replay `EnrollAccount`.

## Coordinator verification

Compile the integrated Helm/protocol consumers after the socket merge. Run focused
synthetic tests and the required workspace coverage workflow from the coordinator's
shared target; publish the coverage summary with the parent delivery. In particular:

- configured local and remote drafts, first-send/reconnect exact identities;
- remember/default precedence, unavailable remembered binding refusal, workspace
  changes and authenticated host replacement;
- private enrollment closure, expiry, disconnect, cancel/publication race, denied
  authority, uncertain start/exchange, reopening, and no private material in saved
  Helm state or terminal diagnostics;
- keyboard/mouse picker selection, explicit browser action and small-layout refusal;
- active/current versus next-run model/account catalog context and pending recovery.

No real provider, credential enrollment, paid inference, or native macOS/Windows
security evidence was used. The full issue is not complete until these integration
blockers and actual coordinator verification are resolved.
