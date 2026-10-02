# Your first voyage in Helm Web

Use Voyage from a browser: choose a folder, describe a task, and follow the work
in a chat interface. Open [Helm Web](https://helm.vessel.voyage/) once your machine
is connected.

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

An owner invitation gives your Web account full access to that Vessel. Only
connect a machine whose operator has authorized this access.

## 2. Choose a workspace and account

Select the connected Vessel, then a **workspace**: the folder you want Voyage
to work in. This folder is on the connected computer, which may be different
from the computer running your browser.

Choose a **profile** to select an AI account and model, or create one. The
console can guide ChatGPT account sign-in on the connected machine; API keys
use [private setup on that machine](provider-accounts.md#execution-host-api-enrollment).
Your Helm Web login and your AI account are separate.

Start with a small folder without secrets. Relevant files and results may be
sent to the selected AI provider. Profiles choose an account and model; they do
not grant extra access to files.

## 3. Create and use the voyage

Choose **Create voyage**. That opens the conversation; sending a message starts
the AI work. For a first task, try:

> Explain the main files in this folder in plain language. Don't change anything
> or run project scripts. Tell me what you couldn't determine.

Watch the answer, tool activity, and any requests for your input. For another
step, stay in the same conversation and ask a follow-up.

You can close the browser and return to the saved conversation. Accepted work
continues on the connected machine while it remains running. If the connection
drops, reconnect and check the status before repeating a task. Unsent messages
and pictures are kept while you switch conversations, but are lost on page reload.

## Connect a machine for browser use

This part currently needs someone comfortable with Linux and secure server
setup. You can use the [local terminal path](getting-started.md) if you don't
need browser access.

The operator needs to:

1. Install the [current public nightly](../installer/README.md#public-nightly-installation)
   on a supported Linux x86-64 machine.
2. Provide an authenticated public HTTPS/WSS endpoint on port 443 with a
   browser-trusted certificate. Installing Vessel alone does not publish it.
3. Create a private pairing invitation for your Helm Web account and help you
   select a folder and an eligible AI account on that machine.

Follow the [complete browser setup reference](web-setup-reference.md) for the
commands, connection requirements, and access choices. Nightlies are development
builds; this is not a one-click hosted service.

## Where to go next

- [Accounts and models](provider-accounts.md)
- [Browser connection setup reference](web-setup-reference.md)
- [Helm Web operation and deployment details](helm-web.md)
- [Technical guide for LLMs and developers](../readme.llm.md)
