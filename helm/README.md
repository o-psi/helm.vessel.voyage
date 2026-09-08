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
