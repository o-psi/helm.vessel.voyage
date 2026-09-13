# Named native provider accounts

Tracking: [#213](https://github.com/o-psi/voyage/issues/213). The design and
acceptance matrix are in [the account plan](provider-accounts-plan.md).

New here? Follow [Your first voyage](getting-started.md) for installation, private
sign-in, a first task, and returning to the same conversation. This page is the
account reference, including alternative providers and existing installations.

## Choose the right authentication path

| Goal | Use | Check before the first task |
| --- | --- | --- |
| New ChatGPT subscription/device-flow account | [Helm private sign-in](#device-sign-in-in-helm) | Named account, explicit host default, model availability; experimental transport, not API credit |
| OpenAI or Anthropic API key | [Executing-host private API enrollment](#execution-host-api-enrollment) | Endpoint/transport, API billing and explicit default; a chat subscription is not API credit |
| Existing legacy native OAuth cache | [Legacy migration](#legacy-migration) | Stop old credential writers, migrate explicitly, select a named account and set a default |
| Remote account | [Remote authority](#remote-authority) | Authenticated executing host, account-use/enrollment grants and host-owner default |

A stored credential or a successful login is **not** a verified entitlement, usable
model, or sufficient balance. No live-provider success is implied by these
instructions. Never paste secrets into chat, a question dialog, a tool argument, or
a model-observed terminal. Use only the dedicated private sign-in view or a private
human terminal on the executing host.

Helm's **Account** composer control and `/account` select an executing-host account.
Use different aliases, such as `personal` and `work`, for two accounts with the same
provider. Aliases are display names, not provider identity verification. OpenAI API
billing, ChatGPT subscription access, and Anthropic API billing remain distinct.
There is no quota-driven rotation and no external Codex bridge.

## Select and switch

Open Account on a new draft or existing voyage. The view identifies the authenticated
Vessel, available connections, account labels, and locally observed availability.
The compact list marks **Current** and **Default** separately. Use arrows to navigate,
Enter to continue, **F5** to refresh the selected account’s usage, and Escape to close.
Choose an account and review **Settings for next run**: model, reasoning, and service.
Tab/Shift+Tab or clicks focus editable fields and **Apply / Cancel** buttons. Empty
reasoning/service fields use defaults; the effective catalog value and its provenance
are shown when known, otherwise the UI explicitly says provider-managed/unknown.
Ctrl+U clears a field; F2 clears reasoning/service overrides.
Unknown capability support is not entitlement. Explicit edits apply atomically with
the account; rejection leaves the prior selection intact. Escape preserves composer
text. Unavailable accounts require review, never silent fallback.

Switching an existing voyage may send retained conversation and tool context to the
new account and its organization. The confirmation warns only when account/connection
identity actually changes, not when reselecting the same identity; switching does
not erase history. While a turn is active, its root, children, retries, and provider
requests retain the admitted account/billing/endpoint identity. The picker stages
**next turn** independently. Safe same-identity credential refresh is allowed; logout
or replacement prevents subsequent dispatch rather than moving the turn to another
account. Native dispatch registers refreshed/rotated credentials with the run's shared
redactor before any response or diagnostic can be published. Requests already sent cannot be recalled.

Selection is held in trusted execution configuration and survives suspension/resume
and branch creation. Branches inherit the selected next-turn account and still need
current execution authority. Exact pending create/configuration envelopes retain the
original identity after a lost response; observation resolves that request instead
of inventing another one. Old servers without account capability are unsupported,
not simulated by a local-only selection.

## Required default account

The host owner must explicitly choose a **default account** before new voyages can
start. Applying draft account settings with no default opens **Ready for the first
task**: review the host and named account, then Enter saves that host-wide default;
Esc returns without changing it. Once the host confirms the default, Enter applies
the reviewed settings to the draft without sending a message. **F6 Set default**
opens the separate default
review for account management. There is no automatic first-account selection.
New voyages inherit the default unless explicitly overridden; model/reasoning/service
settings do not need explicit defaults. Legacy per-Helm remembered account choices no
longer override the host default. Existing/frozen voyage selections remain unchanged.
An unavailable default requires human action, never automatic fallback. Replace the
default before removing its account. With no accounts, sign in or use private API
setup first, then select the account, review settings and the explicit host-default
consent. Scoped users ask the host owner to set it.

## Usage observations

The selected account shows a cached **provider-reported Codex allowance** observation,
not a locally counted token total or an API-cost estimate. Opening/navigating the picker
reads the private cache only. **F5** explicitly requests one bounded provider refresh;
there is no background provider polling or automatic account rotation. OAuth fetches
execute on the credential host through the native adapter, not a Codex bridge.

The minimal observation includes primary/secondary windows, percent used, actual
window duration, provider reset timestamp, observation age, and latest refresh status.
Additional categories, credit balances/purchases, local per-account token attribution,
and API costs are not included (see [#71](https://github.com/o-psi/voyage/issues/71)).
Unknown data is never zero. Observations older than five minutes, or retained after a
failed refresh, are marked stale; passing a reset timestamp never locally refills the
allowance. Refresh failure preserves the last successful snapshot and does not block
account selection. Provider observations are not entitlement or guaranteed availability.

Usage reads require a private authorized human connection and the executing host’s
account-use authority. Session/model grants cannot read account usage. Cached replies
are fenced by account identity, connection and capability revisions and the current
Helm socket. Tokens, raw provider bodies, and usage are not added to conversation or
public tool history. Offline fixtures establish parser/authorization/UI behavior only;
the read-only upstream endpoint is not a live compatibility guarantee.

## Device sign-in in Helm

In Account choose **+ ChatGPT — sign in with your subscription**. Select the
native ChatGPT connection if more than one is available, then enter a new safe alias.
Adding an existing alias fails; it does not overwrite a working account.
**+ OpenAI / Anthropic — use an API key…** opens private host CLI instructions,
not a callback relay or API-key form in chat. Both choices remain visible with no
accounts. Review subscription versus API billing before choosing.

A dedicated private view displays the fixed provider verification website,
temporary user code, authenticated executing host, expiry, and status. Open the
website only using the view's explicit action (or manually). The code/URL are not
conversation messages, command receipts, events, saved drafts, or model input.
The code stays visible during status refreshes and pending provider polls. It is
withheld by the host once token exchange begins. Observed expiry, completion,
cancellation, closure, and connection loss clear the displayed material. **O** opens the provider page; enter the code
shown after **Device code** in that page. No code is sent through chat.

On failure, Helm shows a fixed diagnostic category, the failed step, and an HTTP
status when available; provider response bodies are never displayed or retained as
diagnostics. **N** closes the original attempt using its stable cancellation
identity, then offers the same alias for a fresh sign-in. Press **Enter** to request
the new code. The old attempt must be observed as closed before another can begin;
a lost cancellation response is resolved against the original identity. **R**
refreshes that attempt rather than replaying authorization. A genuinely uncertain
token exchange is never retried automatically. Older retained attempts may have no
failure details.

After successful enrollment, the account list refreshes in place and highlights the
new account when it is available to the initiating principal. Select it and confirm
its model/settings to use it. If list refresh fails, **R** retries that read without
starting another sign-in. Success racing with cancellation is shown as success.

The native polling adapter accepts pending HTTP 403/404 responses even when their
body is empty or non-JSON, while explicit denial and expiry errors remain terminal.
Other malformed responses stop with an error rather than replaying an uncertain
effect. The legacy device CLI also recognizes pending/slow-down responses wrapped
with HTTP metadata and respects their polling interval.

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

If the account picker loses its connection, **Enter** opens Vessels when the route
is unavailable, or reloads the account view when the route is available again.
**Ctrl+G** opens Vessels even while an account request is pending. Reconnect there,
then reopen Accounts to inspect any retained enrollment. These actions clear private
view material and never repeat the sign-in mutation. Account action errors appear
inside the picker rather than behind it in the main status line.

## Execution-host API enrollment

This is the alternative API-key branch, not a prerequisite for the ChatGPT device
flow. You need the API provider's credit/billing separately from any chat-product
subscription. In your **own private terminal on the machine executing Voyage**,
as its OS account, choose **one**:

```sh
# OpenAI API credit/key
vessel auth accounts setup --provider openai --account personal-api

# Or Anthropic API credit/key
vessel auth accounts setup --provider anthropic --account personal-api
```

`setup` selects the official provider endpoint and privately prompts for the key
without echo. Escape/Ctrl-C cancels. It creates a named account without a connection
UUID detour; it does **not** send a provider request, validate credit/model access,
or silently set a host default. Never send a key in chat, a command argument, a
URL, or a model-observed terminal. If using a Helm terminal requested for this
purpose, press **F3**, choose its name, and press **Enter** to attach privately;
**Ctrl+]** returns to Helm. Do not type a secret before private attachment.
For remote Vessels, input belongs on that execution host rather than in Helm on
your local machine.

Return to Helm's **Account** view. In the API instructions view, **Enter** refreshes
the accounts; choose the new alias, review model/settings, Apply, and review the
explicit host-default consent if this host has no default. The host owner must
confirm it, then apply the reviewed draft settings with Enter. Rejoin [First task](getting-started.md#4-first-task) after the default
and draft selection are confirmed. A stored key alone is not provider readiness.

### Advanced connections and environment bindings

Custom endpoints and explicit transport selection use `connect`/`add`. UUIDs below
are placeholders obtained from safe metadata, not secrets. Review the endpoint
before enrolling a credential; do not reuse a connection for a different provider.

```sh
vessel auth accounts connections
vessel auth accounts connect --label "OpenAI API" \
  --endpoint https://api.openai.com/v1 --transports openai-responses,openai-chat
vessel auth accounts add --connection CONNECTION_UUID --account work-api
vessel auth accounts list
```

`add` without `--env` also requires an attached private human terminal and reads
the key without echo. The official Anthropic endpoint is
`https://api.anthropic.com/v1` with `--transports anthropic`.

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

### Legacy migration

The commands `vessel auth login`, `vessel auth login --device`, and
`vessel auth import-codex` manage the **legacy native OAuth cache**. They do not,
on their own, enroll a named account or set the host default required for a new
voyage. New users should use Helm's named-account sign-in instead. Importing an
existing Codex cache does not launch Codex or transfer its product billing to API
access.

Legacy OAuth migration is an explicit upgrade boundary:

```sh
# First stop old credential-writing binaries; this flag is an owner assertion.
vessel auth accounts migrate-legacy --old-writers-stopped
```

After migration, use **Account** in Helm to review the named account and explicitly
set the host default (**F6 Set default**). Check the selected model before sending.

Before admitting a turn with an old unbound OAuth configuration, select a named
account or perform this explicit migration. The refusal is intentional: reading a
mutable legacy cache cannot freeze a run's billing identity, and the runtime cannot
assert that old credential writers were stopped on the owner's behalf.

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
