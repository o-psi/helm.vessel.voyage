# v1.0.3 — Live Events and Host Browser

Voyage puts AI agents to work across computers you control: choose an execution
host and workspace, assign an outcome, and review the work and verification in
Helm Web or Helm TUI. Helm presents and directs work, Vessel supervises processes
on each host, and each independent Voyage owns its agent loop, tools and history.

## Changes

- Ordered live events keep Helm TUI and Helm Web current, with recovery after
  disconnects and gaps in retained events.
- Persistent Goals belong to the Voyage runtime and survive client disconnects.
- Voyage owns the browser on the executing Vessel host. Helm TUI and Helm Web
  provide shared site viewing and control, including private browsing, child
  frames and localized visual fallback for canvas and video. Larger pages retain
  their captured styles while snapshot size and browser reads remain bounded.
- Browser opening, reconnect and shutdown handling preserve explicit user intent
  and distinguish closing a viewer from closing the host browser.
- Browser guidance follows the observed page phase and control capability;
  preparation, error and review states no longer suggest that browsing is ready.
- Client navigation and conversation workflows include the live-event and UX
  improvements recorded in the release milestone.
- Model discovery lists only models returned by the selected provider account.
  Saved model configuration is preserved; failed discovery and manually entered
  model IDs remain explicitly unverified rather than becoming discovered choices.
- Provider access denial is distinguished from invalid sign-in and exhausted
  allowance. Denied requests do not silently switch billing accounts or refresh
  credentials and retry. Native ChatGPT sign-in is presented without an
  experimental label; subscription choices follow the executing host's supported
  native connection, with explicit prerequisites when support is unavailable.

Helm disconnect does not cancel a voyage. Each voyage remains an independent
execution process supervised by Vessel. Helm Web is deployed separately from
these Linux binaries.

The authoritative acceptance and delivery evidence is in
[browser issue #333](https://github.com/o-psi/helm.vessel.voyage/issues/333),
[release issue #375](https://github.com/o-psi/helm.vessel.voyage/issues/375),
[provider access issue #415](https://github.com/o-psi/helm.vessel.voyage/issues/415), and
[the release milestone](https://github.com/o-psi/helm.vessel.voyage/milestone/3).
The [browser qualification guide](testing/host-browser-v1.0.3.md) records completed
checks and their limits. Broader production browser acceptance remains open in
#333, moved to **v1.1.1** by the release owner.

## Linux downloads and installation

The release targets **Linux x86-64 with glibc 2.39 or later** and the ELF loader
`/lib64/ld-linux-x86-64.so.2`. These requirements were inspected in all four
selected optimized executables. Release assets are:

- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz`
- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `voyage-installer-x86_64-unknown-linux-gnu.gz`
- `voyage-installer-x86_64-unknown-linux-gnu.gz.sha256`

The full archive contains `helm`, `vessel`, `voyage`, `voyage-installer`, generated
CLI documentation and the pinned host-browser worker. The standalone installer
asset does not contain the runtime programs.

Download the full archive and its checksum from
[release v1.0.3](https://github.com/o-psi/helm.vessel.voyage/releases/tag/v1.0.3),
then run:

```sh
sha256sum -c voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz
cd voyage-v1.0.3-x86_64-unknown-linux-gnu
./bin/helm --version
./bin/voyage-installer install --bin-dir "$PWD/bin" --no-start
```

Keep `bin/`, `share/` and `release.json` together. The installer verifies the
release inventory and preserves browser assets on upgrade. Follow the
[installer guide](../installer/README.md) for ordinary user-service setup and
activation. The command above installs without starting the service.

## Verification and known browser limitation

The selected four optimized programs report 1.0.3. Formatting, Clippy,
56 worker checks, packaging regressions, conversation-file checks, all four
concurrent-voyage variants and the full offline TUI/native plus Web browser
journey passed with observed cleanup.

The Rust workspace measurement records **3,114 passed, zero failed, ten ignored**
and **81.0350% line coverage**, measured from clean
[Core c19f040](https://github.com/o-psi/helm.vessel.voyage/commit/c19f04092a1964ee7f7f5e7e4cc39775da03b957).
Exact counts, toolchain and exclusions are in the
[coverage record](../coverage/latest.json). JavaScript, Python and manual journeys
are separate from this measurement; these checks do not certify live providers.

[Hosted run 37222528825](https://github.com/o-psi/helm.vessel.voyage/actions/runs/37222528825)
qualified the same delivered runtime source through
[07278f2](https://github.com/o-psi/helm.vessel.voyage/commit/07278f2fa9f09b0afd36083f3463e7c8b2233fde):
archive/checksum, four binaries, all 124 browser assets and embedded viewer bytes
were verified. Its installation and upgrade retention check used an inactive
service-manager fixture. The development build was also installed on CT106,
with native program/assets, service readiness, strict TLS, authenticated local
socket access and independent updater retirement verified. Those native facts
identify the development build; stable archive acquisition is verified separately.

**Known limitation:** full production host-browser acceptance is unfinished.
The production journey passed site classes, nested-frame control, media,
Rustbook and GitHub checks, but the Three.js Web view stopped before the required
page marker appeared. A focused check reproduced a bounded Chrome frame-tree
query timeout. Failed runs remain failed; no uncertain website action was replayed.
The release owner deferred the remaining browser work to
[#333 in v1.1.1](https://github.com/o-psi/helm.vessel.voyage/issues/333).
This release includes the implemented browser improvements and does not claim
universal site compatibility or completed production browser acceptance.

## Runtime and support boundaries

Browser execution requires Python 3, Node and sandbox-capable Chromium on the
executing Vessel host. These programs are not required on a Helm viewer machine.
Provider credentials and permission to use a provider remain separate from the
binary installation; this release makes no paid-provider certification claim.

This release does not claim macOS, Windows, ARM64 or musl support. Privileged and
system Vessel deployment, reviewed execution identities, native adoption and
system update/rollback qualification are deferred to **v1.1.0**. Native macOS and
Windows nightly qualification and the deferred Tax-Axis updater work also remain
in that release's scope. The intended administrator setup uses a privileged
supervisor and a separate
unprivileged public gateway. That work remains unfinished; the supported user
installation here is one deployment option, not the product's capability ceiling.
The remaining 100% reachable-production coverage objective is tracked in
[#353 for v1.1.1](https://github.com/o-psi/helm.vessel.voyage/issues/353).

No production signing trust or reproducible-build claim is made. See
[release documentation](releasing.md) for artifact integrity and
[remote updates](remote-updates.md) for the supported review and receipt model.
After v1.0.3 is published, the established next nightly target is v1.1.0.
