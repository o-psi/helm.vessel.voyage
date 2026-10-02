# Remote Vessel updates

Helm Web checks `execution_profiles` before sending profile requests. An older
Vessel stays connected and the setup draft remains available. The update view
explains a missing updater instead of repeatedly sending an unsupported command.

Managed Linux Vessels expose `remote_updates` and `running_release` in owner
capabilities. Only full account-owner connections can prepare, inspect, approve
or discard an update. Workspace/session execution and lifecycle grants do not
confer installation authority. Helm presents the operation; an independent
installer process on the Vessel host performs it.

## Ordinary rollback format admission and legacy limits

Ordinary self-update preparation now requires declared persistent format contracts
on the installed and candidate releases. Every candidate catalogue/journal writer
format must be readable by the previous release before installation is approved.
The exact declarations are retained separately from the legacy package ID and
rechecked before publication; old ready receipts without those pins refuse apply.
Missing declarations are unknown compatibility, not permission to migrate.

The shipped v1.0.2 Web updater invokes its installed old installer, which has no
handoff to the candidate implementation. Use the normal public `install.sh install`
or `install.sh upgrade` entry with the current verified bundle (or invoke that
bundle's current installer with its existing install/upgrade command). The current
installer's ordinary plan and owner review include the quiescent handover; it
stages the exact candidate privately and invokes its own snapshot/quarantine
implementation, never the old Vessel prepare/apply endpoints. Wizard and explicit
CLI reviews share the same checks. Inactive/unmanaged original services, migrated
old namespaces or pending uncertain operations refuse before publication rather
than gaining a speculative bootstrap. An unmodified old Web updater must be
capability-gated as unsupported; merely containing new code in its download does
not repair its execution path.

### Supported quiescent v1.0.2 handover

The shipped v1.0.2 supervisor accepts catalogue schema 1 and Voyage journal
schemas through 12. This is a format-breaking legacy handover: preparation uses
the installed trusted helper to observe its catalogue, requires a managed active
ordinary supervisor, pins its actual account namespace and refuses any owner with
unobserved cleanup, an active run, a newer journal or a protected execution binding.
Active legacy owners are never cancelled to make the update eligible.

Apply stops only reviewed gateways and the managed supervisor, fences the
supervisor and every canonical session's startup/execution locks, then takes an
exact private SQLite snapshot before any candidate migration. The candidate
supervisor is quarantined to readiness/status observations: no voyage start,
restart, browser/tool action, account enrollment or other mutation is admitted.
Enrollment and notification drivers wait for durable commit. Existing account
storage is not created or migrated by quarantined startup.

Before commit, the updater proves the authoritative ordinary admission tables,
registration projections, session journals/private resources and original account
namespace are unchanged, and all new execution/admin authority tables are empty.
Every reviewed service must expose the candidate executable; its actual account
namespace must match the pinned previous process environment. The durable
`committing` receipt precedes lifting quarantine; a stopped worker can finish
that observation through reconciliation without reinstalling or replaying effects.

On failure, the candidate and gateways must be observed stopped and ownership
locks held before the unchanged-state proof permits restoring the exact original
SQLite snapshot. No schema-version row is edited, no current registration is
silently dropped and no journal or credential is rewritten. Changed admissions,
authority, accounts or private state retain snapshot/quarantine/releases and an
unconfirmed receipt. Local rollback to legacy binaries also refuses migrated or
active state. This mechanism cannot reconstruct a pre-migration snapshot for a
host that was already migrated by an older updater; that host's retained recovery
obligation remains distinct. Native legacy update/rollback qualification is required
in addition to the synthetic proof tests.

Before effects, ordinary update reviews pin the original supervisor unit definition,
state directory, active state and enablement. Apply rechecks those pins. Rollback
restores that exact definition, starts the previous supervisor when it was
previously active (including when candidate activation left it failed/inactive),
and preserves original enablement rather than enabling a disabled service. An old
receipt without the activation pin cannot prove restoration intent.

After an activation failure, the updater rolls back the pointer once, attempts
supervisor restoration and every unchanged reviewed gateway independently, and
checks active kernel executable identities against the previous release. Only
unchanged gateway units may have their start-limit failure reset during this
approved rollback. A changed definition, readiness failure or candidate executable
leaves explicit unconfirmed obligations; pointer rollback alone is not recovery.
See [the observed gateway and rollback defect](https://github.com/o-psi/helm.vessel.voyage/issues/401).

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
EXPECTED_INSTALLATION_FINGERPRINT`. This first stops the exact managed
supervisor/gateway to fence new public admissions, then drains protected owners
and freezes their journals through their executing identities. Guardian retirement
is observed separately; supervisor stop is not cleanup evidence. It stages the
new release inactive.
`start --scope system EXPECTED_RELEASE` separately activates it. These are source
implementation paths awaiting the coordinated native verification gate.

### User installation adoption

`adopt-user prepare UUID --scope system --bin-dir ABS --execution-user ORIGINAL
--gateway-user DISTINCT --gateway-origin HTTPS --credential-key ROOT_RUN_KEY
--credential-unit PROVISIONER.service --source-credential-key ORIGINAL_RUN_KEY
--no-start` records the explicit source/account, reads and saves exact user-unit
definition hashes before effects, then claims user-owner/service quiescence.
A lost quiescence reply retains those unit pins; neither preparation nor status
repeats the uncertain stop effect. An observed host reboot establishes local process retirement; it does
not establish completion of external effects. After that reboot, `adopt-user
review UUID` returns the exact review digest. `adopt-user apply UUID DIGEST`
exports history and settings through a dropped-identity helper, interrupts retained
work without replay, publishes new ordinary bindings/incarnations and stages an
inactive root supervisor. Provider credentials remain in the original home. The
original human connection-key provisioner must make the same runtime key available;
this path neither invents nor copies provider credentials into root custody.

Application owns an original-UID supervisor lease plus one exact source-freeze
helper per voyage. Those helpers retain the original startup and execution locks
through target publication and namespace fencing. Every stage checks positive
owned-child liveness; a lost lease refuses further effects. Source authority,
catalogue and preferences must still equal the reviewed snapshot after freeze.
The bounded adoption transaction supports up to 64 sessions and ten minutes;
larger or slower handoffs refuse with retained source and preactivation recovery.
A recorded reboot alone is not the ownership fence. Review also pins the source
directory device/inode, full target manifest metadata and both named accounts;
held leases verify directory identity throughout publication and relocation.
Separate source and system filesystems are supported by a bounded opaque export
while those original-UID leases remain held. The source is atomically retained on
its own filesystem inside a root-owned container; an immutable opaque archive is
kept under root-private adoption retention. Root does not parse the old journals
or provider registry. Temporary rollback traversal is limited to the original
identity, while its retained name and the canonical source fence remain
root-controlled.
The retained source is sealed root-only before old guards release. Rollback takes
root descriptor locks as metadata before temporarily enabling traversal, then
passes those same lock descriptions into the original-UID helpers. Child closure
does not unlock the parent's retained exclusion; every failure seals the container
before the parent lock set releases. Source/session inode pins are rechecked after
rename. A staged root fence has its inode recorded before publication, so partial namespace fencing has an exact
recovery identity.

`adopt-user activate UUID DIGEST` activates reviewed root/gateway services.
`adopt-user rollback UUID DIGEST` accepts reviewed preactivation stages, including
interrupted capture/freeze/partial installation and preparatory unit effects
whose exact definitions were saved. Preparatory recovery requires observed boot
retirement before the original services are restored. It refuses admitted target
guardians or live/drifted system units, retains copied artifacts and restores the
frozen user namespace and exact reviewed user services. It holds the original
supervisor plus every startup/execution guard, verifies the retained opaque digest
before source mutation, and clears each prepared marker through its exact abort
UUID on the same retained private helper pipe. Interrupted work and withdrawn Goal
continuation stay interrupted. The exact original source inode is renamed back;
there is no reconstruction or overwrite fallback for missing/changed retention.
Read-only lookup distinguishes an absent command from unavailable/corrupt storage.
Repeating a pending rollback reconciles exact abort receipts and reversible file
cleanup. User service restoration is claimed before its effect; a lost reply is reconciled by read-only readiness observation.
Legacy human grants retain ordinary access and are permanently excluded from root
administrator authority; fresh root pairing is required. New incarnations remain
dormant until explicitly restarted, with no claim that old processes survived.

`adopt-user status UUID` observes saved state and exact helper command identities
without repeating uncertain effects. Forward continuation after partial adoption
is refused; the concrete preactivation rollback path restores the retained user
installation. Uncertain activation remains a saved source/readiness obligation
and does not permit unqualified legacy rollback. Native verification remains a
release gate; these source paths are not completed migration evidence.

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

### Source-format preservation during adoption rollback

The shipped `v1.0.2` journal schema is 12 and the current journal schema is 20.
Source handoff opens an existing schema without invoking quiescent upgrade; it
adds handoff metadata and interrupts runs while preserving that source schema and
canonical session settings/history. Only copied target journals are committed or
upgraded for the new runtime. Source catalogue export likewise uses a read-only
SQLite connection and validates an admitted reader schema. This is source analysis,
not proof that an old binary can restore a native installation. The coordinated
release verification must exercise the shipped old reader against the retained
frozen source, on both common and separate source/retention filesystems.

### Current-client admission to verified user updates

Current Helm Web requires the explicit `verified_user_updates` capability for
new ordinary-user update effects, alongside the existing owner and remote-update
authority. Old published Vessels advertised remote updates while executing their
old installed updater; a version string does not prove the newer rollback and
activation-quarantine contract. Existing operation status remains observable
without automatically applying or discarding it. An old server needs the supported
current-installer maintenance/bootstrap entry; clients must not turn presentation
into an arbitrary host executor or infer new grants to manufacture a handoff.
The feature is not advertised for the staged system/root update surface.

### Already migrated ordinary forward recovery

For an already migrated ordinary installation whose verified live candidate and
managed legacy pointer differ, see [owner-reviewed forward recovery](testing/ordinary-forward-recovery.md).
The standalone current `recover-user` entry uses a fresh operation and exact review
hash; original uncertainty stays recorded. It never replays the old updater,
restores schema1 or changes authority. Native qualification is a separate gate.

Forward recovery explicitly retains raw source hashes while comparing the complete
supported notification SQLite schema/typed rows/clock during the live-source
review interval. Only cold notification physical layout can drift before source
quiescence; every file is still accounted for. The subsequently held snapshot and
pending activation proof remain strictly raw-byte fenced. See the forward recovery
runbook for refusal cases and native qualification limits.

The explicit ordinary forward-recovery entry also handles an approved running
schema2 candidate that predates format declarations, under the narrow
[source-derived current-target admission](testing/ordinary-forward-recovery.md#approved-running-source-without-an-older-format-declaration).
It records observed formats, pins the executing current installer/target contract
and retains strict held raw proof. This does not supply absent old rollback readers,
modify historical manifests, migrate journals, restore data or broaden ordinary
update/rollback admission. Current native qualification remains a separate gate.

Forward recovery's gateway review preserves an existing credential-only drop-in
using the maintained supervisor environment allowlist. It pins its owned private
file identity/hash and exact effective/live/recovery credential namespace; unknown
or changed overrides refuse. See the [qualification boundary](testing/ordinary-forward-recovery.md#existing-credential-only-gateway-override).
