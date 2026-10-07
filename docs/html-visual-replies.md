# HTML visual replies

Voyage exposes `html_render` to publish a self-contained interactive page in its
conversation. Helm Web shows a verified publication directly above the subsequent
reply, with an Expand control. There is no activation or source button. Ordinary
HTML and Markdown code fences remain code; they never publish executable pages.

The model supplies `html`, a short `title`, and a `height` from 180 to 640 CSS
pixels. Documents are bounded to 128 KiB UTF-8. Use inline styles/scripts, data
images, responsive layouts and CSS theme variables `--background`, `--foreground`,
`--muted`, `--muted-foreground`, `--border`, `--primary`, `--primary-foreground`,
`--card` and `--card-foreground`. External scripts, fonts and network resources
are unavailable. This is an interactive visual document, not an MCP application
with tool access or a website browser.

When the executing host has its browser distribution provisioned, `html_preview`
checks the page first. It accepts width 240–1600 and dark/light appearance, and
returns a PNG, measured content height, captured height and bounded console
messages. It uses a disposable context inside the supervised browser, independent
of the task page's cookies, references and website state. It does not download
browser dependencies or bypass required OS sandbox policy. Existing human/private
browser fences still apply. Failed or uncertain preview effects retain their
normal receipt and cleanup obligations.

Publication writes an immutable session artifact before returning a typed
`ToolOutput` Resource and `htmlRender` metadata. Artifact identity is derived from
the executing run, canonical tool call and operation; a conflicting repeat is
refused. Large tool-argument projections retain bounded complete call-name/ID summaries,
without arguments; these are provenance, not execution authority. The existing
authenticated `read_artifact` route supplies bounded chunks.
Helm checks the complete successful tool result, exact reference, each chunk,
UTF-8 and SHA-256 before creating the frame. Inactive, stale or truncated views do
not execute a page. The frame has an opaque origin and inline-script sandbox;
it has no session/storage/tool bridge, popup, download, form or top-navigation
permission. Theme and bounded height messages bind to the exact frame and channel.
These browser boundaries do not provide an OS-level CPU or memory sandbox for
arbitrary scripts.

The Web producer/consumer delivery is tracked in
[Web #77](https://github.com/o-psi/webhelm/issues/77) and
[Voyage #438](https://github.com/o-psi/helm.vessel.voyage/issues/438).
Publishing source does not establish that an existing Vessel has installed the
new runtime or that a live model successfully used the tools.

The browser packager requires `html-preview.mjs` alongside `worker.mjs`; its
existing module inventory includes and hashes both for the installed distribution.
