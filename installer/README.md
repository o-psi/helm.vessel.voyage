# Install and upgrade Voyage

The Linux installer installs a complete release: `helm`, `vessel`, `voyage` and
`voyage-installer`. With no action it opens an interactive review/apply/cancel
wizard. The review is a dry run; applying performs the displayed installation.
Provider setup and remote Vessel pairing are separate operations. Existing
configuration, credentials and voyage data are preserved.

## Install from a local release

Extract a full release into an owned directory. Run its installer next to the
other three binaries:

```sh
/absolute/release/bin/voyage-installer
```

The installer detects its sibling binaries. The standalone installer asset downloads
a published release with `upgrade`, or uses an explicit full release directory with
`--bin-dir`; it does not contain the runtime binaries. Contributors using a local
build can pass their build directory explicitly with `--bin-dir`.

The same operations are available without a terminal:

```sh
voyage-installer install --bin-dir /absolute/release/bin --dry-run --start
voyage-installer install --bin-dir /absolute/release/bin --start
```

If an old unmanaged command occupies `~/.local/bin`, review its replacement and
pass `--replace-existing`. The installer retains a backup before publishing the
managed command. It refuses unsafe paths and unexpected changes rather than
silently taking over them. Do not use sudo.

Executables are stored in private, immutable release directories. Commands in
`~/.local/bin` resolve through one release pointer, so switching versions keeps the
four programs together. Ensure `~/.local/bin` is on PATH; the installer does not
rewrite shell startup files.

## Public nightly installation

