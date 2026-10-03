<a id="voyage"></a>

![Voyage — AI agents working across your computers](https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-banner.svg)

<div align="center">

### Put AI agents to work across your computers.

**[Get started](#choose-how-you-want-to-use-it)** &nbsp; · &nbsp;
**[User guides](docs/README.md)** &nbsp; · &nbsp;
**[For LLMs & developers](readme.llm.md)**

</div>

Voyage is for getting work done with AI agents on computers you control. Direct
work from a browser or terminal, choose the machine and workspace, and give an
agent an outcome to deliver. Agents can edit files, run commands, build and check
software, produce documents, and operate services through the access available
on the execution host.

Keep development on your workstation, builds on a build machine, and operations
on the server that owns the service. Connect their Vessels to Helm, select where
each task runs, and follow its changes, tool activity and results. Each voyage
has its own execution process and history, so closing the interface does not
cancel accepted work.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-dark.svg" />
  <img width="100%" src="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-light.svg" alt="Choose a computer. Assign an outcome. Review the result." />
</picture>

## Give agents work to complete

| Task | Example instruction |
| --- | --- |
| Build and repair software | “Fix the failing import in this project, run the relevant checks, and show me the changes and results.” |
| Produce a deliverable | “Create an onboarding guide from this project's code and configuration. Verify the commands and save the finished guide.” |
| Operate infrastructure | “On the connected build host, set up a runner for this repository and verify that it connects and starts after reboot.” |

These tasks use the selected host's tools, credentials and permissions. Server
administration requires an authorized management path. See
[installation scope](installer/README.md#choose-the-host-account-before-installing)
for what is available today and the remaining administrator-install work.

## Three programs with distinct responsibilities

The architecture separates the interface, host supervision and agent execution:

| Component | What it owns |
| --- | --- |
| **Helm** | Browser and terminal interfaces for directing work, connecting computers, reviewing changes and responding to requests. |
| **Vessel** | The host supervisor: authenticates connections, starts and supervises independent Voyage processes, and routes authorized commands and observations. |
| **Voyage** | The agent runtime: one session per independent process, owning its agent loop, tools, execution, history and cleanup. |

```text
Helm Web / Helm TUI
  ├── Vessel on workstation ── Voyage: implement and verify a fix
  └── Vessel on server      ── Voyage: carry out an operations task
```

Helm can disconnect and reconnect while work continues. Vessel supervises the
processes without running their agent loops inside itself. Each Voyage owns its
work and durable history. Host shutdown or runtime failure can interrupt work;
recovery uses saved state and observed results. See the
[architecture](docs/architecture.md) for the process and ownership contracts.

## Direct, review and continue

Choose the computer, workspace, AI account and access mode for a task. Describe
the intended outcome and how to verify it. Follow execution, inspect changed
files and outputs, and respond when the agent needs a decision or permission.
Continue in the same voyage or start another for separate work.

Approval mode supports work with review where policy requires it. Read-only mode
is available for inspection. Full access remains subject to the host's actual
permissions; it does not grant Linux administrator privileges.

Provider credentials stay on the computer doing the work. Relevant files and
tool results may be sent to that provider. Use an eligible **ChatGPT or Grok
subscription** (SuperGrok), or an API-key account; the
[account guide](docs/provider-accounts.md) explains supported builds, sign-in and
billing. Voyage does not include AI credit.

<a id="start-with-helm-web"></a>
<a id="start-in-the-terminal"></a>

## Choose how you want to use it

- **[Helm Web](docs/getting-started-web.md):** direct tasks on connected computers
  from your browser. Each computer needs a Vessel connection.
- **[Helm TUI](docs/getting-started.md):** direct local or remote work from your
  terminal using the same Vessel and Voyage process model.
- **[Install and maintain a host](installer/README.md):** review installation
  scope, host access, updates and the remaining system-service work.

The current published installer provides a Linux user-scoped installation. Its
prebuilt path requires Linux x86-64, glibc 2.39+, Python 3.11+, curl and a systemd
user session. Browser setup currently uses a public development nightly and an
operator-configured authenticated HTTPS/WSS endpoint.

The intended host-administration setup is a privileged Vessel supervisor with a
separate unprivileged public gateway and explicitly authorized execution
identities. Ordinary tasks run with ordinary permissions; administrator tasks
require owner authorization. Supported system installation, administrator review
and migration are still being completed in [#344](https://github.com/o-psi/helm.vessel.voyage/issues/344)
and [#380](https://github.com/o-psi/helm.vessel.voyage/issues/380). User installation
is a scoped option for work under an existing account, not the intended limit of
the product. The staged system path is not yet a supported production install.

<a id="learn-more"></a>

## Find your next step

[**Browser guide**](docs/getting-started-web.md) ·
[**Terminal guide**](docs/getting-started.md) ·
[**Installation & updates**](installer/README.md) ·
[**Connect more computers**](docs/vessel-connections.md) ·
[**All documentation**](docs/README.md)

**Building on Voyage? Reading with an LLM?**
[readme.llm.md](readme.llm.md) maps the architecture, source, commands,
permissions and current implementation limits.
