<a id="voyage"></a>

![Voyage — an AI workspace on your own machine](https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-banner.svg)

<div align="center">

### Turn “can you help me with this?” into work you can see.

**[Get started](#choose-how-you-want-to-use-it)** &nbsp; · &nbsp;
**[User guides](docs/README.md)** &nbsp; · &nbsp;
**[For LLMs & developers](readme.llm.md)**

</div>

<br />

Voyage lets you work with an AI on your own computer or server. Pick a folder,
explain what you want in everyday language, and follow along as it reads files,
answers questions, or makes changes with the access you allow.

Start small: understand a folder, work through some notes, or draft a document.
Keep the conversation and come back to it when you have more to do.

Use an eligible **ChatGPT or Grok subscription** (SuperGrok), or an API-key
account. See [provider account options](docs/provider-accounts.md) for sign-in,
supported builds and the separate billing routes.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-dark.svg" />
  <img width="100%" src="https://raw.githubusercontent.com/o-psi/helm.vessel.voyage/main/docs/assets/voyage-steps-light.svg" alt="Choose a folder. Ask in your own words. Follow the work." />
</picture>

<br />

## A few things to try

<table>
<tr><td width="33%" valign="top"><h3>Understand</h3><p>“Explain the main files in this folder. Tell me where to start without changing anything.”</p></td>
<td width="33%" valign="top"><h3>Make sense of it</h3><p>“Read these notes and give me a short summary, with the open questions at the end.”</p></td>
<td width="33%" valign="top"><h3>Make something</h3><p>“Draft a getting-started guide from the files in this project. Ask me about anything unclear.”</p></td></tr>
</table>

What Voyage can do depends on the files you share, your chosen AI model, and the
access you give it.

## Work at your pace

<table>
<tr><td width="50%" valign="top"><h3>Your folder. Your starting point.</h3><p>Choose what to share. Begin in read-only mode when you want explanations before edits.</p></td>
<td width="50%" valign="top"><h3>A conversation you can return to.</h3><p>Come back for the next step. Closing the interface does not cancel work already running on your machine.</p></td></tr>
<tr><td valign="top"><h3>Follow along as it works.</h3><p>Read the conversation and tool activity, answer questions, and review requests for permission.</p></td>
<td valign="top"><h3>Browser or terminal.</h3><p>Use the interface that suits you. Both connect to the computer doing the work.</p></td></tr>
</table>

<br />

<a id="start-with-helm-web"></a>
<a id="start-in-the-terminal"></a>

## Choose how you want to use it

<table>
<tr><td width="50%" valign="top"><h3>In your browser</h3><p>A familiar chat interface, connected to a machine you control.</p><p>Best when you already have a connected machine, or someone who can help set one up.</p><p><strong><a href="docs/getting-started-web.md">Start with Helm Web →</a></strong></p></td>
<td width="50%" valign="top"><h3>In your terminal</h3><p>Work locally on Linux, right from the folder you want help with.</p><p>Best when you are comfortable copying a few commands into a terminal.</p><p><strong><a href="docs/getting-started.md">Start in the terminal →</a></strong></p></td></tr>
</table>

**Before you begin:** browser use needs a Linux machine with a secure public
connection; signing in to the website alone does not provide one. The browser
guide separates your first conversation from the operator's setup.

The local installer supports Linux x86-64 with glibc 2.39+, Python 3.11+, curl,
and a systemd user session. The current browser setup uses a public development
nightly. Voyage is under active development, and some setup still needs technical help.

## Three names, one workflow

**Helm** is the interface you use: browser or terminal. **Vessel** is the service
on the computer doing the work. A **voyage** is your ongoing AI conversation and
its work. You choose the folder and AI account on that computer.

Your AI provider credentials stay on the computer doing the work. Relevant file
contents and tool results may be sent to your chosen provider. Read-only mode
prevents changes; it does not keep shared content from the provider.

Voyage does not include AI credit. Provider access and charges are separate from
signing in to Helm Web; [the account guide](docs/provider-accounts.md) explains
the supported options.

<a id="learn-more"></a>

## Find your next step

[**Browser guide**](docs/getting-started-web.md) ·
[**Terminal guide**](docs/getting-started.md) ·
[**Installation & updates**](installer/README.md) ·
[**Accounts & models**](docs/provider-accounts.md) ·
[**All documentation**](docs/README.md)

---

**Building on Voyage? Reading with an LLM?**
[readme.llm.md →](readme.llm.md) maps the architecture, source, commands,
permissions, and current implementation limits.
