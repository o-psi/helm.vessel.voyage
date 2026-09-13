[Comparative audit and evidence limits](ux-comparison.md)

# Pi terminal coding-agent — source UX audit for #271

Public metadata for the requested `badlogic/pi-mono` repository resolves to `earendil-works/pi` at this pin; source URLs retain the requested redirecting name.

**Pin:** `71dca871bc80b6bc97be37f0ca3189399d651fff` (GitHub `commits/HEAD`, retrieved 2026-09-13 UTC). Scope: `badlogic/pi-mono`, `packages/coding-agent`, with its TUI and agent-loop dependencies. This is a source inspection, **not exercised terminal UX**. No installation, agent execution, provider/auth calls, sharing, or paid calls were performed. Public GitHub metadata/archive downloads only. “Implements” below means source behavior, not runtime certification. No tracked files or issues were intentionally changed.

Evidence labels link to immutable source in the manifest below. Core registered commands and dispatcher-only commands are enumerated separately; dynamic third-party commands cannot be enumerated in a clean upstream audit. Keys are defaults, not guarantees that a terminal transmits the chord. Unknown is not absence.

## 1. Slash actions: discovery → outcome → recovery/safety

`/` autocomplete uses the built-in registry; handlers below are in [I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts), registry [C](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/slash-commands.ts). Selectors replace/focus the editor and normally restore it on selection/cancellation. Keys overlap by focused surface; see complete registry below.

| Action | Source outcome / recovery / consequential distinction |
|---|---|
| `/settings` | Settings selector, nested menus; change runtime/persisted preferences. JSON-only settings are not all menu leaves; see §5. |
| `/model [provider/model]` | Selector or direct model resolution; invalid/unavailable selection reports error. Search and Tab filtering; Ctrl+S saves default in selector. Do not equate model availability with credentials. [MODEL](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/model-selector.ts) |
| `/thinking [level]` | Selector/direct level; model-dependent supported levels, temporary choice versus Ctrl+S saved preference. CLI vocabulary includes off/minimal/low/medium/high/xhigh/max; not every model supports all. |
| `/scoped-models` | Enable/disable/filter/reorder model cycling set, save via Ctrl+S. Scope must be visible when cycling. |
| `/tree` | Navigate existing session branches, not a new file; filters, folding, labels, timestamps, copy selected message, optional branch summary. Does not undo filesystem effects. [TREE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/tree-selector.ts)[CO](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/compaction.md) |
| `/export [path]` | `.jsonl` selects JSONL; otherwise HTML (default generated path). Status prints destination; exceptions show export error. Quoted paths supported by a simple first-token parser, not a shell parser. [I:6060–6106](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L6060-L6106) |
| `/import <path.jsonl>` | Confirms replacement of current session; cancel status; missing cwd asks replacement; missing file has explicit error; other failures may enter fatal-runtime handling. [I:6107–6149](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L6107-L6149) |
| `/share` | **Registry says secret GitHub gist, but implementation first tries Radius** with organization visibility when configured/authenticated; otherwise HTML via `gh gist create --public=false`. Radius JSONL includes system prompt/tool schemas. No explicit audience confirmation in this handler. Upload loader supports cancellation; network cancellation does not prove remote deletion. Secret gist is link-accessible, not private access control. [SH](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/session-share.ts) |
| `/copy` | Last assistant message to clipboard, status/error. Shortcut can prefer selection; no assistant message is not a successful copy. Clipboard delivery not exercised. |
| `/name [text]` | Set session display name; empty input supplies usage/current-name feedback. |
| `/session` | Session identity/file and usage statistics; inspect rather than mutate. |
| `/changelog` | Render release notes; collapse preference available. |
| `/hotkeys` | Render effective keyboard shortcuts; use context-specific hints too. |
| `/fork` | Select earlier user message; create separate session up to that point and restore prompt for editing. Unlike tree navigation, creates separate history. No code rollback. [S](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/sessions.md)[F](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/session-format.md) |
| `/clone` | Duplicate session at current position; separate session identity, not replay of tools. |
| `/trust` | Save project/parent trust decision for future startup in trust.json; **does not reload current session**, restart required. This is extension/config trust, not per-tool approval. [R](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md) |
| `/login [provider]` | Provider authentication selector/direct provider path, API-key or OAuth/provider-specific flow; errors/cancellation and post-login model guidance. Browser/device flows depend on provider and host. [AU](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/providers.md)[I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts) |
| `/logout` | Provider auth removal selector; removing Pi auth does not imply revoking upstream tokens or removing environment credentials. [AU](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/providers.md) |
| `/new` | Reset/new session through runtime host; clears conversation view, not disk changes; extension lifecycle can participate. |
| `/compact [instructions]` | Manual model-backed summary, refuses empty history; loader/Esc abort; failure/error and aborted status. Consumes provider budget if actually used; not exercised. [CO](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/compaction.md) |
| `/resume` | Session browser; current/all directory scopes, search, rename, delete with confirmation, sorting/path/named filters. Missing cwd resolution and trust matter on cross-project switch. [RES](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/session-selector.ts)[S](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/sessions.md) |
| `/reload` | Reload keys/extensions/skills/prompts/themes/context; refuses during streaming or compaction, resets extension UI, loading/error/status flow. Not an agent restart. |
| `/quit` | Shutdown lifecycle and terminal cleanup; not durable detach. Helm's independent Voyage survival is a different product contract. |

