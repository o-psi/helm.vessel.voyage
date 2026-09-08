# Helm

Helm is a TUI that connects only to local or remote Vessels. Each Vessel
supervises and exposes Voyage runtime processes, with **one session per independent
Voyage process**. See the [architecture](../docs/architecture.md).

`helm connect` combines independent voyages through authenticated local HTTP and
scoped HTTPS Vessel routes. Incoming durable updates use SSE, while commands remain
ordinary bounded POST requests. Remote connections use scoped HTTPS credentials.
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

## Paste images directly

Copy an image or take an OS screenshot, then **Ctrl+V** or **Alt+V** in the normal
composer. Both new and existing voyages accept images. A delivered Shift+Insert
key also invokes paste. Terminal Ctrl+Shift+V/Cmd+V can be intercepted by the
terminal; if an image-only paste produces nothing, use Alt+V.

Images appear as owned **[Image N]** elements at the caret. Type before/after them;
Left/Right crosses each element and Backspace/Delete removes it without losing
surrounding text. Name, actual type, size and dimensions appear in normal composer
rows. Literal lookalike text is not an attachment. Enter sends ordered text/images,
including image-only first turns.

Pasting exact local image paths, quoted/escaped paths, or local file URLs also
attaches files; ordinary prose/URLs remain text. Clipboard and file reads happen
on the Helm host, not the remote Vessel. Up to four PNG/JPEG/WebP images and 2 MiB
combined are supported. Linux needs wl-clipboard (Wayland) or xclip (X11) for native
clipboard acquisition; path paste works without those clipboard utilities.

Reads are asynchronous: typing keeps its position relative to the pending paste,
and Escape cancels. A send waits for the paste to finish/cancel. Private drafts
retain bytes after the source clipboard/file disappears; older image drafts migrate
without rewriting pending command identities. Delivery recovery never replays an
uncertain submission. See [Images and pasted screenshots](../docs/multimodal-implementation.md)
for platform caveats, resource limits and validation.

There is no image modal, F6 image control, or in-app screenshot-capture command.
Pasting a screenshot already in the clipboard needs no extra capture confirmation.
