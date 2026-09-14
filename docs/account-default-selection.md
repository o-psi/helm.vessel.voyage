# Default account selection before model discovery (#278)

The previous model checks exercised catalogue availability, not correct default
selection. The account picker initialized selection to row zero; unresolved saved
draft settings could also suppress default hydration. These were real gaps even
when direct catalogue reads succeeded.

## Implemented behavior

- An unfrozen draft with no concrete account resolves the authorized executing-host
  default before model discovery. Previously saved settings without an account are
  treated as unresolved, not initialized. Existing model/effort choices are retained
  where provider-compatible; an explicit account or provider mismatch is not silently
  replaced. A usable inherited default completes initial account hydration without
  leaving the user in an unnecessary account-confirmation loop.
- The account picker highlights the exact current binding, or host default if
  unresolved, including when it is the second row. An unavailable current/default
  row remains selected but non-actionable; Enter does not pick another billing
  identity. Refreshes retain that selection unless an explicit enrollment focus
  names the newly created account.
- Failure to read host defaults is no longer swallowed into empty defaults. No
  default or unavailable default requires explicit recovery; the first available
  account is not automatically chosen. Host default is never changed by hydration.
- AccountModels includes the validated account label alongside its exact binding.
  The chooser caches a bounded, sanitized label only for the matching request and
  account. Users see the account name without opening a private account picker
  merely to load its label. Credentials and usage data are not included.
- Existing/frozen voyages keep their selected account even if the host default
  changes. This fix does not retroactively switch their billing identity.

## Verification

192 Helm library tests passed, including second-listed default preselection,
explicit-first preservation, saved-unbound-draft hydration with retained text,
unavailable default refusal, no default and incompatible-provider handling.

`helm/tests/account_default.py` uses real Helm/Vessel/Voyage with two synthetic
accounts. It makes the second-listed account the host default, opens an unsent
new draft and verifies default resolution, chooser label and account-row selection.
It then opens an explicitly first-account-bound voyage and verifies that binding
wins over the host default. No inference request or unintended voyage creation
occurs; host default is unchanged and fixture cleanup is observed. Evidence is
ignored under `target/account-default/pty-verified/`. Normal chooser and stalled
catalogue/retry regressions also passed.

Full workspace coverage passed **496 tests, 0 failures, 1 ignored**, with small
positive line/function/region changes, measured after final source/test changes and committed
in `coverage/latest.json`; both AccountModels wire ends are built. Formatting,
Python syntax, docs links and diff checks apply. Strict Clippy retains pre-existing
warnings and is not reported clean. Linux synthetic tests do not establish native
or live-provider billing/account entitlement behavior.

Tracked in [#278](https://github.com/o-psi/voyage/issues/278), related to #272/#277.
Managed installation and installed-binary verification are recorded after actual
completion. Existing clients must reopen to load the fix; no user account/default
mutation is needed for deployment.