**Dispatcher-only core commands** (absent from completion registry): `/debug` dumps diagnostic layout under agent directory; `/arminsayshi`, `/dementedelves` render easter-egg art. [I:3090–3107,6447–6503](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L3090-L3107). These must not disappear from an exhaustive command inventory merely because help omits them.

**Bundled extension:** `/llama` is registered by `src/extensions/llama/index.ts`, not core slash registry. Opens llama.cpp router model management: catalog refresh, download, load, unload; connection errors offer retry/close, non-TUI calls warn. It depends on configured client/server; no server exercised. [LL](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/extensions/llama/index.ts)

**Dynamic extensions/resources:** `registerCommand`, `registerShortcut`, `registerTool` may add arbitrary actions/UI; prompt templates expand `/name [args]`; skills expose `/skill:name` when enabled. Permission gates, plans, subagents, git checkpoints and custom questions are extension examples, **not built-in guarantees**. [EX](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/extensions.md)[PT](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/prompt-templates.md)[SK](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/skills.md)

## 2. Editor and keyboard action inventory

The following tables are extracted directly from **both source registries**, retaining the exact `defaultKeys` expressions (including platform branches). App entries override inherited TUI entries. `[]` means an available action with no default binding, not absent behavior. `windowsKeybindings` means native Windows **or WSL**; suspension uses native platform separately. Descriptions are upstream registry labels. [K](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/keybindings.ts)[T](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/keybindings.ts)

### [T](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/keybindings.ts) defaults

| Action ID | Default keys/expression | Action |
|---|---|---|
| `tui.editor.cursorUp` | `"up"` | Move cursor up |
| `tui.editor.cursorDown` | `"down"` | Move cursor down |
| `tui.editor.historyPrevious` | `[]` | Select previous prompt history entry |
| `tui.editor.historyNext` | `[]` | Select next prompt history entry |
| `tui.editor.cursorLeft` | `["left", "ctrl+b"]` | Move cursor left |
| `tui.editor.cursorRight` | `["right", "ctrl+f"]` | Move cursor right |
| `tui.editor.cursorWordLeft` | `["alt+left", "ctrl+left", "alt+b"]` | Move cursor word left |
| `tui.editor.cursorWordRight` | `["alt+right", "ctrl+right", "alt+f"]` | Move cursor word right |
| `tui.editor.cursorLineStart` | `["home", "ctrl+home", "ctrl+a"]` | Move to line start |
| `tui.editor.cursorLineEnd` | `["end", "ctrl+end", "ctrl+e"]` | Move to line end |
| `tui.editor.jumpForward` | `"ctrl+]"` | Jump forward to character |
| `tui.editor.jumpBackward` | `"ctrl+alt+]"` | Jump backward to character |
| `tui.editor.pageUp` | `["pageUp", "ctrl+pageUp"]` | Page up |
| `tui.editor.pageDown` | `["pageDown", "ctrl+pageDown"]` | Page down |
| `tui.editor.deleteCharBackward` | `"backspace"` | Delete character backward |
| `tui.editor.deleteCharForward` | `["delete", "ctrl+d"]` | Delete character forward |
| `tui.editor.deleteWordBackward` | `["ctrl+w", "alt+backspace"]` | Delete word backward |
| `tui.editor.deleteWordForward` | `["alt+d", "alt+delete"]` | Delete word forward |
| `tui.editor.deleteToLineStart` | `"ctrl+u"` | Delete to line start |
| `tui.editor.deleteToLineEnd` | `"ctrl+k"` | Delete to line end |
| `tui.editor.yank` | `"ctrl+y"` | Yank |
| `tui.editor.yankPop` | `"alt+y"` | Yank pop |
| `tui.editor.undo` | `"ctrl+-"` | Undo |
| `tui.input.newLine` | `["shift+enter", "ctrl+j"]` | Insert newline |
| `tui.input.submit` | `"enter"` | Submit input |
| `tui.input.tab` | `"tab"` | Tab / autocomplete |
| `tui.input.copy` | `"ctrl+c"` | Copy selection |
| `tui.select.up` | `"up"` | Move selection up |
| `tui.select.down` | `"down"` | Move selection down |
| `tui.select.pageUp` | `"pageUp"` | Selection page up |
| `tui.select.pageDown` | `"pageDown"` | Selection page down |
| `tui.select.confirm` | `"enter"` | Confirm selection |
| `tui.select.cancel` | `["escape", "ctrl+c"]` | Cancel selection |
| `tui.altScreen.pageUp` | `"pageUp"` | Scroll viewport up one page |
| `tui.altScreen.pageDown` | `"pageDown"` | Scroll viewport down one page |
| `tui.altScreen.halfPageUp` | `[]` | Scroll viewport up half a page |
| `tui.altScreen.halfPageDown` | `[]` | Scroll viewport down half a page |
| `tui.altScreen.lineUp` | `[]` | Scroll viewport up one line |
| `tui.altScreen.lineDown` | `[]` | Scroll viewport down one line |
| `tui.altScreen.previousPrompt` | `["ctrl+shift+up", "ctrl+up"]` | Jump to previous semantic prompt |
| `tui.altScreen.nextPrompt` | `["ctrl+shift+down", "ctrl+down"]` | Jump to next semantic prompt |
| `tui.altScreen.search` | `"ctrl+shift+f"` | Search the primary scroll view |
| `tui.altScreen.searchNext` | `["enter", "ctrl+g"]` | Select the next search match |
| `tui.altScreen.searchPrevious` | `["shift+enter", "ctrl+shift+g"]` | Select the previous search match |
| `tui.altScreen.searchClose` | `"escape"` | Close transcript search |
| `tui.altScreen.top` | `"home"` | Scroll viewport to top |
| `tui.altScreen.bottom` | `"end"` | Scroll viewport to bottom |
### [K](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/keybindings.ts) defaults

