# Catalogue failure diagnosis and safe error propagation (#277 follow-up)

The user reported **Could not load models** after the spinner fix. A generic
read-only CLI discovery also failed, but the helper discarded every cause.
This delivery does **not** claim a reproduced root cause for the user's transient
failure: subsequent metadata-only checks found both configured accounts returning
six models, the selected voyage's exact account returning six models over the
actual Vessel socket, and the installed chooser displaying those models.
No inference prompt, paid generation or account-setting mutation was used.

## Confirmed fixes

1. The unbound-account fallback sent a Voyage controls request but did not unwrap
   its public `result` envelope, unlike the earlier `client.voyage` path. This
   prevented that fallback from parsing a valid model inventory. It now accepts
   the wrapped controls result. This is not asserted to explain the named-account
   screenshot.
2. Voyage discovery now returns an allowlisted typed failure frame (configuration,
   workspace, policy, authentication, rate limit, network, invalid response, display
   validation, timeout or unavailable) rather than silently closing stdout for
   every failure. Successful responses remain the model array. Vessel recognizes
   the error frame, observes helper cleanup, and reports a fixed safe marker/message.
   Older peers can continue to reject unknown failure shapes generically; neither
   success compatibility nor private authority is broadened.
3. Helm preserves only that allowlisted category across the socket and into the
   chooser—even for remote routes—without displaying surrounding server diagnostics.
   Errors now distinguish sign-in rejection, rate limiting, malformed catalogue,
   local configuration and other causes. Unknown errors remain generic. Provider
   bodies, tokens, endpoints and raw config are not added to the failure frame.

The existing bounded loading watchdog, explicit Retry, stale-account/generation
checks and no-prompt semantics remain. These messages are not permission to retry
uncertain execution effects; model discovery is a read-only catalogue operation.

## Evidence

- Metadata-only live observations: both configured ChatGPT accounts returned six
  models through the supervisor; the selected voyage account also succeeded over
  the exact WebSocket transport. A separate local Helm observer selected that
  voyage and opened the installed model chooser; six model rows appeared. It
  cancelled the chooser and detached without sending a message. Initial observer
  navigation attempts opened the finder before catalogue hydration; no-match was
  not treated as an account failure. The exact historical failure remains unknown.
- 188 Helm library and 25 protocol library tests passed. Tests cover allowlisted
  diagnostic filtering, unknown/extra field rejection, no raw-body propagation to
  remote UI errors, and wrapped unbound catalogue parsing.
- `helm/tests/model_catalog_errors.py` exercises real helper → Vessel → Helm at
  synthetic HTTP 401, malformed 200 and 429, with deliberately secret-looking error
  bodies. The expected safe category appears, raw body text does not, the draft is
  retained, no inference occurs and cleanup is observed. Loading-timeout/retry and
  normal chooser PTY regressions also pass. Evidence is under ignored
  `target/catalog-diagnosis/`; private probe files are not published.
- Full workspace coverage passed **492 tests, 0 failures, 1 ignored**, with small
  positive line/function/region changes; measured after final edits and recorded in
  `coverage/latest.json`; both wire-contract ends are built and checked. Formatting,
  Python syntax, link and diff checks apply. Strict Clippy still fails on existing
  unrelated warnings; no passing quality claim is made for it.

## Limits and follow-through

The live catalogue was healthy when inspected. If it fails again, the new category
will identify the failure layer without asking the user to guess or share tokens.
Do not claim “bad login” from a generic error or mark the original transient cause
resolved without evidence. #277 remains open for recurrence-specific diagnosis;
the typed error/fallback fixes are completed scope. Managed installation is verified
separately; existing clients must reopen to load new error presentation.
