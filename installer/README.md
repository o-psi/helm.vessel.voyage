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
its own sibling binaries. The standalone installer asset needs an explicit full
release directory with `--bin-dir`; it does not contain the runtime binaries.

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

Run the **new release's installer** to upgrade, or give the installed installer the
new full release directory:

```sh
/absolute/new-release/bin/voyage-installer upgrade --start --dry-run
/absolute/new-release/bin/voyage-installer upgrade --start
voyage-installer rollback --start --dry-run
voyage-installer rollback --start
voyage-installer status
```

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

## The local service

The installer provisions `voyage-vessel.service` as a systemd user service. Its
unit uses an immutable release and the private Vessel state directory used by
Helm (`$XDG_STATE_HOME/voyage/vessel`, default `~/.local/state/voyage/vessel`).
The unit lives in `$XDG_CONFIG_HOME/systemd/user`, default `~/.config/systemd/user`.
These paths must be absolute, owned and safe; the state path must fit Linux's
Unix socket length limit. `--start` enables and starts the service, then checks the actual local
endpoint. `--no-start` leaves an inactive service inactive; an already active
managed service is updated while preserving its active state. Provider readiness
requires separate verification.

```sh
voyage-installer service-status
voyage-installer service-stop
systemctl --user start voyage-vessel.service
```

`KillMode=process` preserves independent voyages across supervisor restart.
Overrides that could kill descendants are refused. The installer never enables
lingering or elevates privileges. Service lifetime follows the systemd user manager;
logout/boot persistence requires separately configured lingering, and reboot still
terminates processes. Stop individual voyages through Vessel when draining work.

After stopping the supervisor, `voyage-installer service-uninstall` removes its
unit while retaining binaries, configuration and voyage data. It does not delete
surviving processes or revoke remote grants. Native macOS/Windows service
installation is unsupported; packaging those binaries is not deployment evidence.