| Action ID | Default keys/expression | Action |
|---|---|---|
| `tui.editor.undo` | `process.platform === "win32" ? "ctrl+z" : windowsKeybindings ? "alt+z" : "ctrl+-"` | Inherited TUI action; platform override |
| `tui.altScreen.previousPrompt` | `windowsKeybindings ? "ctrl+up" : ["ctrl+shift+up", "ctrl+up"]` | Inherited TUI action; platform override |
| `tui.altScreen.nextPrompt` | `windowsKeybindings ? "ctrl+down" : ["ctrl+shift+down", "ctrl+down"]` | Inherited TUI action; platform override |
| `tui.altScreen.search` | `windowsKeybindings ? "ctrl+f" : "ctrl+shift+f"` | Inherited TUI action; platform override |
| `app.interrupt` | `"escape"` | Cancel or abort |
| `app.clear` | `"ctrl+c"` | Clear editor |
| `app.exit` | `"ctrl+d"` | Exit when editor is empty |
| `app.suspend` | `process.platform === "win32" ? [] : "ctrl+z"` | Suspend to background |
| `app.thinking.cycle` | `"shift+tab"` | Cycle thinking level |
| `app.thinking.save` | `"ctrl+s"` | Save thinking level |
| `app.model.cycleForward` | `"ctrl+p"` | Cycle to next model |
| `app.model.cycleBackward` | `windowsKeybindings ? "alt+p" : "shift+ctrl+p"` | Cycle to previous model |
| `app.model.select` | `"ctrl+l"` | Open model selector |
| `app.tools.expand` | `"ctrl+o"` | Toggle tool output |
| `app.thinking.toggle` | `"ctrl+t"` | Toggle thinking blocks |
| `app.session.toggleNamedFilter` | `"ctrl+n"` | Toggle named session filter |
| `app.editor.external` | `"ctrl+g"` | Open external editor |
| `app.message.copy` | `"ctrl+x"` | Copy message to clipboard |
| `app.message.followUp` | `windowsKeybindings ? "ctrl+q" : "alt+enter"` | Queue follow-up message |
| `app.message.dequeue` | `windowsKeybindings ? "alt+q" : "alt+up"` | Restore queued messages |
| `app.clipboard.pasteImage` | `windowsKeybindings ? "alt+v" : "ctrl+v"` | Paste image from clipboard (text fallback) |
| `app.session.new` | `[]` | Start a new session |
| `app.session.tree` | `[]` | Open session tree |
| `app.session.fork` | `[]` | Fork current session |
| `app.session.resume` | `[]` | Resume a session |
| `app.tree.foldOrUp` | `process.platform === "darwin" ? ["alt+left", "ctrl+left"] : ["ctrl+left", "alt+left"]` | Fold tree branch or move up |
| `app.tree.unfoldOrDown` | `process.platform === "darwin" ? ["alt+right", "ctrl+right"] : ["ctrl+right", "alt+right"]` | Unfold tree branch or move down |
| `app.tree.editLabel` | `"shift+l"` | Edit tree label |
| `app.tree.toggleLabelTimestamp` | `"shift+t"` | Toggle tree label timestamps |
| `app.session.togglePath` | `"ctrl+p"` | Toggle session path display |
| `app.session.toggleSort` | `"ctrl+s"` | Toggle session sort mode |
| `app.session.rename` | `"ctrl+r"` | Rename session |
| `app.session.delete` | `"ctrl+d"` | Delete session |
| `app.session.deleteNoninvasive` | `"ctrl+backspace"` | Delete session when query is empty |
| `app.models.save` | `"ctrl+s"` | Save model selection |
| `app.models.enableAll` | `"ctrl+a"` | Enable all models |
| `app.models.clearAll` | `"ctrl+x"` | Clear all models |
| `app.models.toggleProvider` | `"ctrl+p"` | Toggle all models for provider |
| `app.models.reorderUp` | `"alt+up"` | Move model up in order |
| `app.models.reorderDown` | `"alt+down"` | Move model down in order |
| `app.tree.filter.default` | `"ctrl+d"` | Tree filter: default view |
| `app.tree.filter.noTools` | `"ctrl+t"` | Tree filter: hide tool results |
| `app.tree.filter.userOnly` | `"ctrl+u"` | Tree filter: user messages only |
| `app.tree.filter.labeledOnly` | `"ctrl+l"` | Tree filter: labeled entries only |
| `app.tree.filter.all` | `"ctrl+a"` | Tree filter: show all entries |
| `app.tree.filter.cycleForward` | `"ctrl+o"` | Tree filter: cycle forward |
| `app.tree.filter.cycleBackward` | `"shift+ctrl+o"` | Tree filter: cycle backward |

