# Bounded model catalogue loading (#277)

The direct chooser could remain **loading** if account discovery stalled or a
route/generation-bound reply was discarded. Account-model reads had no local UI
deadline and spawned work was not owned by the chooser. This is a confirmed
loading-state defect; it does not establish why a particular real provider stalled.

## Fix

All model discovery paths now share one owned request with a 12-second deadline.
A timeout, missing/setup-failed account or stale response settles the loading state.
The chooser keeps the current model and unsent conversation draft available and
shows a **Retry** action. Retry is an explicit fresh catalogue read, not inference,
tool replay or automatic provider retry. It cancels the prior read and replaces
its ID. Timed-out IDs are retired so late replies cannot repopulate the chooser.
Closing/changing the picker and private account handoff cancel owned work; shutdown
awaits retired tasks using existing UI cleanup accounting.

A UI watchdog is necessary in addition to the transport timeout: a reply discarded
by stale-route filtering must still stop the spinner. It does not bypass route or
account identity checks, automatically switch accounts, or interpret missing
catalogue data as unsupported models. Use model keeps its existing validation path.

## Verification

186 Helm library tests passed, including forced deadline, missing reply, late reply,
retry-ID replacement, failure settlement and retained draft/no command admission.
Existing direct chooser and account tests pass.

`helm/tests/model_loading.py` holds an actual loopback `/models` response beyond the
deadline through real Helm/Vessel/Voyage. Loading visibly starts, then stops after
approximately 12 seconds; the current model stays available. Releasing the server
and clicking Retry displays an additional catalogue model. Cancel retains the draft;
no prompt or model-inference request is sent. Fixture cleanup is observed. The
normal/narrow direct chooser regression also passes. Evidence is ignored under
`target/model-loading/`. An earlier fixture only delayed the first read and did
not reproduce the stuck state; it was strengthened to hold every read until the
operator Retry step, not counted as stalled-request evidence.

Full workspace coverage passed **489 tests, 0 failures, 1 ignored**, with small
positive line/function/region changes. Measured after final source/test edits and committed
in `coverage/latest.json`, selecting current Cargo artifacts. Formatting, Python
syntax, documentation links and diff are checked. Strict Clippy retains the prior
20 warnings and is not a clean pass. No real-provider credential, paid usage,
native-platform or operator-session catalogue success is claimed.

Tracked in [#277](https://github.com/o-psi/voyage/issues/277), related to
[#276](https://github.com/o-psi/voyage/issues/276) and broader
[#272](https://github.com/o-psi/voyage/issues/272). Installation must be observed
before claiming the normal command updated; existing Helm clients need reopening.
