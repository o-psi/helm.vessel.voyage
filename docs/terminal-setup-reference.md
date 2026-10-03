# Terminal setup reference

For a shorter walkthrough, start with [Your first voyage in the terminal](getting-started.md).
This reference retains the detailed setup, authority and recovery procedures.

For browser use, start with [Your first voyage in Helm Web](getting-started-web.md).

Follow this guide for **local Linux terminal use**: install the programs, open
Helm, set up an account privately, have an agent create a useful workspace guide,
verify the artifact, and return to the same voyage for more work. Helm directs
work, Vessel supervises the independent Voyage process, and Voyage executes the
task. You can connect additional Vessels for work on other computers; no remote
server or project map is required for this local walkthrough.

The Linux x86-64 v1.0.2 binary release is described in the
[release guide](releases-v1.0.2.md). These instructions are not a
claim that a new user's live login or paid request has been tested. Subscription
sign-in uses an experimental provider endpoint. Installation, authentication,
account selection, and model access are different checks; none guarantees the next.

## Before you start

You need a Linux machine, a terminal, a folder you are comfortable letting the
selected provider learn about, and permission to use that provider. Start with a
small folder without secrets. Model requests can send file contents and tool results
to the selected provider. Use a folder where you can create the guide. Access
modes are application policy rather than an OS sandbox; inspection-only work can
also send content to a provider.

Choose your billing route before entering credentials:

