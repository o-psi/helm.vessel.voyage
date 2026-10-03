# Browser setup reference

For a shorter walkthrough, start with [Your first voyage in the browser](getting-started-web.md).
This reference retains the detailed setup, authority and recovery procedures.

Helm Web directs AI agents to complete tasks across Vessels you operate. Helm
provides the interface; each Vessel supervises independent Voyage processes on
its host. Voyage owns task execution, and workspaces and provider credentials
remain on the executing computer. The Web server does not run the agents. This
guide uses the production React console at [helm.vessel.voyage](https://helm.vessel.voyage/).

## Before you start

You need a supported Linux x86-64 host for Vessel and Voyage, an authenticated
public HTTPS/WSS endpoint for that Vessel on port 443, and a browser. A local
Vessel installation by itself is not reachable from Helm Web. You also need an
eligible AI provider account on the executing host; Helm Web does not include
provider credit. Only configured Web sign-in providers appear on the login page.

The currently available installer provisions a user service for work within the
installing account. The intended host deployment includes authorized
administrative work, but its [system-service and privileged execution path](privileged-vessel-plan.md)
is unfinished. Infrastructure work today needs a management path established by
the host operator; a connected Vessel alone is not proof of the required access.

For the current Web-compatible development build, download and review the
[public nightly bootstrap](../installer/README.md#public-nightly-installation) on
the Vessel host, then run as its ordinary user:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh
VOYAGE_VERSION=nightly sh install.sh install --start
```

That command starts a local user service with the installing account’s existing
OS permissions. It does not grant sudo or cluster-management access. Check the
[host account prerequisites](../installer/README.md#choose-the-host-account-before-installing)
before assigning infrastructure work. Publishing its authenticated public
endpoint is a separate operator setup; follow the [Web connection requirements](helm-web.md#connect-your-vessels)
and [Vessel connection guide](vessel-connections.md#connect-the-web-console-as-the-owner).
Use a browser-trusted TLS certificate and the Vessel's public API, not its private
loopback listener. Keep invitation and connection credentials private.

## 1. Sign in and pair your Vessel

Open [Helm Web](https://helm.vessel.voyage/) and sign in. New accounts start with
no Vessel connections. Open **Connections** (or **Set up your first Vessel**) and
copy the personal tenant's pairing principal. On the Vessel host, create an owner
invitation for that principal and the exact public endpoint. Replace the paths
with the installed Vessel state directory and an owned private output location:

```sh
"$HOME/.local/bin/vessel" pair-invite --directory /path/to/vessel/state \
  --endpoint https://your-vessel.example.com \
  --principal TENANT_PRINCIPAL_UUID \
  --full-access \
  --output /private/invitation.json
```

Enter the private invitation in Helm Web's pairing form. Do not paste it into a
conversation or issue. The form verifies the Vessel before saving the connection.
Full access lets this Web tenant operate the Vessel; review that trust before
pairing. Owner pairing and the voyage’s Full access mode do not elevate its Linux
identity or bypass host permissions. For limited access or existing credentials, see the [connection details](helm-web.md#connect-your-vessels).

## 2. Choose a workspace and account

Choose your connected Vessel, then a workspace on **that Vessel host**. Use a
small folder without secrets for a first task: relevant content may be sent to
your selected AI provider. Choose an existing profile, or create one with an
available account and model. Profiles select the provider account, model,
reasoning level, and service tier; they do not change workspace permissions.

If no account is available, choose **Add subscription account** for ChatGPT or
Grok subscriptions, then select **ChatGPT** or **SuperGrok**. Sign-in creates the
account on the selected Vessel. API-key setup uses a private prompt on the executing host; see
[provider account setup](provider-accounts.md). A Web sign-in to Helm and an AI
provider account on the Vessel are separate things.

## 3. Create and use the voyage

Choose **Approval** in the access selector to permit workspace edits under the
runtime approval policy. Enter a task and choose **Send** to create the voyage
and start its first run. **Create without message** creates the independent
conversation without a model request. Assign a task with a concrete artifact
and acceptance criteria:

> Create WORKSPACE-GUIDE.md in this folder for someone taking over the work.
> Describe its purpose, map the main files, and include setup and verification
> commands only when you can support them from existing files. Link each claim
> to its source path, mark unknowns, and leave other files unchanged. If the guide
> already exists, update it without removing unrelated content. Check the finished
> file against those requirements and report the changes and verification results.

Sending it starts a run on the Vessel. Follow tool activity and respond to input
and approval requests. **Approval** follows runtime policy rather than asking
separately for every edit. When the run finishes, open the guide and check its
references, commands, and changed files against the task. Request corrections in
the same voyage and verify the result before relying on it. Read-only access is
available for inspection-only tasks.

You can leave Helm Web and return to the saved conversation while its Voyage continues. If the connection drops, reconnect
and inspect the run status; an uncertain submission is not automatically sent
again.

For connection recovery, profiles, browser viewing, and update controls, see
[Helm Web's current behavior](helm-web.md). The
[terminal first-voyage guide](getting-started.md) covers local TUI use instead.
