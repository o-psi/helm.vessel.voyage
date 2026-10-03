![Voyage guides](https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-guide.svg)

# Your first voyage in Helm Web

Direct AI agents to complete work on your connected computers from a browser.
Choose a Vessel and workspace, assign a task, and verify the changes. Helm Web
is your interface; Vessel supervises independent Voyage processes on the selected
computer. Open [Helm Web](https://helm.vessel.voyage/) once your machine is connected.

## Before you start

Helm Web is the interface, not a computer rental or an AI subscription. The work
runs on a computer you control, using your AI provider account.

You need a **Vessel connection**: the link from Helm Web to that computer.
If someone has already set up a Vessel for you, ask them to help connect it to
your Helm Web account. If you are setting it up yourself, follow
[Connect a machine for browser use](#connect-a-machine-for-browser-use).
Signing in to the website alone does not create this connection or add AI credit.

## 1. Sign in and pair your Vessel

Open [Helm Web](https://helm.vessel.voyage/) and choose an available sign-in
option. New accounts have no connected machines.

Open **Connections** or **Set up your first Vessel**. The person operating the
machine uses your displayed pairing principal (the identifier for your Web
account) to make a private invitation. Enter that invitation in the connection
form, rather than in chat. The [connection reference](web-setup-reference.md#1-sign-in-and-pair-your-vessel)
has the operator commands.

An owner invitation gives your Web account full access to that Vessel. It does
not grant Linux administrator rights: work uses the host account’s existing
permissions, even with Helm **Full access** selected. Only connect a machine
whose operator has authorized this access.

## 2. Choose a workspace and account

Select the connected Vessel, then a **workspace**: the folder you want Voyage
to work in. This folder is on the connected computer, which may be different
from the computer running your browser.

Choose a **profile** to select an AI account and model, or create one. The
console supports ChatGPT or Grok subscriptions. Choose **Provider account →
Add subscription account**, then **ChatGPT** or **SuperGrok** to sign in on the
selected Vessel. API keys use [private setup on that machine](provider-accounts.md#execution-host-api-enrollment).
Your Helm Web login and your AI account are separate.

Start with a small folder without secrets. Relevant files and results may be
sent to the selected AI provider. Profiles choose an account and model; they do
not grant extra access to files.

## 3. Create and use the voyage

Choose **Approval** in the access selector. This permits workspace edits under
the runtime approval policy. Enter a task and choose **Send** to create the
voyage and start work. **Create without message** creates the voyage without
starting a run. For a first task, try:

> Create WORKSPACE-GUIDE.md in this folder for someone taking over the work.
> Describe its purpose, map the main files, and include setup and verification
> commands only when you can support them from existing files. Link each claim
> to its source path, mark unknowns, and leave other files unchanged. If the guide
> already exists, update it without removing unrelated content. Check the finished
> file against those requirements and report the changes and verification results.

Follow tool activity and respond to requests for your input or approval. Open the
finished file and review the changes against the task. Ask the agent to correct
any errors and verify the correction in the same conversation. **Approval**
uses the runtime approval policy, not a separate approval for every edit.
Read-only access remains available for tasks that only require inspection.

You can close the browser and return to the saved conversation. Accepted work
continues on the connected machine while it remains running. If the connection
drops, reconnect and check the status before repeating a task.

## Connect a machine for browser use

This part currently needs someone comfortable with Linux and secure server
setup. You can use the [local terminal path](getting-started.md) if you don't
need browser access.

The operator needs to:

1. Choose the intended work and a host account with the required access. The
   current user-service installer handles work within that account. The planned
   [system-service and administrator execution path](privileged-vessel-plan.md)
   is unfinished; infrastructure work today requires an operator-established
   management path. See [host account prerequisites](../installer/README.md#choose-the-host-account-before-installing).
2. Install the [current public nightly](../installer/README.md#public-nightly-installation)
   on a supported Linux x86-64 machine.
3. Provide an authenticated public HTTPS/WSS endpoint on port 443 with a
   browser-trusted certificate. Installing Vessel alone does not publish it.
4. Create a private pairing invitation for your Helm Web account and help you
   select a folder and an eligible AI account on that machine.

Follow the [complete browser setup reference](web-setup-reference.md) for the
commands, connection requirements, and access choices. Nightlies are development
builds; this is not a one-click hosted service.

## Where to go next

- [Accounts and models](provider-accounts.md)
- [Browser connection setup reference](web-setup-reference.md)
- [Helm Web operation and deployment details](helm-web.md)
- [Technical guide for LLMs and developers](../readme.llm.md)
