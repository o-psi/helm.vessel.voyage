# Voyage

### Turn “can you help me with this?” into work you can see.

Voyage lets you work with an AI on your own computer or server. Pick a folder,
explain what you want in everyday language, and follow along as it reads files,
answers questions, or makes changes with the access you allow.

Start small: understand a folder, work through some notes, or draft a document.
Keep the conversation and come back to it when you have more to do.

**[Get started](#choose-how-you-want-to-use-it)** ·
**[User guides](docs/README.md)** ·
**[For LLMs and developers](readme.llm.md)**

## A few things to try

> “Read these notes and give me a short summary, with the open questions at the end.”

> “Help me understand this folder. Explain the main files without changing anything.”

> “Draft a getting-started guide from the files in this project. Ask me about anything unclear.”

These are starting prompts, not prebuilt workflows. What Voyage can do depends
on the files you share, your chosen AI model, and the access you give it.

## Work at your pace

- **Choose what to share.** Work in a folder you select. Start in read-only mode
  when you want explanations before edits.
- **Follow the work.** Read the conversation and tool activity, answer questions,
  and review requests for permission.
- **Pick up where you left off.** Return to a saved conversation for the next step.
  Closing the interface does not cancel work already running on your machine.
- **Use the interface that suits you.** Open Voyage in a web browser or use its
  terminal app. Both connect to the machine doing the work.

<a id="start-with-helm-web"></a>
<a id="start-in-the-terminal"></a>

## Choose how you want to use it

| | Best when… | Start here |
| --- | --- | --- |
| **Web browser** | You want a familiar chat interface and have a connected machine, or someone who can help set one up. | [Your first voyage in the browser](docs/getting-started-web.md) |
| **Terminal** | You use Linux and are comfortable copying a few commands into a terminal. | [Your first voyage in the terminal](docs/getting-started.md) |

**New to the setup?** The browser is the friendlier interface once a machine is
connected. It currently needs a Linux machine with a secure public connection;
signing in to the website alone is not enough. The guide separates what you do
in the browser from what the person setting up that machine needs to do.

For local terminal use, the installer supports Linux x86-64 with glibc 2.39+,
Python 3.11+, curl, and a systemd user session. The guides explain which build to
install; the current browser setup uses a public development nightly.

## Three names, one workflow

**Helm** is the interface you use: browser or terminal. **Vessel** is the service
on the computer doing the work. A **voyage** is your ongoing AI conversation and
its work. You choose the folder and AI account on that computer.

Your AI provider credentials stay on the computer doing the work. Relevant file
contents and tool results may be sent to your chosen provider. Read-only mode
prevents changes; it does not keep shared content from the provider.

Voyage does not include AI credit. Provider access and charges are separate from
signing in to Helm Web; [the account guide](docs/provider-accounts.md) explains
the supported options. Voyage is under active development, and some setup still
requires technical help.

<a id="learn-more"></a>

## Find your next step

- [First voyage in the browser](docs/getting-started-web.md)
- [First voyage in the terminal](docs/getting-started.md)
- [Installation and updates](installer/README.md)
- [Accounts and models](docs/provider-accounts.md)
- [All user guides and technical documentation](docs/README.md)

**Working on Voyage, or reading this with an LLM?** Start with
[readme.llm.md](readme.llm.md) for the architecture, source map, commands,
permissions, and current implementation limits.
