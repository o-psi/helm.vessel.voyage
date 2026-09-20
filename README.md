# Voyage

AI help for work in your terminal. Ask questions about a project, edit files,
investigate problems, or automate a task—in a conversation you can return to later.

You interact through **Helm**, Voyage's terminal app. Work can keep running when
you close Helm, so you can disconnect and come back without starting over.

## Get started

### 1. Install

**Linux x86-64 only for now.** You'll need curl, Python 3.11+, glibc 2.39+ and
a systemd user session. [Requirements and other installation options →](installer/README.md#download-a-published-version)

Run as your normal user, without `sudo`:

```sh
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://raw.githubusercontent.com/o-psi/voyage/main/install.sh -o install.sh
sh install.sh
```

This installs the latest release and starts its background service. Prefer to
review first? Run `sh install.sh install --dry-run` before installing.

### 2. Open a folder

```sh
export PATH="$HOME/.local/bin:$PATH"
cd /path/to/your/project
helm --access read-only
```

Replace `/path/to/your/project` with a folder you'd like help with. Read-only mode
is a good place to start: Helm can inspect files without changing them. Choose a
folder without secrets; relevant content may be sent to your AI provider.

### 3. Connect your AI account

Open **Account** below the message box, or type `/account`. Add an account, choose
its model, and follow the prompts to make it your default.

Bring your own provider access; Voyage doesn't include AI credit. You can use an
API account or the experimental ChatGPT subscription sign-in.
[Account setup and billing options →](docs/provider-accounts.md)

### 4. Try a task

> What is in this folder? Summarize the main files and suggest where I should start.

Press **Enter** to send and **F1** whenever you need help. Ready to make changes or
come back to a conversation? Follow the [first-task walkthrough](docs/getting-started.md).

## Learn more

- [Getting started](docs/getting-started.md) — a guided first task, from sign-in to returning later.
- [Accounts and models](docs/provider-accounts.md) — connect a provider or troubleshoot access.
- [Install and upgrade](installer/README.md) — installation options, updates and rollback.
- [Work on another machine](docs/vessel-connections.md) — connect Helm to a remote Vessel.
- [Configuration](docs/configuration.md) — customize models, permissions and settings.
- [Build and contribute](docs/development.md) — work on Voyage itself.
- [All documentation](docs/README.md) — including security and how Voyage works.
