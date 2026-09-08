# Vessel connections: Linux verification

Issue: [#194](https://github.com/o-psi/voyage/issues/194). Verified code revision:
`8435cfc045d60a3302378d7292259956ec31058e` (2026-09-08).

This record describes observed checks, not certification of every platform or
network. See [Vessel connections](vessel-connections.md) for operation and
[the implementation plan](vessel-connections-plan.md) for design scope.

## Build and preservation checks

- Locked builds of Helm, Vessel, Voyage and the installer passed.
- Strict Clippy across all targets of those four packages passed with `-D warnings`.
- Rust formatting and Git diff checks passed.
- Existing focused Composer, clipboard and image-attachment unit checks passed:
  **24 passed, 0 failed**. These cover marker ownership, exact pending-command
  preservation, private clipboard handling and no-replay image submission.
- A complete optimized build passed in an isolated, digest-pinned official
  Rust 1.90 / Debian 12 build environment, using the locked offline dependency
  cache. The four binaries require no glibc symbols newer than **2.34** and ran on
  the Debian 12 deployment host. The build did not replace its system libraries.

## Real terminal workflows with isolated providers

Tests used actual Helm PTYs and separate Vessel-supervised Voyage processes, not
mock screenshots. Provider fixtures were local and synthetic.

| Workflow | Observed result |
| --- | --- |
| Ordinary Helm, mouse-open Vessels, pair, review and save | Same window connected; original unsent local draft preserved exactly. |
| Masked pairing input and narrow layouts | Pairing text stayed out of the composer; keyboard and mouse controls worked in narrow terminals, including a 32×12 control-entry check. |
| Remote workspace selection and first send | Workspace selection created only a draft; first send executed on the intended remote fixture. |
| Two remote conversations | Distinct remote owners, correct responses and no local provider dispatch. |
| Quit/reopen ordinary Helm | Saved connection automatically reconnected; draft restored; no extra voyage or turn created. |
| Disconnect during remote execution | Remote work continued; local work remained usable; reconnect recovered the response without resubmission. |
| Forget and restore | Same connection UUID and private credential reference restored, without implicit revocation or replay. |
| Existing session credential import | Exact credential copied privately; source removal did not break the connection; new-conversation access remained refused. |
| Duplicate grant import | No duplicate saved connection and no authority merge. |
| Revoked access | Running manager displayed explicit revocation; no further provider dispatch occurred. |

## Admission loss and transport boundaries

A bounded forwarding fixture dropped requests or replies at explicit boundaries:

- **Start lost before admission:** original ID was durably fenced; editable text
  returned; a delayed original Start was refused. A later explicit send worked.
- **Start reply lost after admission:** existing owner recovered without another
  creation or implicit first-turn submission; explicit continuation worked.
- **Submit lost before admission:** exact-envelope resolution restored editable
  text without a provider request; a later explicit turn worked.
- **Submit reply lost after admission:** original receipt recovered with exactly
  one provider request.
- **Private terminal write loss:** the actual private terminal client detached
  without cancelling the voyage or retrying bytes. Reattachment did not replay
  input. Neither private test marker appeared in canonical history.

Transport checks rejected proxy HTTP 500, uncertain envelopes, incompatible
protocols, redirects, URL-embedded credentials and untrusted TLS certificates.
The redirect target received no credentials; the untrusted TLS server received no
HTTP request. Known expiry was distinguished from uncertain delivery.

## Server authority and storage

Observed protocol/runtime checks covered:

- Principal-bound one-time invitations and exact redemption retry; a new command
  ID or another principal could not reuse an invitation.
- Wrong-origin rejection, per-invitation failed-attempt bounds and invitation
  expiry. Original successful redemption remained recoverable after invitation
  expiry, but not after connection expiry or revocation.
- Pinned Vessel identity, approved-workspace metadata, idempotent creation,
  out-of-workspace refusal and arbitrary host-config-path refusal.
- Legacy single-session grants remaining restricted; read-only pairing unable to
  create a voyage.
- Multi-session SSE for independently running voyages.
- Connection revocation reaching running Voyage authority watchers without
  replaying provider requests.
- Stale-window registry updates refused without overwriting newer preferences.
  Unsafe registry permissions, symlinks and hardlinked credentials were refused;
  original data and immutable recovery identities were preserved.
- Supervisor creation-resolution smoke checks included reconstructed intent-only
  crash gaps, delayed requests, payload collisions and unconfirmed legacy records.
  These were persisted-fixture checks, not power-loss experiments.

Owned fixture processes and terminals were observed cleaned up. Corrected fixture
assertions were rerun rather than reported as product failures or passing checks.

## Installed deployment

The verified complete release was installed on the operator's Linux desktop and
Debian 12 remote host. The installer verified hashes and authenticated supervisor
readiness while retaining existing conversations and prior releases. The remote
loopback gateway was upgraded separately and its existing Cloudflare Tunnel used
without SSH as a Helm transport.

Observed on the actual deployment:

- Pairing and authenticated review through the installed Helm panel and public
  HTTPS endpoint.
- Ordinary installed Helm startup automatically reconnecting the saved remote,
  with no connection command-line flags.
- In-app selection of the approved remote workspace without a model request.
- Two independently created empty verification voyages through the workspace
  credential, followed by explicit cleanup of only those test voyages. The
  pre-existing remote conversation remained available.
- Authenticated workspace-grant SSE through Cloudflare with normal certificate
  validation.
- Provider sign-in remained authenticated on the executing host; credentials were
  not copied between machines. No additional paid-provider test turns were sent
  for this upgrade.

## Limits

No native macOS/Windows, full-machine reboot/power-loss, or large-scale load result
is claimed. The 32-subscription batching implementation was inspected; the runtime
SSE checks exercised multiple sessions, not a >32-session load test. Application
workspace policy is not an OS sandbox. Cloudflare evidence applies to the tested
route, not every proxy or Access configuration. Browser-based execution remains
outside this work.

## Post-release upgrade regression (#195)

The first installed check established catalogue access and new-session behavior,
but did not verify reading a pre-upgrade suspended conversation through the new
workspace credential. The user then observed that conversation as Disconnected:
its catalogue entry was reachable, while snapshot/history observation selected the
retired runtime, which did not understand the new workspace authority record.
The earlier catalogue/SSE transport checks were not sufficient evidence that this
specific old conversation's snapshot could be read.

The affected live deployment was repaired by an explicitly authorized restart of
its positively suspended owner using the installed runtime. No turn was submitted,
its UUID and two existing messages were preserved, and its history was successfully
read again after re-suspension.

The general source fix selects the supervisor's configured current runtime for
clean suspended observations and scoped stop checks. The override is ephemeral:
it does not rewrite the saved executable/incarnation, start an agent for a read,
or bypass cleanup/authority fences. Helm now separates a conversation-read error
from route-level disconnection.

A mixed-version fixture reproduced the failure with a retired runtime and a new
workspace grant. With the fix, snapshot, history, events and scoped clean-stop
checks passed without waking an execution owner, changing its stored executable
or incarnation, or adding provider requests. An actual PTY also showed Needs
attention rather than Disconnected for the read failure and displayed the original
history once the corrected supervisor was introduced. Strict all-target Helm/Vessel
Clippy and builds passed. This source fix is published for subsequent upgrades;
the immediate deployment repair did not reset pairing or require reinstalling BP.
