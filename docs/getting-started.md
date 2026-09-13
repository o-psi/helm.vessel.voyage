# Your first voyage

Follow this guide for **local Linux terminal use**: install the programs, open
Helm, set up an account privately, ask one small question about a folder, inspect
the result, and return to the same conversation. No remote server or project map
is required.

This is first-release development. These are source-verified instructions, not a
claim that a new user's live login or paid request has been tested. Subscription
sign-in uses an experimental provider endpoint. Installation, authentication,
account selection, and model access are different checks; none guarantees the next.

## Before you start

You need a Linux machine, a terminal, a folder you are comfortable letting the
selected provider learn about, and permission to use that provider. Start with a
small folder without secrets. Model requests can send file contents and tool results
to the selected provider. Read-only mode prevents tool mutations, **not disclosure**,
and is application policy rather than an OS sandbox.

Choose your billing route before entering credentials:

| What you have | Route | What it does not provide |
| --- | --- | --- |
| A ChatGPT account eligible for the provider's subscription/device flow | Private ChatGPT sign-in in Helm below | OpenAI API credit; guaranteed subscription entitlement or model availability |
| OpenAI API credit and an API key | [Private API setup](provider-accounts.md#execution-host-api-enrollment) | ChatGPT subscription access |
| Anthropic API credit and an API key | [Private API setup](provider-accounts.md#execution-host-api-enrollment) | Access merely from a Claude chat subscription |
| An old `vessel auth login` or imported Codex cache | [Legacy migration](provider-accounts.md#legacy-migration) | A ready named/default account just because a cache exists |
| None of these | Obtain eligible access directly from your chosen provider first | Helm does not include provider credit |

The walkthrough below uses ChatGPT device sign-in. If that route is not appropriate,
complete the API branch instead and rejoin at **First task** with a named default
account. Do not buy a subscription on the assumption this experimental integration
will work for it.

## 1. Install on Linux

There is no released-asset promise in this guide. The reproducible starting point
is the source tree. Install Git, a stable Rust toolchain (Cargo included), and your
Linux distribution's native compiler/linker build prerequisites first; see
[development](development.md). Building executes project/dependency build code as
your user and needs network access for uncached dependencies.

In your normal shell:

```sh
git clone https://github.com/o-psi/voyage.git
cd voyage
cargo build --workspace --release --locked
./target/release/voyage-installer
```

The installer opens a review/apply/cancel wizard. Review the destination and service
choices, then apply only what you intend. Do not use `sudo`. It installs `helm`,
`vessel`, `voyage`, and `voyage-installer` together; provider login is separate.

Add the installed commands to this shell's search path and inspect the installation:

```sh
export PATH="$HOME/.local/bin:$PATH"
voyage-installer status
helm --help
```

The installer does not edit shell startup files. Add that PATH entry using your
shell's normal configuration if you want it in future terminals. If installation
refuses an existing unmanaged command, read the [replacement procedure](../installer/README.md)
instead of deleting it. A build/install failure is a blocker; do not continue with
an older `helm` accidentally found on PATH.

Already have a trusted full archive? Use its sibling `bin/voyage-installer` and
follow the [local-release procedure](../installer/README.md#install-from-a-local-release).
Keep its binaries and guide directories together. A standalone installer does not
contain the runtime programs. Upgrades require an actually available published
release or an explicit source/local-build choice. Native macOS/Windows are outside
this Linux installer walkthrough.

## 2. Launch Helm

Change to the folder for your first task, then launch with inspection-only authority:

```sh
cd /path/to/your/folder
helm --access read-only
```

Replace `/path/to/your/folder` with a real folder path. Helm connects to the local
Vessel and can start one if absent; do not start another supervisor by hand just
because the screen is still connecting. If connection fails, use **Ctrl+G Vessels**
and inspect the error before retrying.

A **draft** is an unsent first message, not a running voyage. **Ctrl+N** opens a new
local draft. Check the executing host and workspace displayed above the first
message. **F1** opens help. You can leave without submitting using **Ctrl+C**.

## 3. Sign in privately and choose the default

Open **Account** beneath the composer (or type `/account` and press Enter). Use
arrows to choose **+ ChatGPT — sign in with your subscription**. If more than one
authorized ChatGPT connection is offered, choose the intended one. Enter a new
descriptive alias, such as `personal`, and press **Enter**; an existing alias
is not silently overwritten.

The dedicated private sign-in view shows the executing host, provider website,
temporary device code, expiry, and status. Press **O** to open the displayed
provider page, or open it yourself, and enter the displayed code there. Complete
the provider's instructions in the browser. **Never paste a password, API key,
token, or device code into the conversation.** The private view is not model input;
provider tokens remain on the executing host.

When enrollment succeeds, Helm refreshes the account list and highlights the new
account. Select it with **Enter**, review **Settings for next run**, and use
**Apply**. Check the model shown; leave reasoning/service unset unless you
intentionally want an override. An unavailable model or account needs correction,
not silent fallback.

If this host has no default, Helm opens **Ready for the first task** before applying
the draft selection. Review the executing host and named account; **Enter** saves
it as this host's default, while **Esc** returns without changing it. This is a
host-wide choice for future voyages, not just this draft, and it does not change an
active run. Scoped remote users who cannot set it must ask the host owner. After
the host confirms the default, Helm returns to the reviewed settings; press
**Enter** to apply them to this draft. This does not send a message. Check the
draft's account/model before sending. **F6 Set default** remains a separate account
management action, not a hidden prerequisite to this first-time flow.

If the view needs more space, enlarge the terminal to at least **44 × 22**.
If sign-in fails, read its failure category. **R** inspects the same attempt;
**N** closes the old attempt before offering a new one. After a lost connection,
reconnect through **Ctrl+G**, then reopen Account to inspect retained enrollment.
Do not start duplicate sign-ins to fix an unknown outcome. A successful enrollment
is not proof of credit or model entitlement. See [account recovery](provider-accounts.md#device-sign-in-in-helm).

## 4. First task

Back in the draft, check that its workspace, account/model, and read-only access
are the ones you intended. Type this, then press **Enter**:

> List the top-level files in this folder and explain briefly what this folder
> appears to contain. Do not modify files or run project scripts. Say what you
> could not determine.

The first send creates the voyage and submits the message. Wait for a visible
outcome. Creating a voyage is not proof that the provider accepted the request.
If sending shows an unknown/pending outcome, keep that draft and let Helm inspect
its retained request; do not create a second voyage to repeat the same task.

Do not paste credentials if the model asks for them. If the provider refuses
billing, authentication, or model access, correct the account outside chat.
[Provider attempts](provider-attempts.md) explains observed failures and safe
continuation. There is no automatic move to someone else's account.

## 5. Inspect what happened

Read the answer and the visible tool activity, not just the status label. Compare
claimed filenames with your folder. **PageUp/PageDown** scroll the transcript;
**Ctrl+End** returns to the latest output. Use **F9 Actions → Details** to inspect
the selected voyage's identity and process details. The provider-attempt details
help distinguish a request failure from a completed task.

This first task should not change files. Read-only refusal is expected if a tool
tries to mutate them. For future write tasks, review the requested access change
explicitly and inspect the actual diff before trusting or committing changes.
An assistant's success sentence alone is not verification.

To stop work, use **F9 Actions → Cancel current run**. A requested cancellation is
not observed cleanup. **Ctrl+C leaves Helm; it does not cancel the voyage.**

## 6. Leave and return

Press **Ctrl+C** to leave Helm. Later, open it again from your task folder:

```sh
helm --access read-only
```

Press **F2 Voyages**, choose the existing voyage, and press **Enter**. Inspect its
history and current status before sending a follow-up. For example:

> Which file would you suggest I read first, and why?

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
  [operations](operations.md). No service or remote-access administration is needed
  just to understand the first task above.
- **Development:** [build and contribution guide](development.md),
  [quality and verification limits](quality.md), and [architecture](architecture.md).
