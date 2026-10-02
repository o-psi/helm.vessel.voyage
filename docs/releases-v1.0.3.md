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

Remaining v1.0.3 gates include deployed browser TLS/client acceptance, complete
reachable-production coverage, native legacy rollback/interruption qualification
(#401) and stable artifact publication. Account route fencing (#400) and unchanged
legacy owner admission (#402) are delivered and closed. Ordered live-event and UX
acceptance are recorded in closed #366/#372 and #377, respectively.
The authoritative full-scope measurement is [coverage/latest.json](../coverage/latest.json).
Use that record and #353's linked source/object audit for the current measured
commit, counts, exclusions and remaining gaps. A source batch awaiting its
coordinated measurement does not inherit an older result. The fresh
80-turn journey also passed: 160 messages, 2,720 ordered events, retention-gap
recovery and lossless 17-message paging without repeated inference, with observed
cleanup. The unchanged reachable-production objective remains tracked by #353;
passing tests do not imply that objective is met.

Helm Web `4bb4ae03709d2a3eac5e299df729cab1f4d374a7` is deployed through the
existing scoped updater in CT 106.
Public health, manifest and all six asset files matched the verified local build;
authenticated reload and Changes panel keyboard focus restoration passed.
The UX delivery is recorded in closed core #377 and Web #5. The current archive also passed the expanded actual native/Web-adapter browser
journey with observed cleanup; see [browser qualification](testing/host-browser-v1.0.3.md).
Complete deployed browser acceptance remains a separate gate.

CT 106 completed one separately reviewed forward recovery on qualified source
`79507feba866f99d8576351b75e4fbc50fa5a527`, restoring managed service paths and
the unchanged approved HTTPS origin. The original uncertain update receipt remains
byte-identical; the new recovery does not claim that old operation succeeded.
Both retained voyages, canonical histories, accounts and credential configuration
were verified unchanged. The original saved Web connection now authenticates with
its exact twelve v1.0.2 owner rights; WorkspaceRead remains absent and its command
was definitely refused before effects. See closed #402 and #401's delivery record.

Ordinary native qualification passed normal upgrades and live-owner refusal with
observed cleanup. A controlled candidate-death case exposed a rollback service
transition refusal and remains unconfirmed with its snapshot/quarantine retained.
The correction must also preserve the restore guard's continuously held ownership
leases while selecting the old reader. Source tests alone do not establish native
rollback: fresh qualification and the remaining interruption/context cases stay
open in #401. Complete deployed browser acceptance stays open in #333; the
qualification route is disabled until its explicit bounded job is provisioned.

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
