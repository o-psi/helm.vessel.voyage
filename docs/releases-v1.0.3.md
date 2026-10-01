# v1.0.3 Linux x86-64 candidate

**Verification pending; no stable v1.0.3 release is published.** The checked-in
workspace and four executable versions are 1.0.3 on main.
Stable `latest` still selects the shipped v1.0.2 release until explicit publication.
The exact source, checks and remaining obligations are tracked in
[release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375) and
[integration PR #382](https://github.com/o-psi/helm.vessel.voyage/pull/382).

## Candidate contents

The full archive contains `helm`, `vessel`, the independent `voyage` runtime and
`voyage-installer`, plus the pinned Voyage-owned browser worker under
`share/voyage/browser`. Helm disconnect does not cancel a voyage. Helm Web is
delivered and verified separately in
[Web integration PR #10](https://github.com/o-psi/webhelm/pull/10).

Source changes include ordered live events, persistent Voyage-owned Goals, browser
grounding and shared full-site viewing/control, and client UX updates. The current
release milestone is **v1.0.3 — Live Events and Host Browser**.

Privileged/system Vessel support, owner-reviewed execution identities, native
adoption and system update/rollback qualification belong to **v1.1.0 — Secure and
Reliable Vessels** (#344/#346/#380), following the latest release planning. Their
staged source is present but is not supported production deployment in v1.0.3.
Root metadata helpers do not execute agent loops or centralize credentials.

Remaining v1.0.3 gates include complete browser/client journeys, deployed TLS
acceptance, integrated UX/event verification and stable artifact publication.
The current full-scope coverage record measures clean Rust source `ca71ecd`:
2,339 passed, zero failed, eight ignored; 96,873 / 131,550 lines (73.639681%).
The unchanged reachable-production objective and explicit gap ledger remain
tracked by #353; passing tests do not imply that objective is met.

## Planned assets and installation

The intended Linux x86-64 assets are:

- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz`
- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `voyage-installer-x86_64-unknown-linux-gnu.gz`
- `voyage-installer-x86_64-unknown-linux-gnu.gz.sha256`

After those exact assets are verified and published, verify the full archive's
checksum before extracting. Keep `bin/`, `share/` and `release.json` together:

```sh
sha256sum -c voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz
cd voyage-v1.0.3-x86_64-unknown-linux-gnu
./bin/helm --version
./bin/voyage-installer install --bin-dir "$PWD/bin" --no-start
```

Ordinary user installation is retained. Use the [installer guide](../installer/README.md)
for the supported user-service path. System installation and reviewed cross-identity
adoption are staged v1.1.0 work, with qualification requirements in
[the remote update/adoption guide](remote-updates.md). Compilation or a checksum
does not establish service activation. Unknown effects are reconciled through their
retained original IDs.

## Platform and provider boundaries

This release targets Linux x86-64 GNU/glibc. Actual binary loader/libc requirements
and native installation results must be recorded from the published artifacts.
Runtime Python 3, Node and sandbox-capable Chromium are executing-host browser
requirements. No macOS, Windows, ARM64 or musl support is claimed; the native
macOS/Windows nightly work and deferred Tax-Axis updater adoption are planned for
v1.1.0. Provider credentials, entitlement and budget remain separate from binaries.
No paid-provider certification, production signing trust or reproducible-build
claim follows from these candidate instructions.

After stable publication, the established next nightly target is v1.1.0. That
target must be advanced separately; milestone closure alone does not publish a
release or change the stable download channel.
