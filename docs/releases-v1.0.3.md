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

Remaining v1.0.3 gates are complete deployed browser acceptance (#333) and
stable artifact publication/installation (#375). The user moved the remaining
reachable-production coverage objective [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353)
to **v1.1.1 — Efficient Agents and Better Composers**. Its gaps remain open there;
this scope change does not claim 100% coverage or change measurement exclusions.
Applicable local tests and source-matched coverage remain required for delivered
Rust changes. The authoritative measurement is
[coverage/latest.json](../coverage/latest.json), with #353's linked source/object
audit. A source batch awaiting its coordinated measurement does not inherit an
older result.

The legacy updater repair [#401](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5949504704)
is delivered and closed with exact source, local checks, hosted archive and scoped
native evidence. Account route fencing (#400) and unchanged legacy owner admission
(#402) are also delivered and closed. Ordered live-event and UX acceptance are
recorded in closed #366/#372 and #377, respectively. The fresh 80-turn journey
passed: 160 messages, 2,720 ordered events, retention-gap recovery and lossless
17-message paging without repeated inference, with observed cleanup.

Helm Web `424d8d2397304da633d5b3dcb74b5c80ca58e0f2` is deployed in CT 106
through the existing scoped updater. Deployment invocation
`8164933cd4a944efa432e684d8bf0abf` was verified at 19:30:52 UTC.
All 214 Web checks passed (zero failures and zero skips), along with typecheck
and the production build. The exact public manifest and all six public asset
hashes matched that verified build. A normal authenticated root reload observed
five of five Vessel connections connected.

This Web source includes the shared browser opening-intent correction: explicit
dock opening may start a browser once; socket renewal, owner changes and
incidental viewer remounts observe or reattach without restarting a stopped
browser. Explicit **Start browser** remains available, and closing the panel
only detaches. See [the coordinated source contract](testing/browser-viewer-opening-intent.md).
The earlier UX delivery and Changes focus restoration are recorded in closed
core #377 and Web #5. Earlier archive qualification passed the expanded native/
Web-adapter browser journey with observed cleanup; see
[browser qualification](testing/host-browser-v1.0.3.md). Those historical results
do not establish complete deployed browser acceptance.

CT 106 completed one separately reviewed forward recovery on qualified source
`79507feba866f99d8576351b75e4fbc50fa5a527`, restoring managed service paths and
the unchanged approved HTTPS origin. The original uncertain update receipt remains
byte-identical; the new recovery does not claim that old operation succeeded.
Both retained voyages, canonical histories, accounts and credential configuration
were verified unchanged. The original saved Web connection now authenticates with
its exact twelve v1.0.2 owner rights; WorkspaceRead remains absent and its command
was definitely refused before effects. See closed #402 and #401's delivery record.

The closed #401 record distinguishes normal upgrades and live-owner refusal,
CT128 startup rollback/old-reader qualification, and CT127 helper-boundary
retirement evidence from synthetic manager/PID tests. Original uncertain updates,
failed attempts, snapshots, quarantine and changed-context refusals remain
preserved; they are not relabelled successful rollback. CT129's disposable
fixture lifetime is now observed stopped. Cleanup
`7e80567a-b53c-4d10-a07a-ae267d502ae9` recorded intent SHA
`aeb4e299fcc939c025bfe0bde2323df50367aca2a73cc1bb85770e7c20489ba1`
before one graceful shutdown. Proxmox task
`UPID:pve:001EB99A:0177AA5C:6AC00340:vzshutdown:129:root@pam:` reached
terminal OK; CT/LXC were stopped, its cgroup was absent and no exact container
process instance remained. Result SHA
`1665cfcaa493b65f1980f62847fb0c3336d885425d32ee115fab1b2bba502313`
records that disposition. Disk/configuration, the original receipt, quarantine
and changed operator namespace remain preserved. Historical readiness and
rollback remain unknown; fixture shutdown does not retroactively qualify them.
Closing #401's repair scope does not establish native system installation, every
possible rollback fault or complete browser acceptance.

Complete deployed native browser/site/privacy/renewal and nine-window cost
qualification stays open in [#333](https://github.com/o-psi/helm.vessel.voyage/issues/333).
The actual replacement A browser `08a21fc6-8cca-49c8-a0ea-9e03c2c1e4c1`
has complete maintained cleanup evidence: all 16 pinned process instances are
gone, guardian completion is observed, and its scratch, worker lock and capacity
slot are absent. See [the exact cleanup checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/333#issuecomment-5959727302).
This cleanup does not establish passing full browser qualification.

No browser qualification job is active, and Web qualification is disabled in the
restored dotenv baseline. The original temporary route operation
`5385acaf-b6eb-4b49-80e7-6855c108f73c` remains applied; its guarded exact-baseline
restoration is still owed. Full qualification and required restoration must
finish before stable publication. The corrected core source gate passed 3,092
Rust tests (zero failures, eight ignored), strict Clippy/formatting and 26 focused
browser source checks. The exact clean measured source and full-scope totals
are in [coverage/latest.json](../coverage/latest.json). Hosted build/artifact
follow-through and final optimized release verification remain pending; no stable
release is published.

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
