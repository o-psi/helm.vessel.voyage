# Direct model chooser (#276)

The intermediate **Model options → Model → picker** menu is removed. The composer
Model control, Helm → Model and account, `/preferences`, and `/model` open the
same centered **Choose a model** dialog directly.

## Interaction

- Search filters the model list. Rows identify the current model and the locally
  selected candidate. Click/arrow selection changes only the candidate, never
  runtime settings. Wheel moves the browsing position without applying anything.
- **Use model** explicitly commits the candidate through the existing
  revision/account/override-validated inference path. **Cancel** or Escape closes
  without applying local choices or changing the conversation draft.
- Tab/Shift-Tab visits search, Account, Advanced options, expanded settings,
  review actions, Cancel and Use model. Enter in search selects the highlighted
  candidate and focuses Use model; another explicit activation applies it.
  A typed `/model ID` opens a candidate for review rather than applying invisibly.
- **Account** is present in this dialog; it hands off to the existing **private**
  account/sign-in/default-consent flow, then returns to the chooser. If the account
  and settings are unchanged, the uncommitted candidate is retained. If account
  settings changed, models are refreshed and the operator reviews that account's
  settings. No key/code is entered in the model search or conversation composer.
- **Advanced options** starts collapsed. Thinking and Service expand inline;
  activation cycles available catalog choices and Inherit. Unknown metadata stays
  unknown; an existing explicit value is preserved rather than silently discarded.
  Service selection warns that priority can increase cost. These changes stay local
  until Use model; direct `/thinking` and `/service` selectors remain compatible.
- A model change with overrides shows **Reset overrides** and **Keep overrides**
  inline in this same dialog. Neither choice applies immediately: review it, then
  Use model. Reset atomically clears reasoning/service overrides; Keep leaves
  compatibility to runtime/provider validation. This replaces a separate
  confirmation menu for model selection.
- Conversation/owner changes refuse stale controls. Key/filter changes clear old
  row hit maps. Selecting in the dialog does not create a voyage, send a prompt,
  copy credentials, expand authority, or affect the current running turn.

## Verification

184 Helm library tests pass after retiring the old intermediate-screen tests and
adding direct-chooser coverage: centered bounds, mouse candidate-only selection,
Cancel, inline advanced/review, explicit admission, stale-owner/settings refusal,
filter/pointer invalidation and private-account handoff with retained candidate.

The actual `helm/tests/model_mouse.py` journey passes at 120×40 and 40×18 through
real Helm/Vessel/Voyage with synthetic loopback setup: one-click direct list,
centering, row selection without revision change, collapsed/expanded advanced,
Cancel preserving settings, Account and return, explicit Use model and retained
conversation draft, with no prompt/model-inference request and observed cleanup.
The main-workspace terminal journey also passed. Evidence is ignored under
`target/model-chooser/`. Catalog discovery was unavailable in that fixture; it
verifies the honest current-model fallback, not live model entitlement or every
provider catalogue. Unit tests supply multiple models and override choices.

Formatting, Python syntax and link/diff checks apply. Strict Clippy retains its
pre-existing 20 Helm warnings, with no new chooser warning after local fixes;
not a clean Clippy pass. Full workspace coverage passed **487 tests, 0 failed, 1 ignored**; lines
**39.3718%**, functions **37.3713%**, regions **38.2508%**, all nonnegative changes
from the prior record. Measured after final source edits and published in `coverage/latest.json`, with the complete current Cargo
artifact set rather than historical shared-target binaries.

The old `inference/options.rs` screen is removed, not left as a parallel engine.
The existing private-account UI is intentionally reused, not embedded in ordinary
model input. Other main-UI redesign work remains in #272. Installation and normal
command verification are recorded after observed completion; a running client
must be reopened to load new code.
