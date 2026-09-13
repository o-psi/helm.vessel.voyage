# Voyage

Use AI assistance for work in a folder, keep the conversation, and return to it
later. **Helm** is the terminal interface you use; a **Vessel** supervises work on
a machine; each **voyage** is an independent process with its own conversation.
Leaving Helm does not cancel that work.

## Start here

**[Getting started: install → sign in → first task → return](docs/getting-started.md)**

The guide follows one local Linux path, explains which account pays for requests,
and keeps credentials out of chat. You do not need to configure a remote machine,
learn process IDs, or edit a TOML file to understand the first task.

**First-release development:** the guide uses a source-built Linux installation.
It does not promise a published installer asset. Named-account setup is implemented,
but successful sign-in is not proof of model access, available credit, or a passing
live-provider test. ChatGPT subscription access is experimental; it is not API
credit. Native macOS/Windows install and credential-security verification have
separate limitations.

## Pick another path

- Already installed? [Start Helm](docs/getting-started.md#2-launch-helm).
- Have an API key or an old login? [Accounts and authentication](docs/provider-accounts.md).
- Need a specific model, endpoint, or policy? [Configuration](docs/configuration.md).
- Work on another machine? [Connect a Vessel](docs/vessel-connections.md).
- Administer installation or services? [Installer guide](installer/README.md).
- Build or contribute? [Development](docs/development.md) and [quality](docs/quality.md).
- Learn the system? [Documentation index](docs/README.md),
  [current behavior](docs/current-state.md), and [architecture](docs/architecture.md).

Ordinary Helm chat, run, and managed sessions reach independent `voyage` processes
through Vessel. Switching views changes the input target, not which voyages run.
The [runtime contract](docs/runtime-contract.md) describes ownership and recovery;
[security](docs/security.md) explains the limits of application policy. Source and
focused verification establish only the behaviors actually checked, not universal
platform or provider readiness.
