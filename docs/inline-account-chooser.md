# Inline account choice; legacy settings form removed (#279)

The old **Settings for next run** form was still reachable from Account despite
the direct model chooser. This delivery removes its renderer, `Mode::Confirm`,
legacy settings editor/input handlers and account-specific model-fetch result path.
It is not merely hidden behind another button.

## Current flow

1. Click Model to open **Choose a model**.
2. Account expands an account list **inside the same chooser**. Current/default
   identity and unavailable sign-in state are visible. Account metadata is fetched
   with a bounded read, authenticated host identity and stale-target fencing.
3. Select an account. It remains a local candidate; its model catalogue is fetched
   using that exact binding. No account/model mutation or prompt is sent.
4. Select the model and optional advanced settings. **Use model** atomically
   applies the account/model/effort/service through the existing reviewed command
   path. Cancel retains original settings and the authored conversation draft.
5. **Sign in / add account** alone enters private account enrollment/setup. Selecting
   an available account from that private view returns to the same chooser, never
   resurrecting the retired settings form. `/account` also enters the inline list.

Account changes retain explicit inference overrides as candidates and require the
same inline Reset/Keep compatibility review as model changes. A newly loaded
account catalogue does not treat the old account's models as advertised. Stale
catalogue replies cannot overwrite a later selection. Account metadata and host
identity remain outside model conversation history; no credential material is
stored in the chooser.

A missing **host default** is a distinct authority change. Its explicit default
consent is retained, not silently set by selecting a row. Confirmed default setup
applies the already-reviewed draft selection without reopening a second model
editor or sending a message. Existing explicit/frozen voyages are not retargeted.

## Verification

194 Helm library tests passed. Tests for the removed editor were replaced with
new-path invariants, while private enrollment, loss/reconnect, expiry, host-default
consent and exact-account tests remain. New tests cover inline staged account,
atomic SetAccountInference, unavailable accounts, sanitized labels, retained draft
and old-renderer absence. Metadata loading is bounded and cancelled on close or
stale destination; rendered account hit maps are invalidated on refreshed lists.

`helm/tests/inline_account.py` ran actual Helm/Vessel/Voyage at 120×40 and 40×18
with two synthetic accounts: inline list, old screen absent, second-account models,
Cancel preserving original binding/revision, one explicit combined apply, unchanged
host default, no prompt/inference and observed cleanup. Evidence under ignored
`target/inline-account/pty-delivery/`. Default-second, direct chooser and main
workspace regressions also passed.

Full workspace coverage passed **498 tests, 0 failures, 1 ignored**, measured
after final edits and recorded in
`coverage/latest.json`: lines 39.5327%, functions 37.5561%, regions 38.3560%.
Compared with the prior measurement these decreased 0.0705/0.0485/0.1003 percentage
points after replacing the old form code/tests; the workspace scope was not narrowed.
Formatting, Python syntax, link and diff checks apply.
Strict Clippy retains 19 pre-existing Helm warnings and is not called clean.
No native-platform, live authentication or billing entitlement claim is made.

Tracked in [#279](https://github.com/o-psi/voyage/issues/279), within broader #272.
Managed installation and installed-binary checks are recorded only after completion;
existing clients must reopen to load the replacement flow.
