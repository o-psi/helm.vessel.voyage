# Try the setup flow

Voyage includes an interactive Rust/Ratatui setup preview. All installation,
authentication, enrollment, provider, tunnel and service actions are mocked.
It collects example answers in memory and shows the actions a real setup would
take. It does not read your existing configuration, save answers, collect keys,
open a browser, check connectivity, or install Helm/Vessel.

## Run from this checkout

```sh
cargo build -p voyage-installer --release --locked
VOYAGE_INSTALLER_BIN="$PWD/target/release/voyage-installer" sh install.sh
```

To exercise the piped entry point from a terminal:

```sh
cat install.sh | VOYAGE_INSTALLER_BIN="$PWD/target/release/voyage-installer" sh
```

The local override starts the specified executable without downloading anything.
You can also run `target/release/voyage-installer` directly. Use `--help` or
`--version` without a terminal. The interactive wizard needs at least 48 columns
and 20 rows; smaller windows pause navigation until resized.

Use arrows and Enter to choose, Space to toggle combined roles, Escape to go
back, and Ctrl+C to cancel. Text fields support typing, paste, Backspace and
Ctrl+U to clear. PageUp/PageDown scroll the review and simulated actions.
Use **Change answers** on the review screen to return to the first question.
Earlier answers are retained within the process, but inactive branches disappear
from the review. Restarting begins a fresh preview. No example model list or
network reachability is presented as discovery or verification.

## Question flow

- Choose local work, controlling other Helms, accepting remote work, hosting
  Vessel, or a combination.
- Remote roles choose an existing Vessel, one hosted here, another host, or
  deferred connection. Control-only setup skips model questions. Choosing a
  local agent alongside remote control enables provider setup.
- Vessel hosting offers loopback, LAN, internet or deferred reachability. Internet
  choices include an existing proxy, direct public access and tunnels. Temporary
  Cloudflare tunnels are labeled as temporary/no-signup/no-SSE; named Cloudflare
  tunnels require an account and domain. The proposed Voyage relay is explicitly
  unavailable and leaves connectivity deferred.
- Execution roles choose deferred provider setup, ChatGPT sign-in, API access,
  a local server or keeping existing configuration. Model IDs are examples;
  sign-in and API credentials are never requested by this preview.
- Workers choose a name, one example absolute folder, sharing, approval handling
  and foreground/background operation. Existing sessions stay private until an
  explicit future sharing decision. Background operation previews the required
  service-account/credential-ownership step.
- Review the effective answers, then preview actions or cancel. Completion says
  **Simulation complete**. Deferred prerequisites and checks not run remain visible.

## Setup capabilities and voyage roles

The preview's “controlling other Helms” choice means using this installation as
an operator interface; it skips provider questions when no local agent is selected.
Running a coordinating agent requires execution and provider configuration on the
Helm doing that work, which may be another machine. Hosting Vessel supplies the
control plane and does not make Vessel the coordinating agent.

These setup choices describe capabilities, not permanent assignments. In the planned
[voyage model](voyages.md), interface, coordinating and participant roles may overlap
or run on different Helms, and the user selects the permitted machines per voyage.
A preview folder or worker choice must not become a compulsory project/component
map or pin every task to one host. The wizard does not yet configure or demonstrate
that runtime, coordinator handoff or multi-interface reconnection.

## Small launcher and standalone assets

`install.sh` contains only platform selection, bounded HTTPS download, SHA-256
verification and terminal handoff. It binds the child to `/dev/tty`, so a future
`curl ... | sh` invocation does not consume the downloaded script as user input.
Without a controlling terminal it fails promptly. Temporary downloads are removed
when the launcher exits. No package manager, Rust compiler, Node or Python is
required to run a prebuilt installer. The Unix launcher uses `curl`, `gzip`, common
shell utilities and either `sha256sum` or `shasum`.

Without the local override it requests `voyage-installer-TARGET.gz` and its
`.gz.sha256` from GitHub Release assets. `VOYAGE_INSTALLER_VERSION` selects a tag;
the default is the latest published release. The checksum detects corruption;
it does not provide independent release signing or protect a compromised release
account. Downloads and checksums come from the same HTTPS release origin.

**There is no published installer release yet.** Release workflows produce build
artifacts but do not publish GitHub Releases. Use the local command above now.
Remote bootstrap availability requires publishing matching assets after review;
building the preview does not publish them automatically.

Create a separate compressed installer artifact after a release build:

```sh
./scripts/package-installer preview-UNIQUE_LABEL
(cd dist/installer-preview-UNIQUE_LABEL && sha256sum -c *.sha256)
```

Labels must be unique; existing installer artifact directories are never
overwritten. Unix release workflows also produce these small assets separately
from the full Voyage archive.

The launcher maps Linux x86-64 GNU and macOS x86-64/ARM64 to existing release
targets. Linux ARM64, Alpine/musl and native Windows shell bootstrap are not
supported by this launcher. Windows builds include the executable in the full
archive; native macOS/Windows interactive behavior needs separate verification.

## Verification

```sh
cargo test -p voyage-installer
cargo build -p voyage-installer --release --locked
python3 tests/system/installer_bootstrap.py
python3 tests/system/installer_tui.py
```

Unit tests cover conditional branches, retained answers, validation and rendering.
The Linux PTY fixtures exercise actual keys, Unicode/paste rejection, resize,
cancellation, signals, terminal restoration and the piped launcher. Offline
download fixtures cover verification, failures and cleanup. These tests validate
the preview, not actual provisioning or the broader remote-management workflow.
