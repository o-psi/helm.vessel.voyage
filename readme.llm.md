# Voyage: technical entry point for LLMs and developers

Start here for implementation context. The [main README](README.md) and
[first-voyage guides](docs/README.md) are written for people using the product.
This file maps the technical contracts; it does not replace source or the
repository instructions in [AGENTS.md](AGENTS.md).

## Establish the facts before changing anything

- Read [AGENTS.md](AGENTS.md) for repository-wide instructions, delivery and
  verification requirements.
- Use [current-state.md](docs/current-state.md) and implementation source for
  existing behavior. [Architecture](docs/architecture.md) is the canonical
  component model; design and roadmap documents can contain unfinished work.
- Repository: `o-psi/helm.vessel.voyage`, default branch `main`. Helm Web source is
  in the separate private [o-psi/webhelm](https://github.com/o-psi/webhelm)
  repository; public references do not grant source access.
- Verify current release versions, artifacts, CLI help, UI controls and workflows
  rather than copying historical verification claims.

## Component ownership

| Component | Responsibility | Entry points |
| --- | --- | --- |
| Helm TUI | Presentation, local drafts, selection and authenticated client connections | [helm/](helm/), [component README](helm/README.md) |
| Helm Web | Browser presentation; Laravel login/tenant/pairing/bootstrap; React direct WSS to Vessel | [Web architecture and deployment](docs/helm-web.md), private `webhelm` source |
| Vessel | Supervision, client authentication, scoped routing, discovery and runtime health | [vessel/](vessel/), [component README](vessel/README.md) |
| Voyage | One session per independent process; agent loop, provider calls, tools, canonical journal and cleanup | [voyage/](voyage/), [runtime README](voyage/README.md) |
| Installer | Versioned Linux installation, services, verified upgrades and rollback | [installer/](installer/), [installer guide](installer/README.md) |
| Shared crates | Wire contracts and storage support | [voyage-protocol](crates/voyage-protocol/), [voyage-storage](crates/voyage-storage/) |

```text
Helm TUI or Helm Web → authenticated Vessel → independent Voyage process
                                           → another Voyage process
```

A voyage/session is an ongoing unit of work. A conversation is its history. A run
is one execution within it. Helm disconnect detaches the client; it does not
cancel accepted work. Machine/runtime failure can interrupt work. Restart restores
saved state in a new process incarnation, not live tool or terminal processes.

The voyage owns canonical session state and effect admission. Vessel routing and
Helm presentation must not become competing execution owners. Uncertain command
outcomes use retained identities and receipts, not blind replay. See
[runtime contract](docs/runtime-contract.md), [process access](docs/process-access.md),
[storage](docs/sqlite-storage.md) and [transport](docs/duplex-transport.md).
For unsent-content behavior, use current client source/current-state: older design
text may retain superseded draft descriptions.

## Supported onboarding paths

### Local Linux terminal

The stable bootstrap installs the latest stable release and starts a local user
service. The documented prebuilt path is Linux x86-64, glibc 2.39+, Python 3.11+,
curl and a reachable systemd user manager. Run as an ordinary user, without sudo.

```sh
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh -o install.sh
sh install.sh
export PATH="$HOME/.local/bin:$PATH"
cd /path/to/your/folder
helm --access read-only
```

Use [terminal setup reference](docs/terminal-setup-reference.md) for enrollment,
profiles, exact recovery controls and service/install caveats. The installer does
not change shell startup files, publish a public endpoint or pair Helm Web.

### Browser

Current Web setup uses an explicit public development nightly:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh
VOYAGE_VERSION=nightly sh install.sh install --start
```

The operator must separately expose an authenticated public HTTPS/WSS Vessel
endpoint on port 443 with a browser-trusted TLS certificate, then pair the exact
Web principal through a private invitation. See [Web setup reference](docs/web-setup-reference.md),
[Vessel connections](docs/vessel-connections.md), and [Web transport/deployment](docs/helm-web.md).
The public website does not supply agent compute or provider credit. Do not
advertise planned one-step enrollment as implemented.

## Accounts, permissions and privacy

Provider credentials remain on the execution host. Relevant file/tool content may
be disclosed to the selected provider. Read-only is application policy, not an OS
sandbox or a no-disclosure mode. Profiles bind account, model, thinking and service
tier; they do not confer workspace permissions. Provider authentication, billing,
model discovery and model entitlement are separate checks.

ChatGPT device/subscription sign-in is experimental. OpenAI and Anthropic API
access requires eligible API billing/credentials, separately from chat subscriptions.
Do not put keys, device codes, pairing tokens or private terminal input into
conversations or published logs. Use [provider accounts](docs/provider-accounts.md),
[configuration](docs/configuration.md), [security](docs/security.md), and
[provider attempts](docs/provider-attempts.md).

Access changes, root grants, human connections, participant bindings and
administrator enrollment are distinct authority surfaces. Full-access pairing is
not administrator enrollment. Do not infer new execution rights from membership,
login or a UI status. Application access modes retain executing-host limits.

## Technical reading map

| Topic | Documentation |
| --- | --- |
| Current behavior and pending scope | [Current state](docs/current-state.md), [implementation ledger](docs/implementation.md) |
| Host-owned browser and client viewer | [Host browser](docs/host-browser.md), [protocol](docs/host-browser-protocol.md) |
| Participant work and ownership transfer | [Process access](docs/process-access.md), [Vessel coordination](docs/vessel-coordination.md) |
| Tools, extension packages and SDK | [Executable packages](docs/executable-packages.md), [extension SDK](docs/extension-sdk.md) |
| Saved tool evidence and context | [Evidence runtime](docs/evidence-backed-context-runtime.md), [context review](docs/history-context-review.md) |
| Updates and rollback | [Remote updates](docs/remote-updates.md), [installer](installer/README.md) |
| Privileged execution design and qualification | [Privileged Vessel plan](docs/privileged-vessel-plan.md) |
| Source/build workflow | [Development](docs/development.md), [quality](docs/quality.md), [releasing](docs/releasing.md) |
| Further docs | [Documentation index](docs/README.md) |

## Source and verification

The Cargo workspace membership is defined by [Cargo.toml](Cargo.toml). Local
builds and CLI discovery start from the checkout root:

```sh
cargo build --workspace --locked
./target/debug/helm --help
./target/debug/vessel --help
./target/debug/voyage --help
./target/debug/voyage-installer --help
```

These are build/help commands, not evidence of a working provider account or
successful installation. Follow [development](docs/development.md) and
[quality](docs/quality.md) for the checks appropriate to an actual change.
Rust/test/Cargo changes require the coverage procedure in AGENTS.md.
Documentation-only changes require claim, command, path, link/anchor, manifest
and diff checks; do not rebuild Rust or run paid model requests merely for prose.

Hosted automation is build-only. [nightly.yml](.github/workflows/nightly.yml)
checks out current main and publishes public development archives; it does not
run runtime tests or update the stable channel. The source commit recorded in
`BUILD.txt` and release assets matters more than the triggering event SHA.
Do not publish a stable release or claim native macOS/Windows qualification from
Linux checks. Use the repository delivery record and current evidence for each
platform or feature claim.
