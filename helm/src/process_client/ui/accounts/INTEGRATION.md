# Helm account UI integration — #213 completion handoff

This delivery owns `helm/` only. It includes parent protocol commit `14e88a0`
(owner-local optional `config_path` for account creation/resolution, plus an
enrollment-only connection catalogue) as a prerequisite, not a second protocol
implementation. The parent owns runtime changes, Cargo builds, tests, workspace
coverage measurement and final publication. No Cargo command was run in this
worktree; rustfmt and diff checks are source checks, not passing tests.

## Integrated workflows

- `/account` and the composer control share the keyboard/mouse account picker.
  Confirmation stages exact atomic account/model/override commands, keeps current
  runs unchanged, preserves composer text, and keeps remembered new-voyage choices
  separate from host defaults. An unavailable remembered identity never falls back.
- Local configured drafts call the existing frontend `launch::persist` using
  `Saved::launch_config()`, restoring retained policy override/profile metadata.
  `StartAccount` persists that exact trusted JSON path before dispatch;
  `ResolveStartAccount` recovers it unchanged. Remote creation accepts no local
  configuration path. Neither path resolves credentials on Helm.
- `Client::connection_state()` supplies a view-owned watch. `loss_generation`
  invalidates the request/view identity, drops queued one-shots and clears private
  material even when loss and reconnect coalesce between UI ticks. Rendering also
  gates on the current counter. Automatic polling pauses after loss. Explicit R
  privately inspects the original enrollment under current authority; reconnect
  never replays `EnrollAccount`. Closing drops the private receiver and view.
- Enrollment-only callers can use the connection catalogue without account labels;
  missing/denied defaults do not block adding an authorized device account.
- Device responses remain view-owned (not `Update`, public events, diagnostics,
  conversation or serialized drafts). Expiry, denial, cancellation, completion,
  closure and socket loss remove display material. Empty/malformed codes and URLs
  outside the exact native provider flow are never displayed/opened.
- O is the sole explicit browser action. It passes only a fixed provider URL to
  `xdg-open`/`open` with null standard streams. One launcher is allowed at a time;
  it is polled/reaped and terminated after ten seconds if still pending. This
  bounds the launcher, not the user's browser navigation. Other platforms show
  manual-navigation guidance. No real browser/provider is used by test fixtures.
- Connection selection now supports mouse hits and keyboard scrolling; small
  layouts consume private input without exposing codes. Ctrl+C/Q still exits.

## Focused synthetic App tests for parent execution

`accounts/app_tests.rs` exercises actual App input/render/reply/persistence paths:
keyboard and mouse confirmation, exact pending recovery envelopes, retained current
run and composer, isolated saved-state scanning for private material, prompt-history
and conversation exclusion, explicit browser action, empty codes, expiry, denied
reads, repeated cancel identity, publication winning cancellation, closure, narrow
layout, coalesced disconnect/reconnect with queued and late stale replies, explicit
private reopening, enrollment-only connection selection, configured first-send
policy/path preservation, remembered defaults and unavailable binding refusal.

Fixtures use per-thread temporary storage (no HOME/environment mutation) and a
browser effect substitute. Synthetic clients point to nonexistent local Vessel
files; there are no provider calls. Existing helper-level tests additionally cover
legacy settings deserialization, selection availability, aliases and safe intents.

The parent must compile/test integrated consumers and publish workspace coverage
before treating the Rust delivery as verified. Real-provider workflows and native
macOS/Windows security/launcher behavior are not established by these source checks
or Linux synthetic tests. GitHub CLI authentication was unavailable here; parent
must record actual test outcomes and remaining full-issue scope in #213.