| What you have | Route | What it does not provide |
| --- | --- | --- |
| A ChatGPT account eligible for the provider's subscription/device flow | Private ChatGPT sign-in in Helm below | OpenAI API credit; guaranteed subscription entitlement or model availability |
| An eligible Grok (SuperGrok) subscription | Private **SuperGrok** sign-in in Helm; requires native `xai_oauth` support | API credit or guaranteed access to every Grok model |
| OpenAI API credit and an API key | [Private API setup](provider-accounts.md#execution-host-api-enrollment) | ChatGPT subscription access |
| Anthropic API credit and an API key | [Private API setup](provider-accounts.md#execution-host-api-enrollment) | Access merely from a Claude chat subscription |
| An old `vessel auth login` or imported Codex cache | [Legacy migration](provider-accounts.md#legacy-migration) | A ready named/default account just because a cache exists |
| None of these | Obtain eligible access directly from your chosen provider first | Helm does not include provider credit |

The walkthrough below supports ChatGPT or Grok subscription device sign-in.
Choose **ChatGPT** or **SuperGrok** in the private account screen. If subscription
sign-in is not appropriate, complete the API branch and rejoin at **First task**
with a named default account. Eligibility and available models depend on the
provider and subscription plan.

## 1. Install on Linux

### Current user-service installation

This available installation path runs Vessel within a Linux account. The intended
host deployment includes authorized administrative work, but the
[system-service and administrator execution path](privileged-vessel-plan.md) is
unfinished. Infrastructure tasks currently need an operator-established
management path; see [host account prerequisites](../installer/README.md#choose-the-host-account-before-installing).

Use Linux x86-64 with glibc 2.39+, curl, Python 3.11+ and a reachable systemd
user manager. Run this user-service installer as your ordinary user, without sudo.
Fetch the current bootstrap, inspect it if desired, then install the latest published release:

```sh
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/install.sh -o install.sh
sh install.sh
```

No arguments means `install --start`: it installs a versioned release and enables
and starts the local Vessel user service. To review without publishing, use
`sh install.sh install --dry-run`; to leave an inactive service inactive, use
`sh install.sh install --no-start`. The script checks prerequisites before download
and verifies the archive and binary hashes before handing off to the Rust installer.
It does not edit shell startup files, enable lingering, expose a public port or
pair Helm Web. A running service stays active with `--no-start` during an upgrade.

After success, use the printed PATH command, then continue to **2. Launch Helm**.
For a pinned release or manual archive verification, see the
[v1.0.2 release installation](releases-v1.0.2.md) and
[bootstrap details](../installer/README.md#download-a-published-version).

The installer does not edit shell startup files. Add the printed PATH entry using
your shell's normal configuration if you want it in future terminals. If installation
refuses an existing unmanaged command, read the [replacement procedure](../installer/README.md)
instead of deleting it. An install failure is a blocker; do not continue with an
older `helm` accidentally found on PATH.

Already have a trusted full archive? Use its sibling `bin/voyage-installer` and
follow the [local-release procedure](../installer/README.md#install-from-a-local-release).
Keep its binaries and guide directories together. A standalone installer does not
contain the runtime programs. Upgrades require an available published stable or
nightly release, or explicit local binaries. Building from source is covered in the
[contributor guide](development.md). Native macOS/Windows are outside this Linux
installer walkthrough.

## 2. Launch Helm

Change to the folder for your first task, then launch with approval-controlled
authority so the agent can perform the requested edits:

```sh
cd /path/to/your/folder
helm --access approval
```

Replace `/path/to/your/folder` with a real folder path. Helm connects to the local
Vessel and can start one if absent; do not start another supervisor by hand just
because the screen is still connecting. If connection fails, use **Ctrl+G Vessels**
and inspect the error before retrying.

A **draft** is an unsent first message, not a running voyage. **Ctrl+N** opens a new
local draft. Check the executing host and workspace displayed above the first
message. **F1** opens help. You can leave without submitting using **Ctrl+C**.

## 3. Create an execution profile

Open **Profile** beneath the composer. If this host already has a valid default
account, a **Default** profile is created automatically. Otherwise press **N** to
create a profile, enter a name such as `Everyday`, and choose its provider account,
model, thinking level and service tier. Profile settings do not include permissions
or instructions.

In the profile editor, use account sign-in to add a ChatGPT or Grok subscription
account. Choose **ChatGPT** or **SuperGrok**, then enter a descriptive new
alias such as `personal`. Existing aliases are not silently overwritten. The
private account view also remains available through `/account`.

The dedicated private sign-in view shows the executing host, provider website,
temporary device code, expiry, and status. Press **O** to open the displayed
provider page, or open it yourself, and enter the displayed code there. Complete
the provider's instructions in the browser. **Never paste a password, API key,
token, or device code into the conversation.** The private view is not model input;
provider tokens remain on the executing host.

After enrollment, return to the profile editor, choose the account and model, and
use **Save profile**. Leave thinking/service unset unless you intend an override.
Back in the profile list, **Enter** selects the profile without sending a message.
The first saved profile becomes the default for new voyages.

The profile list offers **E** to edit, **D** to duplicate, **X** to delete (with
confirmation) and **F** to make the selected profile the default. Owner or
full-access human connections can manage profiles. Scoped users can select visible
profiles but need the host owner to manage them. Editing or deleting a profile
leaves existing voyages' copied settings unchanged. An unavailable account or model
requires correction; it does not silently select another account.

If the view needs more space, enlarge the terminal to at least **44 × 22**.
If sign-in fails, read its failure category. **R** inspects the same attempt;
**N** closes the old attempt before offering a new one. After a lost connection,
reconnect through **Ctrl+G**, then reopen Account to inspect retained enrollment.
Do not start duplicate sign-ins to fix an unknown outcome. A successful enrollment
is not proof of credit or model entitlement. See [account recovery](provider-accounts.md#device-sign-in-in-helm).

## 4. First task

Back in the draft, check the workspace, account/model, and **Ask first** access
selected by `--access approval`. Type this, then press **Enter**:

> Create WORKSPACE-GUIDE.md in this folder for someone taking over the work.
> Describe its purpose, map the main files, and include setup and verification
> commands only when you can support them from existing files. Link each claim
> to its source path, mark unknowns, and leave other files unchanged. If the guide
> already exists, update it without removing unrelated content. Check the finished
> file against those requirements and report the changes and verification results.

The first send creates the voyage and submits the message. Wait for a visible
outcome. Creating a voyage is not proof that the provider accepted the request.
If sending shows an unknown/pending outcome, keep that draft and let Helm inspect
its retained request; do not create a second voyage to repeat the same task.

Do not paste credentials if the model asks for them. If the provider refuses
billing, authentication, or model access, correct the account outside chat.
[Provider attempts](provider-attempts.md) explains observed failures and safe
continuation. There is no automatic move to someone else's account.

## 5. Inspect what happened

Open WORKSPACE-GUIDE.md and review the visible tool activity. Check its source
paths, compare the documented commands with existing project files, and confirm
that unrelated files were left unchanged. **PageUp/PageDown** scroll the transcript;
**Ctrl+End** returns to the latest output. Use **F9 Actions → Details** to inspect
the selected voyage's identity and process details. The provider-attempt details
help distinguish a request failure from a completed task.

The first task is complete when the requested file exists and meets the acceptance
criteria, including accurate references and reported unknowns. Review its actual
diff before relying on or committing it. Ask for corrections and verify them in
the same voyage. An assistant's success sentence alone is not verification.

**Ask first** follows the runtime approval policy, so respond when a consequential
action needs approval; it does not prompt separately for every edit. For optional
inspection-only work, launch with `helm --access read-only`. Use **Voyage Actions
→ Access** to change an existing voyage's access mode. Neither mode elevates the
executing Linux account.

To stop work, use **F9 Actions → Cancel current run**. A requested cancellation is
not observed cleanup. **Ctrl+C leaves Helm; it does not cancel the voyage.**

## 6. Leave and return

Press **Ctrl+C** to leave Helm. Later, open it again from your task folder:

```sh
helm --access approval
```

Press **F2 Voyages**, choose the existing voyage, and press **Enter**. Inspect its
history and current status before sending a follow-up. For example:

> Add a troubleshooting section for the setup commands, using evidence in this
> workspace. Verify the added source references.

This continues the existing conversation; **Ctrl+N** would create a separate draft.
Reopening Helm does not mean a dead process survived a reboot, nor does reconnect
cancel or replay work. If cleanup or delivery is unresolved, inspect that state
before requesting another effect. A completed voyage may later appear **Settled**;
that label is not proof that its answer is correct.

## Where to go next

- **Another provider or existing credentials:** [named accounts](provider-accounts.md)
  and [provider configuration](configuration.md#providers-and-authentication).
- **Remote work:** [Vessel connections](vessel-connections.md). The workspace and
  credentials belong to the executing host, not automatically to your laptop.
- **Admin tasks:** [installation/upgrades/services](../installer/README.md) and
  [operations](operations.md). Provision the required host authority before
  assigning infrastructure work.
- **Development:** [build and contribution guide](development.md),
  [quality and verification limits](quality.md), and [architecture](architecture.md).
