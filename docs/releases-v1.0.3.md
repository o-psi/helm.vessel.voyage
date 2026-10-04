# v1.0.3 — Live Events and Host Browser

**Candidate: production browser qualification and stable publication are pending.**
Stable downloads still select v1.0.2. Remove this notice only after the release
criteria and public downloads have been verified.

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
The [browser qualification guide](testing/host-browser-v1.0.3.md) describes the
required journeys; prior checks alone do not establish final production acceptance.

## Linux downloads and installation

The release targets **Linux x86-64 with glibc 2.39 or later** and the ELF loader
`/lib64/ld-linux-x86-64.so.2`. These requirements were inspected in all four
executables from the qualified public development archive. Planned stable assets are:

- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz`
- `voyage-v1.0.3-x86_64-unknown-linux-gnu.tar.gz.sha256`
- `voyage-installer-x86_64-unknown-linux-gnu.gz`
- `voyage-installer-x86_64-unknown-linux-gnu.gz.sha256`

The full archive contains `helm`, `vessel`, `voyage`, `voyage-installer`, generated
CLI documentation and the pinned host-browser worker. The standalone installer
asset does not contain the runtime programs.

After publication, download the full archive and its checksum from
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

The earlier optimized candidate passed archive and checksum verification,
conversation-file and concurrent-voyage checks, and isolated installation and
upgrade with browser assets retained. That installation check used an inactive
service-manager fixture; it does not establish native service activation.

The previously qualified public development archive is
[nightly run 37178357510](https://github.com/o-psi/helm.vessel.voyage/actions/runs/37178357510),
built from [Core source 68151fc9](https://github.com/o-psi/helm.vessel.voyage/commit/68151fc9f8122754dcc32bb813397008dfe1310f).
Its archive/checksum, all four binary versions and ELF targets, glibc requirement,
124 browser assets and embedded viewer assets were verified. This previous build
(release identity `0e10276a`) was installed on the qualification host with native
program/hash and service readiness,
strict TLS and authenticated local socket checks. These are development-build
facts. It predates the completed candidate browser changes through Core source
`c19f040`; a new candidate archive, final production browser acceptance and stable
public downloads remain pending in #375.

The latest completed local Rust workspace measurement records **3,114 passed,
zero failed, ten ignored**, with **81.0350103% line coverage**. The clean measured source is
[Core c19f040](https://github.com/o-psi/helm.vessel.voyage/commit/c19f04092a1964ee7f7f5e7e4cc39775da03b957);
counts, toolchain and exclusions are retained in the compact
[coverage record](../coverage/latest.json). Python, JavaScript and production
browser journeys are separate from that Rust measurement.

Completed browser source through `c19f040` passed **56/56 worker checks** and
**21/21 focused Node 24 checks**. A bounded two-viewer check on the qualification
host verified complete page styling and guardian-observed cleanup of 13 owned
processes. This focused check does not establish the full production browser pass.
Helm Web source
[662b2af](https://github.com/o-psi/webhelm/commit/662b2af99254795c50186d38aa9e26eed873f855)
was published and deployed separately. The optimized workspace build passed with all four programs reporting 1.0.3.
Conversation-file checks, all four concurrent-voyage variants, and the full offline
TUI/native and Web browser journey passed with observed cleanup. Final hosted
archive, installation, production acceptance and stable downloads must still
identify the actual delivered candidate. Offline checks do not establish
live-provider access.

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
