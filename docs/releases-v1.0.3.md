# v1.0.3 Linux x86-64 candidate

**Verification pending; no stable v1.0.3 release is published.** The checked-in
workspace and four executable versions are 1.0.3 in the release review branch.
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
grounding and shared full-site viewing/control, client UX updates, and Linux
system Vessel installation with explicitly configured ordinary execution identities.
Administrator execution requires separately provisioned policy/account state and
explicit enrolled-owner review; ordinary human connections do not acquire it
implicitly. Identity transition retains exact authority, canonical history,
interrupted work and unresolved effects. Root metadata helpers do not execute
agent loops or centralize credentials across identities or Vessels.

System adoption/update/readiness/rollback, private scope revocation, native
UID/group/capability behavior, both client/browser journeys and deployed TLS
acceptance remain verification gates. Prepared tests are not passing evidence.
The coverage objective is still 100% of reachable production behavior with the
unchanged full-workspace denominator and explicit gap accounting. The current
coverage summary measures older separately published source, not this candidate.

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

Ordinary user installation is retained. System installation is an explicit
host-operator action with distinct configured execution/gateway accounts and an
external runtime-key provisioner. Follow the
[installer guide](../installer/README.md) and
[reviewed remote update/adoption guide](remote-updates.md). Scope, identity,
service effects and exact release must be reviewed before activation. A checksum
or successful compilation does not establish working service activation or
rollback. Unknown effects are reconciled through their retained original IDs.

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