Additional dispatch semantics [I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts)[E](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/components/editor.ts)[TREE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/tree-selector.ts)[RES](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/session-selector.ts):
- Enter submits; while streaming it queues **steering**. Follow-up chord queues for after the agent would stop. Dequeue restores queued text; Esc during streaming restores queue and requests abort. Double-Esc within 500ms with empty editor opens configured tree/fork (or none).
- Ctrl+C clears editor; second within 500ms shuts down. Ctrl+D exits only when editor is empty, otherwise editor delete-forward. Ctrl+Z is Unix suspend (resume via shell `fg`), no native Windows default.
- Tab completes slash commands, paths and `@` file references; prompt/skill resources add completions. Paste supports images with text fallback and large-paste handling. External editor uses configured command then VISUAL/EDITOR fallback; clipboard, IME, bracketed paste and terminal protocol behavior are **not runtime verified**. [R](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md)[TERM](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/terminal-setup.md)
- Editor also accepts hard-coded Shift+Backspace / Shift+Delete aliases and Shift+Space (inserts regular space); Ctrl+Shift+D invokes debug at TUI level, outside app registry. [UI](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/tui.ts)
- Tree: left/right or page keys page through list; Backspace edits search, printable input filters; Enter selects, Escape cancels. Ctrl+X copies selected message. Session browser Tab switches scope; Enter confirms deletion only in confirmation state, Escape cancels. Same Ctrl+D can delete a session, select tree default filter, delete a character or exit depending on focus.
- Fullscreen viewport shadows unmodified editor PageUp/PageDown/Home/End; search has its own next/previous/close focus. Mouse/selection and scrollback interactions require hands-on testing, not inferred visual quality.

## 3. Streaming, reasoning, tools, queue and failure handling

