# Helm

Helm is a TUI that connects only to local or remote Vessels. Each Vessel
supervises and exposes Voyage runtime processes, with **one session per independent
Voyage process**. See the [architecture](../docs/architecture.md).

`helm connect` combines independent voyages through authenticated local HTTP and
scoped HTTPS Vessel routes. Incoming durable updates use SSE, while commands remain
ordinary bounded POST requests. An SSH account adapter remains for compatibility.
Ordinary chat, plain chat, one-shot, workflow and managed commands
also use supervised voyage owners. Tools, policy, tasks, subagents, terminals,
workflows and lifecycle actions are authorized runtime requests. See the
[current implementation](../docs/current-state.md).

From the repository root:

```sh
cargo build --workspace --release --locked
export PATH="$PWD/target/release:$PATH"
helm --help
helm connect
helm config
helm doctor
helm chat
helm chat --plain
helm run "summarize the files in this workspace"
helm sessions
```

Configure a provider before sending a prompt. Native API providers do not require
Codex; API billing and subscription access are separate. Provider selection, local
endpoints, credentials, policy and remembered chat preferences are covered in
[configuration](../docs/configuration.md) and the
[example configuration](config.example.toml).

[Operations](../docs/operations.md) covers session controls, managed sessions,
connected local/remote voyages, supervised outbound workers, diagnostics and
recovery. `helm connect` needs the companion `vessel` and `voyage` executables. See
[security](../docs/security.md) for execution and privacy boundaries and
[development](../docs/development.md) for repository work. Automated tests and
evaluations were removed in #140; no current coverage is implied by this inventory.

In the connected TUI, **F9 Actions** and incoming questions/approval requests
open in a right-hand sidebar while the conversation stays visible. The sidebar
takes input focus: use its displayed keys to choose, answer, or dismiss it;
questions do not grant approval. Action editors keep their existing confirmation
and Escape behavior. The sidebar uses half the terminal width, capped at 64
columns; voyage navigation hides when space is needed. The minimum supported
terminal remains 40 columns by 18 rows.

Question and approval sidebars support left-click selection and explicit
**Confirm**/**Send** and **Deny**/**Skip** controls. Selecting **Allow once** does
not submit approval until Confirm is clicked (or Enter is pressed). Click
**Write a custom answer…** to type a response; **Back** preserves its draft.
Use the mouse wheel inside the sidebar to read long requests and **Prev**/**Next**
to switch requests when not editing. Clicks outside the sidebar do not move focus.
Pending or expired requests cannot submit another response; keyboard controls
remain available.

## Image drafts

In an existing-voyage or new-voyage composer, **Ctrl+I** opens the image modal.
**F6** is an equivalent fallback for terminals that encode Ctrl+I as Tab; ordinary
Tab navigation is unchanged. Enter a local image path (optionally `add PATH`) and
press Enter, or enter `remove INDEX` using the displayed one-based index. Paths
with spaces need no shell quoting. Files are read on the computer running Helm,
even when the voyage runs remotely. Escape returns without replacing composer
text or removing attachments.

Up to four PNG, JPEG or WebP images may be attached, with a combined 2 MiB limit.
The modal and saved transcript show names, media types, byte sizes and dimensions.
An image-only first message is supported. Image steering into an active run is
not supported: the send is refused and the full draft is kept.

The modal's `screenshot` command first shows a separate warning that the entire
Helm-local display may contain secrets. Type **CAPTURE** and press Enter to
confirm; Escape cancels. Capture uses Helm-local policy and the bounded native
screenshot helper, not a remote tool or a shell command. It adds an attachment
only: you must return to the composer and explicitly send. Capture currently
blocks the UI during the helper's bounded capture window.

Images are saved in private Helm draft files with immutable upload UUIDs and
SHA-256 digests, not canonical messages or pending public command envelopes.
After saving the frozen draft and command identity, Helm uploads bounded files
and submits metadata references. The images and text stay frozen while delivery
is unresolved. Recovery checks the original command identity; it never repeats
the submission or resumes uploads. A definite rejection keeps the complete draft
for correction. Unfinished immutable uploads do not execute a turn.
