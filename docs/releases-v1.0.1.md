# v1.0.1 — Linux x86-64

This Linux binary release contains `helm`, `vessel`, `voyage`, and
`voyage-installer`, all version 1.0.1. Helm is the terminal client, Vessel
supervises independent voyage processes, and disconnecting Helm does not cancel
a voyage. The web application is not part of these binary assets.

## Install

Use the assets on the [v1.0.1 release page](https://github.com/o-psi/voyage/releases/tag/v1.0.1).
Download the full `voyage-v1.0.1-x86_64-unknown-linux-gnu.tar.gz` archive and its
`.sha256` file into the same directory. Before extracting:

```sh
sha256sum -c voyage-v1.0.1-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf voyage-v1.0.1-x86_64-unknown-linux-gnu.tar.gz
cd voyage-v1.0.1-x86_64-unknown-linux-gnu
./bin/helm --version
./bin/voyage-installer --bin-dir "$PWD/bin"
```

The installer opens a review/apply/cancel wizard. It installs a versioned release
and provisions a systemd user service. Run as your ordinary user, **not sudo**.
The explicit CLI alternative is `./bin/voyage-installer install --bin-dir
"$PWD/bin" --no-start`; use `--start` only when ready to enable/start the service.
Never replace an existing running deployment without reviewing its upgrade plan.
Add `$HOME/.local/bin` to PATH yourself; shell startup files are not modified.

The standalone `voyage-installer-x86_64-unknown-linux-gnu.gz` contains only the
installer, not the three runtime binaries. It downloads the latest stable full
release when no local `--bin-dir` is supplied. The source bootstrap supports
`VOYAGE_VERSION=v1.0.1` for a pinned release. See the
[installer guide](../installer/README.md) and [first task](getting-started.md).

## Scope and limitations

- Linux x86-64 GNU/glibc only; the native build requires **glibc 2.39 or newer**,
  with libgcc_s and the GNU ELF loader. Verification ran on glibc 2.44. No ARM64, musl/Alpine, macOS or Windows binary
  support is claimed. Actual published binary libc requirements are recorded in
  the GitHub release verification notes; source builds can use a different libc
  baseline.
- Managed installation needs a running systemd user manager, Python 3.11+,
  download utilities and ordinary-user private storage. Direct archive commands
  can be used without installing a service.
- Provider accounts and budget are separate. No model credits or credentials are
  included. ChatGPT subscription integration remains experimental; release
  verification does not certify paid-provider access.
- Checksums detect changed bytes but are not independent publisher signatures.
  No production signing trust root or reproducible-build certification is claimed.
- Offline Linux checks are not reboot/logout persistence, native-platform or
  production public-proxy certification. Application policy is not an OS sandbox.
- Existing conversations/configuration are not release assets. Back them up
  privately before upgrades; never publish them with diagnostic reports.

The GitHub release notes and issue #305 record the exact measured source, checks,
coverage and remaining platform limitations. Historical checks are not evidence
for a different build. See [current behavior](current-state.md) for feature scope.

## Maintainer install check

`python3 packaging/verify_linux_install.py --release-dir /absolute/extracted-release`
runs the actual release installer and installed commands in private Bubblewrap
user/mount/PID namespaces. Its fail-closed fixture replaces `/usr/bin/systemctl`
only inside that namespace. It verifies dry-run, real file installation, repeated
upgrade, binary hashes/version and generated unit content, not service activation.
After publication, add `--hosted` to check public HTTPS pinned/default bootstrap
and the installed standalone installer's latest-release acquisition. This needs
Bubblewrap, network/DNS and CA certificates; it supplies no host credentials.
