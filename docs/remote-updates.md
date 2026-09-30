# Remote Vessel updates

Helm Web checks `execution_profiles` before sending profile requests. An older
Vessel stays connected and the setup draft remains available. The update view
explains a missing updater instead of repeatedly sending an unsupported command.

Managed Linux Vessels expose `remote_updates` and `running_release` in owner
capabilities. Only full account-owner connections can prepare, inspect, approve
or discard an update. Workspace/session execution and lifecycle grants do not
confer installation authority. Helm presents the operation; an independent
installer process on the Vessel host performs it.

## Channel selection and approval

In Helm Web, open **Manage Vessels**, then the selected Vessel's details. The
update view defaults to the installed release channel and loads the latest
published stable and development versions asynchronously. Selecting a channel
shows its latest version. One **Update this Vessel** click approves preparation
and installation of that displayed channel and version. The button is disabled
when the selected channel's installed version is current or newer.

Preparation downloads and checks the archive, binary and browser-asset hashes,
platform loader, managed services and updater compatibility. It does not publish
binaries or restart services. Helm automatically applies only when the Vessel's
prepared receipt has the same operation, channel and version as the click, a
valid release hash and an unexpired approval window. A different or invalid
receipt stops before installation. An expired previous preparation is discarded
and replaced within the same click. The click's approval remains in page memory;
after a reload, Helm observes the saved operation without automatically
reapproving it. An uncertain apply reply is never replayed automatically.

Stable downloads use the canonical GitHub repository's latest published stable
release. Explicit nightly updates resolve the newest public `nightly-VERSION`
prerelease and verify its archive checksum, exact source commit in `BUILD.txt`,
platform and browser-asset manifest. The installer accepts no client-supplied
URL, executable or shell command and makes no source-build fallback. The
prerelease is public, so nightly preparation requires no GitHub CLI login on the
executing host. Helm does not transfer credentials. GitHub checksums provide
integrity under repository publication authority; this does not add independent
release-signature verification.

Python 3.11+, curl and the applicable systemd manager are required (user manager
for user installations; system manager for explicit system scope). Public
nightlies currently target Linux x86-64 and glibc 2.39+; unsupported hosts are
refused before installation. A managed update still requires owner approval of
the displayed version and Vessel verification of the exact prepared artifact.

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

## Explicit Linux system scope

A managed root supervisor now delegates owner-approved operations to
`voyage-installer remote-update system`. System receipts and acquisition staging
are root-private under `/var/lib/voyage/install/system-updates`; they are separate
from user updater receipts. A system invocation never falls through to the user
service manager or root's login account. Acquisition uses an explicit private
staging home, fixed executable search paths and public GitHub HTTPS for both
channels, without consulting inherited GitHub credentials.

Preparation pins the installed release, exact root/gateway unit definitions,
configured ordinary accounts, external credential provisioner and protected
default identity/runtime layout. Downloaded loader and updater compatibility
checks run in a bounded transient systemd service with a dynamic unprivileged
identity, homes hidden, private networking and devices, no capabilities and no
privilege gain. Candidate code is not run as root before exact approval.

Application launches a separate root system-manager updater with a bounded
lifetime. It rechecks the approved archive hashes and installation fingerprint,
then invokes the reviewed system lifecycle transaction under its installation
lock. Root/gateway services and private state retain their configured identities;
independent voyages remain outside the supervisor lifetime. Exact approval or
application retries observe the same receipt and never start another updater.
Interrupted preparation/application remains a saved obligation requiring host
inspection; reconnect does not automatically replay an uncertain effect.

Active rollback requires both archives to contain the same strict, code-owned
compatibility contract. The contract enumerates the actual catalogue/journal
reader and writer versions and hashes their implementation plus build inputs.
The verifier rejects missing contracts, unsupported formats, changed core code
or mismatched contracts before stopping services. A qualified activation failure
restores the exact retained units/source and observes readiness; it never restores
a state backup while independent voyages write. A stopped updater is reconciled
against exact reviewed candidate or previous source and readiness without replay.

Older system archives without the contract require an explicit host-operator
`adopt-update-contract --scope system --bin-dir ABS EXPECTED_RELEASE
EXPECTED_INSTALLATION_FINGERPRINT`. This drains protected owners, freezes their
journals through their executing identities and stages the new release inactive.
`start --scope system EXPECTED_RELEASE` separately activates it. These are source
implementation paths awaiting the coordinated native verification gate.

### User installation adoption

`adopt-user prepare UUID --scope system --bin-dir ABS --execution-user ORIGINAL
--gateway-user DISTINCT --gateway-origin HTTPS --credential-key ROOT_RUN_KEY
--credential-unit PROVISIONER.service --source-credential-key ORIGINAL_RUN_KEY
--no-start` records the explicit source/account and requests user-owner/service
quiescence. An observed host reboot establishes local process retirement; it does
not establish completion of external effects. After that reboot, `adopt-user
review UUID` returns the exact review digest. `adopt-user apply UUID DIGEST`
exports history and settings through a dropped-identity helper, interrupts retained
work without replay, publishes new ordinary bindings/incarnations and stages an
inactive root supervisor. Provider credentials remain in the original home. The
original human connection-key provisioner must make the same runtime key available;
this path neither invents nor copies provider credentials into root custody.

`adopt-user activate UUID DIGEST` activates reviewed root/gateway services.
`adopt-user rollback UUID DIGEST` is restricted to the dormant preactivation stage
and restores the retained frozen user namespace and exact reviewed user services.
Legacy human grants retain ordinary access and are permanently excluded from root
administrator authority; fresh root pairing is required. New incarnations remain
dormant until explicitly restarted, with no claim that old processes survived.

`adopt-user status UUID` observes saved state and exact helper command identities
without repeating uncertain effects. Interrupted multistage adoption currently
retains a host-operator recovery obligation; automatic or phase-resumable recovery
across partial install/import/fencing is not implemented. This is remaining release
acceptance, alongside native verification, rather than a completed migration claim.

## Initial adoption

Versions released before this protocol cannot receive an update command they do
not implement. They require one remote administrator installation of an updater
capable build using the existing installer. This can be done over existing remote
management; physical access is not a product requirement. Helm reports this
boundary accurately and never turns a normal voyage into a privileged bootstrap
executor. After adoption, subsequent compatible updates use the consent flow.

The operator moved Tax-Axis's one-time bootstrap to v1.1.0. See [delivery issue #343](https://github.com/o-psi/helm.vessel.voyage/issues/343)
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
