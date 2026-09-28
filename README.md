# Voyage

Work with an AI agent from your browser or terminal. Ask about a project, edit
files, and return to the same conversation later. Closing Helm does not cancel
work already running on a Vessel.

**Helm Web** is the browser interface ([private source repository](https://github.com/o-psi/webhelm); access required). **Helm** is the terminal interface. Both
connect to a **Vessel** on a machine you control; each **Voyage** runs there as an
independent process. Your provider credentials stay on that executing machine.

## Start with Helm Web

1. Install a [current Vessel and Voyage build](installer/README.md#public-nightly-installation)
   on a Linux machine you control. The public nightly is the current development
   path for Web-compatible Vessel features.
2. Give that Vessel an authenticated, publicly reachable HTTPS/WSS endpoint.
   Installing the local service alone does not publish it to the internet.
3. Open [Helm Web](https://helm.vessel.voyage/), choose an available sign-in option,
   and pair your Vessel. Select its workspace and an AI account/profile, then
   choose **Create voyage**. Sending a message starts the work.

Follow the [Helm Web first-voyage guide](docs/getting-started-web.md) for pairing,
account setup, and the exact requirements. Helm Web provides the interface; it
does not host agent compute or include provider credit.

## Start in the terminal

**Linux x86-64** needs curl, Python 3.11+, glibc 2.39+, and a systemd user
session. Run as your ordinary user, without `sudo`:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh
sh install.sh
export PATH="$HOME/.local/bin:$PATH"
cd /path/to/your/project
helm --access read-only
```

The bootstrap installs the latest stable release and starts the local Vessel.
Replace the project path with a folder you are comfortable sharing with your AI
provider. In Helm, open **Account** below the message box (or type `/account`) to
connect an eligible provider account and choose a model. Voyage does not include
AI credit. Then try asking: “What is in this folder, and where should I start?”

The [terminal first-voyage guide](docs/getting-started.md) covers account setup,
read-only work, and returning to a conversation. The [installer guide](installer/README.md)
covers review-first installation, the public nightly, upgrades, and rollback.

## Learn more

- [Helm Web setup and limits](docs/helm-web.md) — login, public Vessel connections, pairing, and deployment.
- [Accounts and models](docs/provider-accounts.md) — provider access and billing distinctions.
- [Vessel connections](docs/vessel-connections.md) — local and remote connection management.
- [Configuration](docs/configuration.md) — models, permissions, and settings.
- [Build and contribute](docs/development.md) — work on Voyage itself.
- [All documentation](docs/README.md) — architecture, security, and verification.