| Action | Source outcome / limit / Helm lesson |
|---|---|
| Submit prompt / observe stream | Agent/turn/message start-update-end events update one assistant component and working/progress indicators; footer refreshes. Source shows incremental rendering, not measured smoothness or latency. [I:3160 onward] |
| Change reasoning / hide reasoning | Thinking level changes model request behavior; Ctrl+T changes **visibility**, not requested reasoning effort. Preserve that distinction in labels. [I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts)[K](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/keybindings.ts) |
| Inspect tools | Tool start/update/end components show arguments, partial output and final outcome; Ctrl+O toggles expansion. Not all tools/providers produce partial updates. Do not equate a tool start with success. [I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts)[TOOLS](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/tools/index.ts) |
| `read` | Read text/images; result size/truncation handling belongs to tool, not conversation pagination. |
| `bash` | Execute commands with streaming output/cancellation support. Shell side effects persist beyond conversation navigation. |
| `edit` | Targeted text replacement; mismatch is an error, not an implicit broad overwrite. |
| `write` | Create/replace file content; destructive capability, no built-in per-write approval. |
| `grep`, `find`, `ls` | Additional search/list tools in all-tools registry and read-only factory; not in default four-tool coding factory. |
| `powershell` | Additional registered tool/factory; platform/config selection must not be confused with default bash factory. Native Windows behavior unverified. |
| `!command` / `!!command` | Human shell command, included in model context versus explicitly excluded. Esc aborts running shell; exclusion from context is **not a private terminal/security boundary**. [I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts) |
| Enter during run (steer) | Queue rendered and delivered at agent boundaries. **Current loop checks steering after a completed tool batch/turn**, not guaranteed interruption between each tool call. No mid-token promise. [L:167–273] |
| Follow-up key | Separate queue delivered when agent would otherwise finish; settings independently choose one-at-a-time/all for steering and follow-up. [A](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/agent-session.ts)[L](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/agent/src/agent-loop.ts)[SE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/settings-selector.ts) |
| Dequeue / Esc | Restore queued messages to editor; source abort request is not proof external effects were undone. Retry-pending input queues for retry turn. [I:4400 onward] |
| Tool error / truncated call | Tool validation/execution errors become results; output-length-truncated assistant tool calls are failed rather than executing possibly incomplete arguments. [L](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/agent/src/agent-loop.ts) |
| Transient provider error | Auto-retry enabled by default: 3 agent retries, base 2s exponential delay, 60s cap. Provider retries separately default 0; server delay cap avoids indefinite waiting. UI displays retry state and Esc cancellation. No outage exercised. [SD](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md)[I](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts) |
| Overflow / threshold | Automatic compaction enabled, default reserve 16384 and recent 20000 tokens; threshold checks and overflow recovery. Summary is lossy; original session history retained. Provider-backed, cancellable; per-model exact-ID overrides apply next check. [CO](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/compaction.md)[SD](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md) |
| Restart / recover session | Persisted JSONL history and resume/continue support conversational recovery, **not survival of an executing process**, external-effect rollback, or automatic transaction replay. Crash consistency and corruption repair not certified by this audit. [S](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/sessions.md)[F](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/session-format.md) |

## 4. Session and history sub-actions

[S](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/sessions.md)[F](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/session-format.md)[TREE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/tree-selector.ts)[RES](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/session-selector.ts)[CO](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/compaction.md) separate the following user intents:
1. Create (`/new`); continue latest (`pi -c`); select prior (`/resume`, `pi -r`); explicit `--session` path/identity; ephemeral `--no-session`.
2. Inspect (`/session`), name (`/name`), search/filter/sort/path toggle in browser, rename, confirm-delete. Deletion is not archive; no recovery-bin claim established.
3. Tree branch navigation retains shared history in one JSONL parent-linked tree; fork selects an earlier user message into a new session; clone duplicates current position. None restore working-tree files.
4. Tree filters: default, no-tools, user-only, labeled-only, all; cycle forwards/backwards; fold/unfold; edit label, toggle timestamps; copy selected entry. Summary of abandoned branch is optional, distinct from history compaction.
5. Compact manually with optional instructions, auto-compact or disable, configure model-specific token budgets. Summaries track file operations/context but are not canonical replacement for original records.
6. Export current session representation to HTML/JSONL, import with replacement confirmation and cwd recovery; share uploads are a separate externally visible effect. Do not equate export, share and copy confidentiality.

## 5. Model, auth, settings and extension action leaves

- Model selector: search, filter via Tab, choose current model, save default with Ctrl+S. Cycling has forward/backward keys, scoped set and order. `/thinking` saves model-dependent preferences separately. Custom provider/model definitions use `models.json`; metadata/context/cost configuration is not measured provider capability. [MODEL](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/model-selector.ts)[MO](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/models.md)[SD](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md)
- Login selects provider/API-key/OAuth flow; credentials stored in agent auth storage, API env vars also supported. OAuth/browser cancellation, expiry/refresh, API-key validation and upstream revocation were **not exercised**. Command-based secret resolution is code execution, not literal secret storage. Subscription billing warnings are not a spending cap. [AU](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/providers.md)
- Global settings and trusted project `.pi/settings.json` merge recursively; JSON controls more than the menu. `/reload` handles resources, not all startup-only effects. [SD](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md)[R](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md)

**Actual settings-selector IDs (including nested actions):** `anthropic-extra-usage`, `light-theme`, `dark-theme`, `apply`, `single-mode`, `autocompact`, `steering-mode`, `follow-up-mode`, `transport`, `http-idle-timeout`, `hide-thinking`, `mermaid-rendering`, `cache-miss-notices`, `collapse-changelog`, `quiet-startup`, `install-telemetry`, `default-project-trust`, `double-escape-action`, `tree-filter-mode`, `warnings`, `model-thinking`, `tui-mode`, `fullscreen-exit-output`, `fullscreen-scrollbar`, `fullscreen-copy-on-select`, `theme`, `show-images`, `image-width-cells`, `auto-resize-images`, `block-images`, `skill-commands`, `show-hardware-cursor`, `editor-padding`, `output-padding`, `autocomplete-max-visible`, `clear-on-shrink`, `terminal-progress`. [SE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/settings-selector.ts)


