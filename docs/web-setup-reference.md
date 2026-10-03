# Browser setup reference

For a shorter walkthrough, start with [Your first voyage in the browser](getting-started-web.md).
This reference retains the detailed setup, authority and recovery procedures.

Helm Web is the browser interface to Vessels you operate. It does not run agents
on the web server. A Vessel supervises each independent Voyage process on its
own host, where workspaces and provider credentials remain. This guide uses the
production React console at [helm.vessel.voyage](https://helm.vessel.voyage/).

## Before you start

You need a supported Linux x86-64 host for Vessel and Voyage, an authenticated
public HTTPS/WSS endpoint for that Vessel on port 443, and a browser. A local
Vessel installation by itself is not reachable from Helm Web. You also need an
eligible AI provider account on the executing host; Helm Web does not include
provider credit. Only configured Web sign-in providers appear on the login page.

For the current Web-compatible development build, download and review the
[public nightly bootstrap](../installer/README.md#public-nightly-installation) on
the Vessel host, then run as its ordinary user:

```sh
curl -fsSLo install.sh https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh
VOYAGE_VERSION=nightly sh install.sh install --start
```

That command starts a local user service. Publishing its authenticated public
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
pairing. For limited access or existing credentials, see the [connection details](helm-web.md#connect-your-vessels).

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

Choose **Create voyage** to create the independent conversation. This action
alone sends no model request. Type a small prompt, for example:

> What is in this workspace? Summarize the main files and suggest where I should start.

Sending it starts a run on the Vessel. You can leave Helm Web and return to the
saved conversation while its Voyage continues. If the connection drops, reconnect
and inspect the run status; an uncertain submission is not automatically sent
again. Unsent text and pictures survive switching conversations within the page,
but are lost on reload.

For connection recovery, profiles, browser viewing, and update controls, see
[Helm Web's current behavior](helm-web.md). The
[terminal first-voyage guide](getting-started.md) covers local TUI use instead.