On a supported fresh Linux x86-64 computer, download and review the bootstrap,
then explicitly select the public nightly channel:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh
VOYAGE_VERSION=nightly sh install.sh install --start
```

Run as your ordinary user with glibc 2.39+, Python 3.11+, curl and a reachable
systemd user manager. No GitHub login is needed. The bootstrap pins one public
prerelease, checks its archive, manifest, binaries and source identity, then
invokes the bundled installer. `VOYAGE_VERSION=latest` is the default stable
channel; a nightly is selected only explicitly. Supported managed Vessels can
also prepare a nightly through the owner-approved [remote update flow](../docs/remote-updates.md).

## Across versions

Full archives contain a versioned `release.json` with the release label, target
and SHA-256 hash of each executable. The installer verifies that manifest before
publishing anything. A local build without a manifest is identified from its
executable versions and binary hashes. Repeating the same release is idempotent;
a different build gets its own retained directory even when the package version
string is unchanged. Unknown installation metadata schemas are refused.

`voyage-installer upgrade` resolves the latest published stable release from
`o-psi/helm.vessel.voyage` on GitHub, pins its tag, downloads the full Linux archive and verifies
its checksum, manifest, platform and four executable hashes before installation.
It does **not** reuse sibling binaries, fetch source implicitly, or fall back to
main when no release is available. An unavailable release/network error leaves the
installation unchanged and explains the explicit alternatives.

```sh
voyage-installer upgrade --start --dry-run
voyage-installer upgrade --start
voyage-installer upgrade --dev --start
voyage-installer upgrade --bin-dir /absolute/new-release/bin --start
voyage-installer rollback --start --dry-run
voyage-installer rollback --start
voyage-installer status
```

`--dev` selects the latest public nightly prerelease, pins its published source
commit and downloads its full archive and checksum. It does not fetch source or
run Cargo on the installation host. The review identifies the exact nightly version,
source commit and archive hash. It conflicts with `--bin-dir` and is only supported
for Upgrade. Explicit local installation still supports trusted local builds without
a release manifest. Reinstalling the same verified binary identity reports
**Already current**, separately from service work.

Automatic upgrades require Linux, Python 3.11+ and curl. Published stable and
nightly downloads currently support Linux x86-64 with glibc 2.39+. Private stable
releases use an existing authenticated GitHub CLI (`gh auth login`) on this machine;
public nightly downloads need no login. Credentials are never requested in installer
input or embedded in command arguments. Provider credentials are not supplied to
acquisition subprocess environments. No provider login or model inference is performed.

`--dry-run` downloads into private temporary staging under
`~/.cache/voyage/upgrades` to review an exact artifact, but does not publish binaries
or change services. Network commands have five-minute bounds, the overall acquisition
is bounded, logs are bounded to 64 MiB, and release download/extraction limits
are 512 MiB/1 GiB. Ctrl+C/termination requests stop acquisition process groups
before staging cleanup. Once publication begins, the bounded install/service
transaction finishes instead of interrupting between journal updates. Forced termination or unconfirmed cleanup can retain
staging; reported retained work must be stopped before removing its directory.

The wizard defaults Upgrade to stable published releases; **d** selects the public
nightly prerelease when Upgrade is selected. An explicit `--bin-dir` fixes
the local source. Preparation happens outside the display thread; review retains
the exact prepared artifact through apply, without resolving latest/nightly again.
Esc/Ctrl+C during preparation cancels it and waits for subprocess cleanup.

**Close and relaunch Helm after upgrading.** An installer cannot replace the code
inside your already-running TUI. Existing independent voyages retain their running
processes and do not automatically move to the new runtime binary.

Saved configurations from before removal of the external Codex bridge may contain
`sandbox.bridge_read`, `sandbox.bridge_inherit_env` and `sandbox.bridge_network`.
The current configuration reader accepts and discards these retired fields; they
grant no access and are omitted when configuration is serialized again. Other
unknown sandbox fields remain errors. Releases that removed the fields without
this compatibility handling can read conversation history but fail suspended
state inspection and reattachment. Upgrade the runtime rather than editing saved
journals or clearing cleanup evidence; the bridge itself remains unsupported.

Rollback selects the recorded previous verified release. It switches binaries and
the managed supervisor unit; it does not reverse session data or configuration
changes. Older executables must still support that data and live process protocol.
Retain old releases while voyages may use them. No uncertain work is replayed.

A private installation journal and lock protect publication and allow interrupted
operations to be retried. Service activation failures are reported and the previous
installation is restored when possible; a first-install failure retains files for
inspection and retry. Read the actual error if restoration cannot be completed.

## Download a published version

The bootstrap resolves `latest` to one exact release tag, downloads that full
platform archive and its checksum over HTTPS, verifies the archive and rejects
unsafe entries before running the bundled installer:

```sh
sh install.sh
VOYAGE_VERSION=v1.0.2 sh install.sh install --start
VOYAGE_RELEASE_DIR=/absolute/extracted-release sh install.sh install --start
```

With **no arguments**, the shell bootstrap selects `install --start`, including
when piped to `sh`; it does not open the wizard. Explicit arguments are preserved:
use `sh install.sh install --no-start` to leave an inactive service inactive, or
`sh install.sh install --dry-run` to validate without publication. Run the bundled
`voyage-installer` with no arguments for the interactive review/apply/cancel wizard.

Before installation, the bootstrap rejects root, requires Python 3.11+ and checks
that the systemd user manager responds within ten seconds. Published downloads
additionally require curl, Linux x86-64 and glibc 2.39+ (not musl/Alpine). Trusted
local source overrides may use another libc baseline or Linux ARM64. The
bootstrap resolves the canonical `o-psi/helm.vessel.voyage` repository for stable
and public nightly downloads.

After successful explicit installation (not dry-run), the bootstrap prints:
```sh
export PATH="$HOME/.local/bin:$PATH"
cd /path/to/your/project && helm
```
Replace the project placeholder; no shell startup file is modified.

The pinned example selects v1.0.2; omit `VOYAGE_VERSION` to resolve latest. Download setup
requires published GitHub release assets, `curl` and Python 3.11 or later. Local
release setup works before publication. Checksums establish integrity against the
HTTPS-delivered manifest, not independent release signing. Temporary downloads are
private and cleaned up; installed releases remain independent of that directory.
`VOYAGE_INSTALLER_VERSION` is retained as a version-selector alias;
`VOYAGE_INSTALLER_BIN` can select a local installer with sibling runtime binaries.
The bootstrap passes its pinned bundle explicitly with `--bin-dir`, so an upgrade
does not resolve latest a second time. Explicit `--bin-dir` is honored;
`VOYAGE_RELEASE_DIR` and `VOYAGE_INSTALLER_BIN` conflict with `--dev`. For a
development archive, use `sh install.sh upgrade --dev --start` or
`VOYAGE_VERSION=nightly sh install.sh upgrade --start`. The bootstrap downloads a
public prerelease and passes its verified binaries directly to the bundled installer.
When no published release is available, provide an explicit trusted local release
directory.
Publication to GitHub main is not publication of a downloadable release.

## The local service

### Read-only system host assessment

`voyage-installer system-assess --execution-user USER --gateway-user USER` reports Linux systemd,
effective UID/capabilities and UID/GID mapping, an explicitly named execution
execution and gateway accounts and their groups, distinct UIDs, the selected system paths and mount writability, and
existing user/system installation markers. It prints bounded JSON with a
`ready_to_install: false` result and specific blockers. This command makes no
filesystem or service changes and can run without root to identify prerequisites.
It does not prove the account can access a workspace or that namespace root has
host authority. A system installation, unprivileged gateway, scope-aware updater
and adoption procedure are still being implemented under
[#344](https://github.com/o-psi/helm.vessel.voyage/issues/344); do not run the
ordinary user installer with sudo to approximate one. The staged system unit
contract pins immutable binaries and separates a root supervisor from an ordinary
gateway, but the installer does not yet publish or activate those units.
The system release stager is also staged behind this boundary: it requires
root-owned `/opt/voyage/releases` paths, verifies the exact manifest and hashes,
and makes the pinned binaries and browser worker readable by ordinary execution
identities. It is not wired to the installer CLI, service activation or rollback.

### User service

The installer provisions `voyage-vessel.service` as a systemd user service. Its
unit uses an immutable release and the private Vessel state directory used by
Helm (`$XDG_STATE_HOME/voyage/vessel`, default `~/.local/state/voyage/vessel`).
The unit lives in `$XDG_CONFIG_HOME/systemd/user`, default `~/.config/systemd/user`.
These paths must be absolute, owned and safe. `--start` enables and starts the
service, then checks the actual authenticated loopback HTTP endpoint. `--no-start`
leaves an inactive service inactive; an already active
managed service is updated while preserving its active state. Provider readiness
requires separate verification.

```sh
voyage-installer service-status
voyage-installer service-stop
systemctl --user start voyage-vessel.service
```

An active voyage finishes its current turn on its existing runtime. Once terminal
state and cleanup are confirmed, it checkpoints and exits; the next turn starts
from the updated Vessel runtime binary with the same session and history. Helm
shows successful completion as Finished for 24 hours, then Settled. Open Helm
windows still need reopening to use the new Helm binary. Pre-existing runtimes
from releases without turn suspension require one explicit clean restart to adopt
this behavior.

For an active-service upgrade, the installer reads the existing catalogue through
the running release's trusted Vessel helper before replacement. After restart it
uses the new helper to verify readiness and preservation of previously live owner
identities. This permits a public API cutover without requiring the new client to
speak the retired service API; failed readiness retains the existing rollback path.

`KillMode=process` preserves independent voyages across supervisor restart.
Overrides that could kill descendants are refused. The installer never enables
lingering or elevates privileges. Service lifetime follows the systemd user manager;
logout/boot persistence requires separately configured lingering, and reboot still
terminates processes. Stop individual voyages through Vessel when draining work.

After stopping the supervisor, `voyage-installer service-uninstall` removes its
unit while retaining binaries, configuration and voyage data. It does not delete
surviving processes or revoke remote grants. Native macOS/Windows service
installation is unsupported; packaging those binaries is not deployment evidence.

## Updates from Helm Web

Managed Linux installations support the owner-approved [remote update flow](../docs/remote-updates.md). The updater prepares a pinned stable or explicitly selected nightly artifact, retains a durable operation receipt and runs outside the Vessel service during replacement. Pre-updater releases require one remote administrator bootstrap. Existing provider credentials and independent voyages are preserved.