Menu groups cover auto-compaction; steering/follow-up modes; transport/HTTP idle timeout; thinking/Mermaid/cache notices; startup/changelog; install telemetry/project trust; double-Esc/tree filter; warnings/model thinking; inline/fullscreen mode and exit output/scrollbar/copy-on-select; theme single/light/dark/apply; images/resize/block; skills; cursor/padding/autocomplete/shrink/progress. Some image controls are conditional. JSON-only leaves additionally include retry/provider retry caps, compaction per-model overrides, model defaults/scopes, external editor, shell configuration, resource paths/package filters, and global HTTP proxy. Menu IDs are implementation identifiers, not slash commands. [SE](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/settings-selector.ts)[SD](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md)

Extension/package actions: `pi install`, `remove`, `update`, `list`, `config` manage npm/git/local resources; source-trust matters before loading code. Enable/disable package resource subsets; reload discovered extensions; CLI `-e` explicitly loads extensions. Themes alter rendering, prompts substitute text/arguments, skills load instructions, extensions execute code/register providers/tools/commands/hooks/UI. Arbitrary third-party leaves are out of finite upstream scope. `/llama` load/unload/download/retry/close is bundled but extension-implemented. [PK](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/packages.md)[EX](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/extensions.md)[LL](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/extensions/llama/index.ts)

## 6. Permissions, consent and recovery boundaries

**Confirmed absence in core product contract:** no built-in OS sandbox or per-tool permission popups; use external container/VM or extension policy. Tool selection such as read-only factories does not contain extension code or establish OS isolation. Agents/tools run with local user privileges. [SEC](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/security.md)[R](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md)

**Present, different boundary:** project trust before project settings/resources/packages/extensions load. Global and CLI extensions plus context files are loaded before project decision and can handle trust event. Interactive asks where needed; noninteractive default `ask`/`never` ignores untrusted project resources, `always` trusts; `--approve`/`--no-approve` override project trust for one run. `/trust` saves future decision, including immediate parent scope; restart needed. `pi update` never prompts. These flags must not be described as approving arbitrary tool executions. [R](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md)

**Not verified/unspecified here:** crash durability under kill/disk-full, terminal restore on broken tty, process descendant cleanup, credential file native ACLs, clipboard secrecy, all transport/provider error mappings, accessibility, Windows/macOS/SSH/tmux/IME rendering. No basis for security certification or claiming absence of a feature just because a runtime scenario was not executed.

## 7. Actionable comparative lessons for Helm (#271)

These are design/test recommendations, not claims that Helm currently lacks them. Helm's separate TUI/Vessel/Voyage ownership and private-human-terminal boundary (local `docs/current-state.md`) should remain intact.

| Priority | Lesson grounded in Pi action | Acceptance target for Helm |
|---|---|---|
| P0 | `/share` registry says gist while implementation can upload to Radius organization | Generate help from real destinations; confirm destination/audience/payload before upload; distinguish cancel request from remotely absent artifact. |
| P0 | Steering is queued at defined loop boundary, Esc is abort not rollback | Render queued/admitted/delivered/aborted as different states; preserve editable unsent text; test steering during tools/retry/compaction. |
| P0 | `/trust` is resource trust, not execution permission | Keep provider auth, project trust, Voyage execution approvals and private-terminal capture consent visibly separate; state when a changed decision takes effect. |
| P1 | Tree/fork/clone/new/resume encode different history intents | Label identity change, context selection, persisted branch and filesystem non-rollback explicitly; confirm destructive session deletion/import. |
| P1 | Keys overlap by editor/tree/session/search focus | Effective per-focus key help and a visible focus cue; test Ctrl+D/Ctrl+P/Ctrl+O/Esc with nested selectors and pending operations. |
| P1 | Reasoning level versus hidden thinking; tool partial versus final | Separate inference configuration from display; preserve canonical streamed text and errors when collapsing/reloading. |
| P1 | Retry caps and missing-cwd recovery are explicit | Show wait reason/count/time and cancel route; bounded unattended behavior; recover cwd without silently executing in a different project. |
| P2 | Hotkeys registry omits hard-coded hidden commands; menu not all settings | Derive action inventory from registry **and dispatcher** and nested menu leaves; mark unbound/platform keys, advanced JSON-only settings, extension provenance. |
| P2 | Reload is unavailable during stream/compaction | Explain refusal and safe next step; do not appear to accept changes that were not applied. |

