# Unified chooser initialization and reconnect (#279 follow-up)

The account settings editor was removed, but the old **Choose account** list could
still open automatically during unresolved draft initialization and first send.
Connection loss in that view reproduced the screenshot despite the successful
existing-account switch tests. Those tests missed the initialization branch.

## Fix

Unresolved first-use and first-send paths now create an **initializing Choose a
model** chooser. Its bounded account read also resolves the authorized host default.
A usable inherited default hydrates the draft and returns automatic startup to the
composer, without an account selection detour or implicit prompt. Explicitly opening
the chooser continues to the model list. An absent or unavailable default stays in
the inline account list; no first-account fallback or host-default mutation occurs.

Account-list timeout/connection failure is shown inside that same chooser with
**Retry**, **Cancel**, and explicit sign-in access. Retry after reconnect refreshes
the original destination. Failed reads cannot apply settings. Pending/frozen draft
identity, explicit provider compatibility and private sign-in/default-consent
boundaries remain enforced. No automatic initialisation or first-send call opens
the private legacy account list anymore. That private surface is reserved for
explicit sign-in/add-account, host-default consent and recovery within those flows.

## Actual verification

195 Helm library tests passed. New injected metadata-failure test checks initial
chooser ownership, Retry/error visibility, no old account view, no session creation
and retained text. The existing first-send test now asserts the new chooser rather
than incorrectly treating the old account screen as required behavior.

`helm/tests/chooser_initialization.py` exercises real unconfigured first use and
first-send, stops only its synthetic Vessel, observes the inline account error,
restarts that fixture, retries successfully and cancels with draft retained. It
asserts **Choose account** and **Settings for next run** never appear, and no
voyage/inference is created. Cleanup is observed. Default-second and inline combined
account/model apply regressions pass separately. Evidence is ignored under
`target/chooser-init/`.

Full workspace coverage passed **499 tests, 0 failures, 1 ignored**, with positive
line/function/region deltas; measured after final source/test edits and committed
in `coverage/latest.json`. Formatting, Python syntax, link and diff checks apply.
Strict Clippy retains 19 pre-existing warnings, not a clean pass. No native/live
provider or all-UI completion claim is made. Installation is verified separately;
existing clients must reopen to load the corrected initialization path.

Tracked in [#279](https://github.com/o-psi/voyage/issues/279). This closes the missed
initialization/reconnect entry path, not the broader #272 redesign or independent
network reliability investigation.
