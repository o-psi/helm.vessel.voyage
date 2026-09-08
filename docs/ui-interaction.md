# Helm form and dialog interaction

The private Vessels panel uses `ratatui-interact`'s typed `FocusManager<Button>`
for its form and confirmation scopes. It stores control identities only. The
existing zeroizing private fields, authenticated connection identity checks,
rendered mouse targets and command actions remain owned by Helm.

Pairing starts at the endpoint, then Tab visits invitation, alias, automatic
reconnect, validate, switch to import, and Back. Import omits the invitation.
Shift-Tab reverses the cycle. Rename visits alias, save and Back. Enter in a
field submits; Enter or Space on a focused action activates it. Clicking a field
selects that same control for keyboard entry. Focused rows scroll into view,
including at narrow terminal sizes. Paste on an action is consumed without
editing a field. Ctrl-U clears the focused field.

Access previews and Forget confirmations start on Back. Select the Save/Confirm
action with Tab and activate it, click it, or use the existing explicit `s`
shortcut. Enter alone on a freshly opened confirmation returns to the list.
While a setup operation is busy only Back is focusable; closing clears private fields but
retains the pending operation and its recovery identity. Resize, page/input
transitions and registry reload invalidate old mouse targets.

Private-panel input still precedes clipboard acquisition, draft composers and
other dialogs, including bracketed paste. Ctrl-C/Ctrl-Q retain their application
quit behavior. This scope does not intercept the main composer's completion Tab,
voyage traversal, draft first-send freeze, terminal ownership or interaction
responses. It does not send a message, broaden execution authority, or create a
voyage when changing focus.

## Framework comparison and decision

The comparison used Helm's actual pairing/import/rename forms and private
preview/Forget dialogs, alongside the published APIs and source packages.

| Candidate | Fit to Helm's current implementation | Decision |
| --- | --- | --- |
| `rat-focus` 2.1.1 with the `rat-widget` ecosystem | Per-widget focus flags and `HasFocus`/`FocusBuilder` provide hierarchy, navigation policies and coordinate focus. Helm would need to attach flags/areas to its existing row action model; adopting widget input states would additionally require reconciling private zeroization and existing field bounds. | Good for a widget-owned tree, but that machinery is unnecessary for these existing action rows. Do not add a second focus framework. |
| `ratatui-interact` 0.5.3 | `FocusManager<T>` accepts Helm's existing typed action identities and supplies registration, exact selection and wrapping next/previous traversal. It need not own field contents, rendering, mouse routing or the application's event loop. | Use only this focus primitive, scoped to private forms and confirmations. Other widgets and its clipboard facilities are not adopted. |

Sources: [rat-focus API](https://docs.rs/rat-focus/2.1.1/rat_focus/),
[rat-widget API](https://docs.rs/rat-widget/3.2.1/rat_widget/),
[ratatui-interact FocusManager](https://docs.rs/ratatui-interact/0.5.3/ratatui_interact/state/struct.FocusManager.html).
The source packages' `Cargo.toml`, `rat-focus/src/builder.rs` and
`ratatui-interact/src/state/focus.rs` were also inspected. This is an integration
comparison, not a performance comparison between frameworks.

## Dependency and validation boundaries

`ratatui-interact` 0.5.3 is MIT licensed and declares Rust 1.85 / edition 2024.
Helm disables its default features (the published default list is empty).
Clipboard (`arboard`), filesystem, markdown (`termimad`) and theme serialization
features are disabled. Its normal dependencies include Crossterm 0.29, Ratatui
0.30, regex, thiserror and unicode-width. The upstream Ratatui dependency enables
Ratatui defaults plus `unstable-rendered-line-info`: disabling interact's defaults
does not disable those transitive features. The resolved addition includes
`ratatui-macros`; Ratatui core and Crossterm remain shared with Helm. No clipboard
backend, network runtime, build script or new native library is introduced by the
selected focus integration.

`rat-focus` 2.1.1 is MIT/Apache-2.0 licensed, edition 2024, and does not declare a
`rust-version` in its published manifest. Its dependencies include rat-event,
ratatui-core, ratatui-crossterm, fxhash and log. It was inspected but not added.

The temporary `epic196_private_form_focus` probe passed against the actual
`Manager::input` implementation using synthetic terminal events: private paste, action/field separation,
reconnect toggle, import omission and clearing, forward/reverse wrapping, mouse
field identity, resize invalidation, pending freeze, closing while retaining the
pending operation, and confirmation Back behavior. It ran alongside the actual
transcript probe with `HELM_COLOR=never cargo test -p helm --locked --lib epic196
-- --nocapture`; both passed. The temporary module was removed after verification,
with evidence retained outside the repository and recorded in issue #196. This
does not recreate a repository test suite, verify an actual terminal display,
or establish native macOS/Windows terminal behavior.
