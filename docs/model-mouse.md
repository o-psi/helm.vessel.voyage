# Model modal mouse completion (#275)

> Superseded entry flow: [Direct model chooser](model-chooser.md) removes the
> intermediate Model options menu. Mouse support and centered placement remain;
> model row selection now previews, and Use model explicitly applies.

The model and inference option rows were clickable, but the model/options modals
had no wheel handling and no clickable Back/Cancel. Users still needed keyboard
navigation to complete part of a mouse-driven workflow. This delivery completes
those pointer operations without changing model/account authority.

- Click Model in the composer, then Model/Account/Thinking/Service in the options.
- Wheel inside either panel moves the selected row within bounds. It does not
  apply settings. Wheel outside the panel is consumed without changing selection.
- Model options is centered horizontally and vertically, matching modal placement
  rather than anchoring it to the bottom-left. The row/Back hit map uses that
  same centered rectangle; normal and narrow PTY checks assert equal margins.
- Options has **Back**; the model/effort/service picker has **Cancel**, including
  model-override confirmation. These close without changing settings or draft.
- Clicking a model/confirmation row retains the existing selection, validation,
  confirmation and durable command path. Keyboard behavior remains compatible.
- Choice hit maps are invalidated after input/filter/navigation and on resize;
  another activation must use a painted row. Mouse release/motion does not produce
  a spurious “not visible” status while the newly opened modal awaits its first paint.
- Private account handling remains separate; no credentials enter the composer.

## Verification

184 Helm library tests passed. New tests cover options wheel/Back, picker wheel,
stale pre-redraw clicks, Cancel and override-confirmation cancellation with no
mutation and retained draft. The existing private/stale-owner tests also pass.

`helm/tests/model_mouse.py` exercises actual Helm/Vessel/Voyage with synthetic
loopback configuration at 120×40 and 40×18: composer click, options row click,
wheel, Back, Cancel and actual model-row selection. Draft is retained; no prompt
or model-inference request is sent; fixture cleanup is observed. Evidence remains
under ignored `target/model-mouse/pty-final/`. The first selection test accidentally
clicked the underlying composer label rather than the modal row; it was corrected
to target the visibly selected row, not counted as a product click failure.

Formatting, Python syntax, links and diff checks apply. Strict Clippy retains the
previous 20 warnings and is not reported passing. Full workspace coverage passed **487 tests, 0 failed, 1 ignored**, measured after
final Rust/test edits and committed in `coverage/latest.json`; line/function/region
coverage all increased.
No native-platform, real-provider, or actual operator terminal mouse-protocol
compatibility claim follows from the Linux synthetic terminal journey. A terminal
that does not forward mouse events remains a separate environment diagnosis.

Tracked in [#275](https://github.com/o-psi/voyage/issues/275), related to the broader
[#272](https://github.com/o-psi/voyage/issues/272) UX work. The broader issue is not
closed by this fix. Installation must be observed before claiming the normal
`helm` command updated; already-running clients still need reopening.