## 8. Unexecuted scenario backlog (not passing tests)

Use isolated disposable files and mock/free local provider with explicit approval before any networked inference: (1) type/steer/dequeue while a two-tool batch runs; (2) follow-up ordering in both queue modes; (3) abort stream/retry/compaction and inspect preserved text/history; (4) tree versus fork versus clone with a changed disk file; (5) missing-cwd import cancel/replace; (6) export quoted paths and failed destinations; (7) mocked share destination/audience/cancel-after-upload; (8) trust ask/never/always in interactive/print/RPC; (9) same chords in editor/tree/browser/fullscreen search; (10) reload busy/refusal/error; (11) OAuth cancelled/expired credentials without paid inference; (12) native Linux/macOS/Windows terminal restoration, image clipboard and external editor failure. This audit ran **none** of these.

## 9. Source manifest and reproducibility

Archive: `pi/source.tar.gz`; extracted immutable snapshot: `pi/source/`; GitHub commit metadata: `pi/commit.json`; focused dispatcher index: `pi/interactive-index.txt`; generation script: `pi/build-report.py`; hashes: `pi/manifest.json`. All downloaded material is below `target/ux-audit/pi`. Report is the requested sibling artifact. GitHub can redirect the historical repo alias; commit SHA, not repository branding, is the pin.

Reproduce download: `gh api repos/badlogic/pi-mono/commits/HEAD --jq .sha`, then use that fixed SHA for `https://api.github.com/repos/badlogic/pi-mono/tarball/<sha>`; extract only below the evidence directory. Do not resolve HEAD again and silently mix revisions. This run inspected #271 read-only and observed pre-existing tracked modifications/deletions; no issue mutations, staging, commit or publication were performed.

| Ref | Pinned source |
|---|---|
| C | [packages/coding-agent/src/core/slash-commands.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/slash-commands.ts) |
| I | [packages/coding-agent/src/modes/interactive/interactive-mode.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts) |
| K | [packages/coding-agent/src/core/keybindings.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/keybindings.ts) |
| T | [packages/tui/src/keybindings.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/keybindings.ts) |
| E | [packages/tui/src/components/editor.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/components/editor.ts) |
| A | [packages/coding-agent/src/core/agent-session.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/agent-session.ts) |
| L | [packages/agent/src/agent-loop.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/agent/src/agent-loop.ts) |
| S | [packages/coding-agent/docs/sessions.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/sessions.md) |
| F | [packages/coding-agent/docs/session-format.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/session-format.md) |
| CO | [packages/coding-agent/docs/compaction.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/compaction.md) |
| SH | [packages/coding-agent/src/modes/interactive/session-share.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/session-share.ts) |
| SE | [packages/coding-agent/src/modes/interactive/components/settings-selector.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/settings-selector.ts) |
| SD | [packages/coding-agent/docs/settings.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/settings.md) |
| AU | [packages/coding-agent/docs/providers.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/providers.md) |
| MO | [packages/coding-agent/docs/models.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/models.md) |
| EX | [packages/coding-agent/docs/extensions.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/extensions.md) |
| LL | [packages/coding-agent/src/extensions/llama/index.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/extensions/llama/index.ts) |
| SEC | [packages/coding-agent/docs/security.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/security.md) |
| R | [packages/coding-agent/README.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/README.md) |
| TOOLS | [packages/coding-agent/src/core/tools/index.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/tools/index.ts) |
| TREE | [packages/coding-agent/src/modes/interactive/components/tree-selector.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/tree-selector.ts) |
| RES | [packages/coding-agent/src/modes/interactive/components/session-selector.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/session-selector.ts) |
| MODEL | [packages/coding-agent/src/modes/interactive/components/model-selector.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/model-selector.ts) |
| PK | [packages/coding-agent/docs/packages.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/packages.md) |
| SK | [packages/coding-agent/docs/skills.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/skills.md) |
| PT | [packages/coding-agent/docs/prompt-templates.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/prompt-templates.md) |
| TERM | [packages/coding-agent/docs/terminal-setup.md](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/docs/terminal-setup.md) |
| UI | [packages/tui/src/tui.ts](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/tui/src/tui.ts) |

Verification: registry extraction checks all 23 registered slash names appear in this report, includes the three dispatcher-only commands and bundled `/llama`; key tables include every matched definition in both registries. Manifest checks local source paths and hashes. This is inventory/link-target validation, not an upstream test suite or runtime UX exercise.

### Published compact source hashes

