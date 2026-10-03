<a id="voyage"></a>

![Voyage — AI agents working across your computers](https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-banner.svg)

<div align="center">

<a id="put-ai-agents-to-work-across-your-computers"></a>

### Keep work moving while you get on with your day.

**[Get started](#choose-how-you-want-to-use-it)** &nbsp; · &nbsp;
**[User guides](docs/README.md)** &nbsp; · &nbsp;
**[For LLMs & developers](readme.llm.md)**

</div>

Voyage puts AI agents to work on computers you control. Bring a problem to
investigate, a change to make, or something to create. Start from your browser
or terminal, let the agent work, and turn your attention to the next thing.

Keep separate pieces of work in their own voyages. Several can run at once,
each with its own conversation and execution process. Come back to review what
changed, inspect the results, answer a request, or continue from where you left
off. Closing the interface does not cancel work already running.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-dark.svg" />
  <img width="100%" src="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-light.svg" alt="Choose a computer. Assign an outcome. Review the result." />
</picture>

<a id="give-agents-work-to-complete"></a>

## Start with the problem

You can begin before you know the solution. Describe what is happening, what you
want to achieve, and any limits on the work. Ask the agent to investigate, explore
an approach, or carry a task through implementation and verification.

| Task | Example instruction |
| --- | --- |
| Investigate an idea | “Look through these project notes. Identify what would be needed to try the idea and suggest a small first experiment.” |
| Repair software | “This import is failing. Find the cause, fix it, run the relevant checks, and show me the changes and results.” |
| Produce a deliverable | “Turn these notes into an onboarding guide. Check it against the source material and save the finished document.” |
| Operate infrastructure | “On the connected build host, set up a runner for this repository and verify that it connects and starts after reboot.” |

These tasks use the selected host's tools, credentials and permissions. Server
administration requires an authorized management path. See
[installation scope](installer/README.md#choose-the-host-account-before-installing)
for what is available today and the remaining administrator-install work.

## Let work continue while you step away

Use one computer or connect several. Keep development on your workstation, builds
on a build machine, and operations on the server that owns the service. Choose
where each voyage runs and use Helm to move between them without stopping the
others.

Work runs on the selected computer independently of the interface. If it is on a
server that stays available, you can close your laptop and reconnect later.
Work running on the laptop itself still depends on that laptop being awake.
Host shutdown or runtime failure can interrupt execution; saved history lets
you return to the conversation and review what happened.

<a id="direct-review-and-continue"></a>

## Return with the context intact

Each voyage keeps its conversation, tool activity and recorded results together.
Reopen it to see where the work reached, inspect changed files and outputs, and
decide what comes next. Give a correction or follow-up in the same conversation,
or start another voyage for separate work.

For work you intend to leave running, include the checks and deliverables you
will need when you return. A useful handoff might include the finished file,
the changes made, verification results, and anything still unresolved. Review
those results before relying on the work.

You choose the AI account and access mode. Approval mode asks for permission
where policy requires it. Some tasks need your input before they can proceed.
Read-only mode is available for inspection. Full access remains subject to the
host's actual permissions; it does not grant Linux administrator privileges.

Provider credentials stay on the computer doing the work. Relevant files and
tool results may be sent to that provider. Use an eligible **ChatGPT or Grok
subscription** (SuperGrok), or an API-key account; the
[account guide](docs/provider-accounts.md) explains supported builds, sign-in and
billing. Voyage does not include AI credit.

## Three programs with distinct responsibilities

| Component | Its role in your work |
| --- | --- |
| **Helm** | Your browser or terminal interface for directing work across connected computers, reviewing results and responding to requests. |
| **Vessel** | The supervisor on each computer: authenticates connections, starts and supervises independent Voyage processes, and routes authorized commands and observations. |
| **Voyage** | The runtime for one ongoing session, with its own process, agent loop, tools, history and cleanup. |

Helm presents the work, Vessel supervises it, and each Voyage executes it. This
separation lets you disconnect the interface while execution continues. See the
[architecture](docs/architecture.md) for the process and ownership contracts.

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
