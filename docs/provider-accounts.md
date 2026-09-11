# Named native provider accounts

Tracking: [#213](https://github.com/o-psi/voyage/issues/213). The design and
acceptance matrix are in [the account plan](provider-accounts-plan.md).

Helm's **Account** composer control and `/account` select an executing-host account.
Use different aliases, such as `personal` and `work`, for two accounts with the same
provider. Aliases are display names, not provider identity verification. OpenAI API
billing, ChatGPT subscription access, and Anthropic API billing remain distinct.
There is no quota-driven rotation and no external Codex bridge.

## Select and switch

Open Account on a new draft or existing voyage. The view identifies the authenticated
Vessel, available connections, account labels, and locally observed availability.
Choose an account/transport and review the model, thinking, and service overrides.
Unknown capability support is not entitlement. Explicit edits apply atomically with
the account; rejection leaves the prior selection intact. Escape preserves composer
text. Unavailable accounts require review, never silent fallback.

Switching an existing voyage may send retained conversation and tool context to the
new account and its organization. The confirmation calls this out; switching does
not erase history. While a turn is active, its root, children, retries, and provider
requests retain the admitted account/billing/endpoint identity. The picker stages
**next turn** independently. Safe same-identity credential refresh is allowed; logout
or replacement prevents subsequent dispatch rather than moving the turn to another
account. Requests already sent cannot be recalled.

Selection is held in trusted execution configuration and survives suspension/resume
and branch creation. Branches inherit the selected next-turn account and still need
current execution authority. Exact pending create/configuration envelopes retain the
original identity after a lost response; observation resolves that request instead
of inventing another one. Old servers without account capability are unsupported,
not simulated by a local-only selection.

**Remember for new voyages** is a Helm preference keyed by authenticated Vessel,
workspace, and connection. It does not edit execution-host defaults or change an
existing/pending draft. Host fallback configuration is owner-managed. An unavailable
remembered choice needs explicit review. Changing destination clears the host-local
selection and requires resolving the new host's account.

## Device sign-in in Helm

In Account choose **Add account / Sign in**, select the native ChatGPT connection,
and enter a new safe alias. Adding an existing alias fails; it does not overwrite a
working account. API providers without this device flow offer the private host CLI
alternative, not a callback relay or API-key form in chat.

A dedicated private view displays the fixed provider verification website,
temporary user code, authenticated executing host, expiry, and status. Open the
website only using the view's explicit action (or manually). The code/URL are not
conversation messages, command receipts, events, saved drafts, or model input.
Closure, expiry, completion, cancellation, and connection loss clear the material.

The executing host retains the polling secret, exchanges authorization, and stores
provider tokens privately. They are never forwarded through Helm or another Vessel.
Enrollment does not switch a voyage or start inference. Reopen the original retained
enrollment after disconnect; do not create another attempt because a response was
lost. Resolving the exact original start either observes its retained state or
atomically fences an absent request as not admitted; a late original start cannot
then contact the provider. Cancel is an explicit stable command. If publication won the race, the result
is success, not a fictitious undo. If an exchange outcome is uncertain, the host
retains that state and does not retry the exchange. Local cancellation/expiry is not
proof of upstream revocation.

## Execution-host API enrollment

Run these commands **on the machine executing Voyage**, as its OS account. UUIDs
below are placeholders obtained from safe metadata, not secret-bearing arguments:

```sh
vessel auth accounts connections
vessel auth accounts connect --label "OpenAI API" \
  --endpoint https://api.openai.com/v1 --transports openai-responses,openai-chat
vessel auth accounts add --connection CONNECTION_UUID --account work-api
vessel auth accounts add --connection CONNECTION_UUID --account personal-api
vessel auth accounts list
```

`add` without `--env` requires an attached private human terminal and reads the key
without echo. Escape/Ctrl-C cancels. In Helm, use the executing-host private terminal
workflow; never send a key in chat, a command argument, a URL, or a model-observed
terminal. For remote Vessels, perform the input on that execution host rather than
copying the key through Helm.

An explicitly named host environment binding is also supported:

```sh
vessel auth accounts add --connection CONNECTION_UUID --account work-api --env WORK_API_KEY
```

The argument names the variable, not its value. The registry privately fingerprints
the admitted value; a changed value fails closed. A supervisor's inherited environment
may require restart after environment changes. Stored-key enrollment does not.
An API key is a declared account context, not independently verified organization,
identity, credit, or model entitlement. A compatible transport never permits sending
that credential to a different endpoint.

Host-local lifecycle commands include:

```sh
vessel auth accounts status --account ACCOUNT_UUID
vessel auth accounts rename --account ACCOUNT_UUID --alias work --label Work
vessel auth accounts login --connection CONNECTION_UUID --account personal
vessel auth accounts rotate-api --account ACCOUNT_UUID --generation GENERATION --env WORK_API_KEY
vessel auth accounts logout --account ACCOUNT_UUID
vessel auth accounts logout --account ACCOUNT_UUID --remove
```

Named `login` is the private-terminal alternative to Helm's device view, using the
same durable host enrollment service. `rotate-api` normally advances identity and
invalidates existing bindings. Add `--attest-same-identity` only for an explicit
trusted-owner attestation; this is not provider verification. OAuth reauthentication
uses `reauthenticate --account … --generation … --path PRIVATE_FILE`; a different or
unestablished login requires `--replace-identity`. The login subject and billing
account must both match for identity-preserving refresh/recovery. Opaque legacy
credentials without a subject cannot establish that claim.

Logout tombstones credentials and advances generation before reporting success.
Removal is permanent for that UUID; ordinary logout/reauthentication cannot revive it.
Rename and ordinary OAuth refresh do not change identity. Refresh is coordinated
across processes with a durable per-account intent before network effects. Readers
wait boundedly for another exchange; an unresolved fence is not automatically cleared
or replayed. Reauthenticate explicitly if its owner stopped with uncertain effects.

## Remote authority

The connection must authenticate and pin the target Vessel identity. Public remote
connections use HTTPS; literal loopback HTTP remains a development/local route.
`account_use` and an explicit account UUID allowlist authorize use/selection, separately
from ordinary observe/execute/create rights. `account_enroll` and allowed connection
UUIDs authorize device enrollment, not replacement, deletion, or host administration.
The granting CLI accepts `--accounts` and `--enrollment-connections`; see
[process access](process-access.md) and [Vessel connections](vessel-connections.md).
Existing grants receive neither right implicitly.

An enrollment-only principal may enumerate its permitted connections but receives
no account labels. A newly enrolled profile is usable only by the owner and its
explicitly authorized initiating principal/workspace; it is not added to unrelated
allowlists. Current authority is rechecked at both Vessel and Voyage, including
parent grant revocation. Inherited account references do not grant access.

All Helm requests use the existing [duplex connection](duplex-transport.md). The
private enrollment response is outside canonical journals and generic observations.
Model-facing tools receive no account enumeration, enrollment, or switching capability.

## Existing installations

Legacy OAuth migration is an explicit upgrade boundary:

```sh
# First stop old credential-writing binaries; this flag is an owner assertion.
vessel auth accounts migrate-legacy --old-writers-stopped
```

The legacy cache is retained as upgrade data. Migration is idempotent, including
logged-out caches, and never imports it again after logout. The original migration
binding is retained; replacement cannot silently retarget an old saved configuration.
Do not run old writers against the retained cache after migration.

Legacy native API environment configurations are materialized into stable bindings
on executing-host resolution/admission. The original endpoint, transport, environment
binding, and API billing choice are retained. Admission persists the concrete account
in trusted configuration without changing conversation text. Old scoped grants may
need an explicit account-use update before they can execute with that named binding.
Anonymous explicitly configured local endpoints remain separate from named credentials.

## Verification limits

Use only synthetic keys and loopback providers for unbudgeted checks. The focused
`tests/provider_accounts.py --bin-dir …` journey exercises real Vessel-supervised
processes; Rust tests cover account/device lifecycle and independent refresh. Test
execution and results are recorded with the delivery, not inferred from source.
Actual provider login/refresh requires approved accounts and budget. Linux evidence
does not certify native macOS/Windows storage, deployed TLS proxies, or provider
entitlement. Coverage measures execution, not correctness.