| Source | SHA-256 |
|---|---|
| `packages/coding-agent/src/core/slash-commands.ts` | `c0bf481c18d6791ce1468f687404e607baf641db041ff2a021d1fa254d6b26a0` |
| `packages/coding-agent/src/modes/interactive/interactive-mode.ts` | `f2c577b8e1b4f3d2e2cdc7903fedd0c001bb001c18009383d5187963f2132469` |
| `packages/coding-agent/src/core/keybindings.ts` | `e140384fc16be1bc8306648b657af8b14d2625bb73c5e4dd3fe5409d1381f7aa` |
| `packages/tui/src/keybindings.ts` | `eea5e3fe258ad50337595bb5320955a6d97bac0c8f6c0a838f988d75079bcf38` |
| `packages/tui/src/components/editor.ts` | `88076036a3ff33da05f347101a6ab56749c8be398d7a35dea1976e62230c4ac5` |
| `packages/coding-agent/src/core/agent-session.ts` | `17116255610ad2a3f3a7f6870a12b14b993ace65fbfa461c4a43d0ad9a013237` |
| `packages/agent/src/agent-loop.ts` | `1e16404a231912fbd7643d8317b15ec4cc6245ed8cedc37582a31d430a1cc6ac` |
| `packages/coding-agent/docs/sessions.md` | `b4ed96521936b5b4ba464496d096bd6c345803f3fa63b5a647674af068cd83b2` |
| `packages/coding-agent/docs/session-format.md` | `cf764cf51830fe260ed8139b400ccc35f248c5d971b8e70367c110e72ecf3a90` |
| `packages/coding-agent/docs/compaction.md` | `f70e26e5d54aa0d9d131dbc49cd316deb0909030fab2b66d6a071b5f0ae72c26` |
| `packages/coding-agent/src/modes/interactive/session-share.ts` | `f5add6fccb39e747507a421d9166113a1a864b8063bfb1e948319dfb6ede6852` |
| `packages/coding-agent/src/modes/interactive/components/settings-selector.ts` | `97f30cbd3eb6bf965781cb554d6b94d4364f7dc338c72dbff991bf9ab45268af` |
| `packages/coding-agent/docs/settings.md` | `034c9ad9ef2dc9efa21258a91b10d20ce106601046d473b73bad1e1c136e430f` |
| `packages/coding-agent/docs/providers.md` | `c8741ab21f78c4df43a9068ff05c06639a2fd961116642722b034756a8c26dc8` |
| `packages/coding-agent/docs/models.md` | `bf322aa290929fb167c56ef024ce33db5752b9422fcb22a8a7cd8b8d6adae759` |
| `packages/coding-agent/docs/extensions.md` | `6eb929fc07fab2d8e9224480f0dd1e0f6087abaa2c696b26b15327a2d22702c5` |
| `packages/coding-agent/src/extensions/llama/index.ts` | `748974470600b6a59d8076c4dde266e2abb7e2c54e63dcb6c7e3649bb8946a58` |
| `packages/coding-agent/docs/security.md` | `a7dd1abbbbb6b7cec21c10fdadd073fe9e77e9eb0e6bf9deec7e86f5f31c043d` |
| `packages/coding-agent/README.md` | `b5daa09877da34d122818e03436b51755bc1b5044ae8ab810df63f02888f1ebe` |
| `packages/coding-agent/src/core/tools/index.ts` | `3c66ea1e03530fb85436ee639f35add8bf72718ac6e76b738a1f41be58a2b472` |
| `packages/coding-agent/src/modes/interactive/components/tree-selector.ts` | `731ce9fe6c07d0b1a55a11ce2987f21771376b7651e28355c98fbdf1f053a931` |
| `packages/coding-agent/src/modes/interactive/components/session-selector.ts` | `b6d2f348e6eb68e17914a4936433912155271f0c3199441e14c419cbab21be10` |
| `packages/coding-agent/src/modes/interactive/components/model-selector.ts` | `92d70b9faffc9febf2ce0c519c7c086938bddfc5da9ad28d819f408d54e3637e` |
| `packages/coding-agent/docs/packages.md` | `1067eae058c5ce980a21b04c51ce6d38e201e51197807037d0a063ac3c769b46` |
| `packages/coding-agent/docs/skills.md` | `e44738f2de44436b1ef56ab64231116fdc68a451213b96b27a5338d6296176c7` |
| `packages/coding-agent/docs/prompt-templates.md` | `972bc0b5461d9140fd543b837abb9b4984e59be66182cbb4f72de894d916dbcb` |
| `packages/coding-agent/docs/terminal-setup.md` | `93084e7e5b5cb28dba4d76704b758a414c93cb91055c765a6336f9df96167d33` |
| `packages/tui/src/tui.ts` | `2ca47c56f4a4f24b8c6a4c9c9f2a06004bfc312d9dbcd0ac1921a2cae32bc675` |
