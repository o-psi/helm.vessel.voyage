# Markdown rendering

Helm treats Markdown as a presentation format for assistant prose. The canonical
assistant text stored in a session is never replaced with rendered text or terminal
escape sequences, so resumed sessions, exports, and copied source retain the model's
original Markdown.

The full-screen interface renders CommonMark plus tables, task lists, and
strikethrough. Headings, emphasis, inline and fenced code, quotes, lists, links,
rules, footnotes, and tables use theme-aware terminal styles. Known fenced-code
languages receive syntax highlighting; `NO_COLOR` selects a monochrome theme and
disables highlighting. Code is displayed without generated line numbers so terminal
selection remains useful.

Rendering is width-aware and operates on Unicode grapheme clusters and terminal
display width. Tables that fit use a grid. Tables wider than the viewport switch to
a deterministic label/value layout. Raw HTML is shown as inert text, and images are
represented as `[image: alt] (URL)` because Helm neither executes nor embeds model
content.

During streaming, Helm retains the exact accumulated assistant source and reparses
it on each full-screen update. Incomplete fences, links, emphasis, lists, and tables
therefore remain safe and settle into their final representation as tokens arrive.
The view follows new content only while it is already at the bottom; manual scrolling
keeps the same content anchored through stream growth and terminal resizing.

Only assistant prose is interpreted as Markdown. User messages and tool, error,
reasoning, approval, and activity text remain literal. All display paths remove C0,
C1, terminal escape, and bidirectional-formatting controls before output reaches the
terminal.

When stdout is redirected, or `NO_COLOR` is set in line-oriented mode, Helm streams
sanitized canonical Markdown without ANSI styling and emits exactly one final
newline. Styled line-oriented output buffers one response and renders it coherently
at completion.

The renderer bounds source size, output lines, and nesting depth. Oversized output is
truncated with an explicit marker rather than consuming unbounded terminal memory.

