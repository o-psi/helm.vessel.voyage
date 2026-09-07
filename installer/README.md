# Install and upgrade Voyage

The Linux installer installs a complete release: `helm`, `vessel`, `voyage` and
`voyage-installer`. With no action it opens an interactive review/apply/cancel
wizard. The review is a dry run; applying performs the displayed installation.
Provider setup and remote enrollment are separate operations. Existing
configuration, credentials and voyage data are preserved.

## Install from a local release

Build the workspace or extract a full release into an owned directory. Run its
installer next to the other three binaries:

```sh
/absolute/release/bin/voyage-installer
```

For a repository build, use `target/release/voyage-installer`. The installer detects
its own sibling binaries. The standalone installer asset can download a published release with `upgrade`,
or use an explicit full release directory with `--bin-dir`; it does not contain
the runtime binaries.

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

## Across versions

Full archives contain a versioned `release.json` with the release label, target
and SHA-256 hash of each executable. The installer verifies that manifest before
publishing anything. A local build without a manifest is identified from its
executable versions and binary hashes. Repeating the same release is idempotent;
a different build gets its own retained directory even when the package version
string is unchanged. Unknown installation metadata schemas are refused.

`voyage-installer upgrade` resolves the latest published stable release from
`o-psi/voyage` on GitHub, pins its tag, downloads the full Linux archive and verifies
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

`--dev` explicitly fetches GitHub `main` into a fresh private checkout, pins the
fetched commit and builds the locked workspace with stable Rust (two build jobs).
The review and installed manifest identify `dev-FULL_COMMIT`; unchanged Cargo
version strings do not imply unchanged binaries. It never reuses your checkout or
its potentially stale `target/release` directory. `--dev` conflicts with
`--bin-dir` and is only supported for Upgrade. Explicit local installation keeps
supporting trusted local builds without a release manifest. Reinstalling the same
verified binary identity reports **Already current**, separately from service work.

Automatic upgrades require Linux x86-64/aarch64, Python 3.11+ and curl. Development
builds additionally require Git, stable Rust/Cargo, native build tools and network
access to dependencies. Private repositories/releases use an existing authenticated
GitHub CLI (`gh auth login`) on this machine; credentials are never requested in
installer input or embedded in command arguments. Public sources work without gh. Unattended Git credential prompts and ambient Git configuration are
disabled except for the explicit GitHub CLI credential helper; development builds use a fresh Cargo home rather than local credentials
or Cargo configuration. Build scripts execute code from main as your account:
**this is not a sandbox**. Provider credentials are not supplied to acquisition
subprocess environments. No provider login or model inference is performed.

`--dry-run` downloads/builds into private temporary staging under
`~/.cache/voyage/upgrades` to review an exact artifact, but does not publish binaries
or change services. Network commands have five-minute bounds, compilation has a
one-hour bound, logs are bounded to 64 MiB, and release download/extraction limits
are 512 MiB/1 GiB. Ctrl+C/termination requests stop acquisition process groups
before staging cleanup. Once publication begins, the bounded install/service
transaction finishes instead of interrupting between journal updates. Forced termination or unconfirmed cleanup can retain
staging; reported retained work must be stopped before removing its directory.

The wizard defaults Upgrade to published releases; **d** toggles explicit
main development builds when Upgrade is selected. An explicit `--bin-dir` fixes
the local source. Preparation happens outside the display thread; review retains
the exact prepared artifact through apply, without resolving latest/main again.
Esc/Ctrl+C during preparation cancels it and waits for subprocess cleanup.

**Close and relaunch Helm after upgrading.** An installer cannot replace the code
inside your already-running TUI. Existing independent voyages retain their running
processes and do not automatically move to the new runtime binary.

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
VOYAGE_VERSION=v0.1.0 sh install.sh upgrade --start
VOYAGE_RELEASE_DIR=/absolute/extracted-release sh install.sh install --start
```

The version above is an example, not a claim that it is published. Download setup
requires published GitHub release assets, `curl` and Python 3.11 or later. Local
release setup works before publication. Checksums establish integrity against the
HTTPS-delivered manifest, not independent release signing. Temporary downloads are
private and cleaned up; installed releases remain independent of that directory.
`VOYAGE_INSTALLER_VERSION` is retained as a version-selector alias;
`VOYAGE_INSTALLER_BIN` can select a local installer with sibling runtime binaries.
The bootstrap passes its pinned bundle explicitly with `--bin-dir`, so an upgrade
does not resolve latest a second time. Explicit `--bin-dir` is honored;
`VOYAGE_RELEASE_DIR` conflicts with `--dev`. `sh install.sh upgrade --dev` first
needs a published installer supporting `--dev` (or an explicit
`VOYAGE_INSTALLER_BIN`); it then builds main rather than installing that bundle.
When no release is published, bootstrap a local installer from source first.
Publication to GitHub main is not publication of a downloadable release.

## The local service

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

`KillMode=process` preserves independent voyages across supervisor restart.
Overrides that could kill descendants are refused. The installer never enables
lingering or elevates privileges. Service lifetime follows the systemd user manager;
logout/boot persistence requires separately configured lingering, and reboot still
terminates processes. Stop individual voyages through Vessel when draining work.

After stopping the supervisor, `voyage-installer service-uninstall` removes its
unit while retaining binaries, configuration and voyage data. It does not delete
surviving processes or revoke remote grants. Native macOS/Windows service
installation is unsupported; packaging those binaries is not deployment evidence.
