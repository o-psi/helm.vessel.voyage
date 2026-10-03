![Voyage guides](https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-guide.svg)

# Your first voyage in the terminal

Complete a small task with an AI agent: create a useful guide in your workspace,
check the result, and continue the work. Helm is your interface, Vessel supervises
work on the chosen computer, and each Voyage runs as an independent process.
You can follow this walkthrough by copying the commands and replacing the example
folder path with your own. For the browser interface, use the
[browser guide](getting-started-web.md).

## Before you start

You need a Linux x86-64 computer with glibc 2.39+, curl, Python 3.11+, and a
systemd user session. You also need an eligible AI provider account. Voyage does
not include AI credit; [account options](provider-accounts.md) explains API keys
and sign-in with eligible ChatGPT or Grok (SuperGrok) subscriptions. A chat subscription does not
automatically provide API credit or access to every model.

This walkthrough uses the currently available user-service installation for work
within a Linux account. The intended host deployment includes authorized
administrative work; its [system-service installation and privileged execution path](privileged-vessel-plan.md)
remain unfinished. For server or Proxmox tasks today, an operator must establish
an authorized management path before assigning that work. See
[host account prerequisites](../installer/README.md#choose-the-host-account-before-installing).
Helm access modes govern agent actions; they do not grant Linux privileges.

Choose a small folder without secrets for your first task. Relevant files and
results may be sent to the AI provider. Use a folder where you can create a file.

## 1. Install on Linux

### Current user-service installation

Open a terminal as your ordinary user. Run these commands without `sudo`:

```sh
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh -o install.sh
sh install.sh
export PATH="$HOME/.local/bin:$PATH"
```

This installs the latest stable release and starts Vessel as a local user service.
Vessel supervises the independent Voyage processes that execute your tasks. Wait
for installation to finish successfully before continuing. The PATH command makes the programs available in this terminal; the installer does not
change your shell's startup settings.

If you want to inspect the installation first, run
`sh install.sh install --dry-run` before `sh install.sh`.
For development nightlies, upgrades, or an existing installation, see the
[installer guide](../installer/README.md).

## 2. Launch Helm

Replace the example path with the folder you want help with:

```sh
cd /path/to/your/folder
helm --access approval
```

Helm connects to Vessel as your terminal interface. You will see a place to type
a message and controls for the machine, folder, and AI settings. **F1** opens help.
If it cannot connect, open **Vessels** with **Ctrl+G** and read the connection error.

## 3. Create an execution profile

A **profile** saves your choice of AI account and model.

1. Open **Profile** below the message box.
2. Choose an existing profile, or press **N** to create one. A name such as
   `Everyday` is enough.
3. Select an account and model. To add a ChatGPT or Grok subscription, use
   account sign-in in the profile editor, choose **ChatGPT** or **SuperGrok**,
   and follow that provider's browser steps.
4. Choose **Save profile**, then select it with **Enter**.

Leave advanced thinking and service settings unset unless you want to change
them. For API keys, follow [private account setup](provider-accounts.md#execution-host-api-enrollment)
on the computer running Voyage. Enter credentials only in private account setup,
never in the conversation.

If sign-in or model selection fails, the [detailed account and recovery reference](terminal-setup-reference.md#3-create-an-execution-profile)
explains how to inspect the problem. Account sign-in alone does not guarantee
credit or model access.

## 4. First task

Check the selected folder and profile. The `approval` access mode is shown as
**Ask first**. Type this and press **Enter**:

> Create WORKSPACE-GUIDE.md in this folder for someone taking over the work.
> Describe its purpose, map the main files, and include setup and verification
> commands only when you can support them from existing files. Link each claim
> to its source path, mark unknowns, and leave other files unchanged. If the guide
> already exists, update it without removing unrelated content. Check the finished
> file against those requirements and report the changes and verification results.

The first message creates your voyage. Watch the answer and tool activity.
If a request is waiting or its outcome is uncertain, inspect its status before
sending the same task again.

## 5. Inspect what happened

Open the created file and check that its links and commands match the workspace.
Review the changed files to confirm the task stayed within scope. A completion
message is not a substitute for checking the artifact. Give the agent a correction
or a next task in the same conversation, for example:

> Add a troubleshooting section for the setup commands, using evidence in this
> workspace. Verify the added source references.

**Ask first** uses the runtime's approval policy; it is not a promise of one
approval for every individual file edit. Respond to approval requests as they
arise. For an inspection-only task, `helm --access read-only` is available;
**Voyage Actions → Access** changes the selected voyage's access mode.

## 6. Leave and return

**Ctrl+C** closes Helm without cancelling work already running. To stop work,
use the voyage's stop/cancel action instead. A computer shutdown or runtime
failure can still interrupt it.

Launch Helm again in the same folder, open **F2 Voyages**, and select the saved
voyage with **Enter**. Read its status, then continue the conversation.
**Ctrl+N** starts a separate draft when you want a new task.

## Where to go next

- [Detailed terminal setup and recovery](terminal-setup-reference.md)
- [Accounts and models](provider-accounts.md)
- [Installation and updates](../installer/README.md)
- [Connecting another computer](vessel-connections.md)
- [Technical guide for LLMs and developers](../readme.llm.md)
