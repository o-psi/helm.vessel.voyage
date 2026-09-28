# Remote Vessel updates

Helm Web checks `execution_profiles` before sending profile requests. An older
Vessel stays connected and the setup draft remains available. The update view
explains a missing updater instead of repeatedly sending an unsupported command.

Managed Linux Vessels expose `remote_updates` and `running_release` in owner
capabilities. Only full account-owner connections can prepare, inspect, approve
or discard an update. Workspace/session execution and lifecycle grants do not
confer installation authority. Helm presents the operation; an independent
installer process on the Vessel host performs it.

## Review and approval

In React voyage settings, open **Vessel updates**. Choose the latest stable
release or explicitly choose the latest completed development build. Preparing
an update downloads and checks its archive, binary and browser-asset hashes,
platform loader, managed services and updater compatibility. It does not publish
binaries or restart services. Helm shows the exact version, source identity and
services before **Update this Vessel** approves that prepared release.

Stable downloads use the canonical GitHub repository's latest published stable
release. Explicit nightly updates resolve the newest public `nightly-VERSION`
prerelease and verify its archive checksum, exact source commit in `BUILD.txt`,
platform and browser-asset manifest. The installer accepts no client-supplied
URL, executable or shell command and makes no source-build fallback. The
prerelease is public, so nightly preparation requires no GitHub CLI login on the
executing host. Helm does not transfer credentials. GitHub checksums provide
integrity under repository publication authority; this does not add independent
release-signature verification.

Python 3.11+, curl and a working systemd user manager are required. Public
nightlies currently target Linux x86-64 and glibc 2.39+; unsupported hosts are
refused before installation. A managed update still requires owner review and
approval of the exact prepared artifact.

## Durable operation and reconnect

Prepare reserves a unique operation identity before launching a bounded transient
systemd user service. Approval records the exact release hash and original
installation. Repeated commands return the same receipt; an uncertain launch or
lost response never automatically reapplies the update. A prepared review expires
after one hour. Only one prepared or unresolved operation is admitted at a time.

The updater runs outside the supervisor's service so replacing Vessel does not
kill the installer. Existing transactional installation, supervisor readiness and
rollback checks preserve independent voyages. Public gateway services are included
only when their live executable belongs to this installation, their exact process
directory matches, and they use either persistent managed path: `~/.local/bin/vessel`
or `~/.local/share/voyage/install/current/bin/vessel`. Immutable release paths are
refused because restarting them would launch the old version. Their definitions
are checked again before application. No unrelated
service is restarted. Credential key-file environment drop-ins are preserved;
custom execution, loader, stop and kill overrides are refused.

Service comparison retains the configured executable, arguments and unit/drop-in
paths while excluding systemd's PID, exit-status and timestamp observations.
`Requires=voyage-vessel.service` can restart a gateway during supervisor activation;
that expected process replacement must not be mistaken for a configuration edit.

Helm retains the operation identity through reloads and polls its receipt after
reconnect. A completed receipt is followed by a fresh capability check for the same
Vessel and exact running release before setup can continue. Interrupted application
is reported as unconfirmed. A historical completed receipt whose release differs
from the current connection permits a fresh review, but does not count as verified
activation of that old release. Status checks can reconcile a stopped updater only
when the exact installed hashes and all live services are observed; otherwise it
blocks a new update pending inspection; reboot or
process loss is never described as successful completion. Private installer
receipts live under `~/.local/share/voyage/install/updates`; raw acquisition logs are never sent to Helm. Structured receipts omit internal
staging paths and use bounded failure summaries. Receipts are bounded to prevent unbounded
admission. Prepared staging is removed on discard or successful completion.

## Initial adoption

Versions released before this protocol cannot receive an update command they do
not implement. They require one remote administrator installation of an updater
capable build using the existing installer. This can be done over existing remote
management; physical access is not a product requirement. Helm reports this
boundary accurately and never turns a normal voyage into a privileged bootstrap
executor. After adoption, subsequent compatible updates use the consent flow.

The operator has deferred Tax-Axis's one-time bootstrap until remote access is
available. See [delivery issue #343](https://github.com/o-psi/helm.vessel.voyage/issues/343)
for actual deployed versions, browser checks and remaining obligations. The
protocol/UI implementation alone does not establish deployment acceptance.

## Focused verification

Run the Rust workspace coverage gate in `AGENTS.md`. Focused checks include:

```sh
cargo test -p voyage-installer -p vessel --locked remote_update -j 8
cargo test -p voyage-installer --locked -j 8
python3 installer/src/test_source_acquire.py -v
npm run test:react --prefix web
cd /path/to/webhelm && node --test tests/php.test.mjs
```

These offline checks cover authority refusal, exact approval/replay, expired or
changed installation, uncertain dispatch, worker failure, artifact provenance,
stale replies, draft retention and capability gating. Real installation,
reconnection and shared-browser acceptance are recorded separately in #343.
