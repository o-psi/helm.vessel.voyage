# Helm action inventory — UX audit #271

Companion to [the comparative audit](ux-comparison.md). Evidence: source review,
not executed TUI acceptance. Paths below are relative to this document. This
[The comparative recheck](ux-comparative-review.md) qualifies the UX conclusions. This
inventory separates fixed UI actions from runtime-advertised tools: a schema-driven
operator action is not a permanently installed command.

## Source key

- **H1** [completion registry](../../helm/src/process_client/ui/completion.rs), [dispatch](../../helm/src/process_client/ui/actions.rs), [control mutations](../../helm/src/process_client/ui/controls.rs).
- **H2** [input routing](../../helm/src/process_client/ui/input.rs), [composer](../../helm/src/composer.rs), [editor contract](../ui-editor.md), [help](../../helm/src/process_client/ui/presentation.rs).
- **H3** [draft creation](../../helm/src/process_client/ui/new_draft.rs), [voyage finder](../../helm/src/process_client/ui/voyage_picker.rs), [sidebar](../../helm/src/process_client/ui/sidebar.rs), [action rendering](../../helm/src/process_client/ui/sidebar/menu.rs).
- **H4** [transcript input](../../helm/src/process_client/ui/transcript/input.rs), [transcript implementation](../../helm/src/process_client/ui/transcript), [export](../../helm/src/process_client/ui/export.rs).
- **H5** [inference](../../helm/src/process_client/ui/inference.rs), [accounts](../../helm/src/process_client/ui/accounts.rs), [access](../../helm/src/process_client/ui/inference/access.rs).
- **H6** [interactions](../../helm/src/process_client/ui/interactions), [panel contract](../ui-right-panels.md), [operator forms](../../helm/src/process_client/ui/operator.rs), [operator bridge](../../helm/src/process_client/ui/operator_bridge.rs), [Explore](../../helm/src/process_client/ui/explore.rs).
- **H7** [Vessels](../../helm/src/process_client/ui/vessels), [Vessel interaction contract](../ui-interaction.md), [private terminals](../private-terminal.md), [terminal UI](../../helm/src/process_client/ui/terminals.rs).
- **H8** [browser dispatch](../../helm/src/process_client/ui/browser.rs), [local browser](../local-browser.md), [attachments](../../helm/src/process_client/ui/attachments.rs), [paste](../../helm/src/process_client/ui/paste.rs), [previews](../ui-previews.md).
- **H9** [workflows](../../helm/src/process_client/ui/workflows.rs), [inbox](../../helm/src/process_client/ui/inbox.rs), [provider attempts](../../helm/src/process_client/ui/provider_attempts.rs), [provider recovery](../provider-attempts.md).

## Fixed slash commands: complete completion-registry inventory

Arguments are schematic, not literal values. An entry without arguments can open a
picker rather than perform the corresponding mutation. These are full-screen Helm
commands; do not assume plain chat implements them.

| ID | Entry/action | Outcome and UX contract | Evidence |
|---|---|---|---|
| H001 | `/help` | Scrollable help; F1 equivalent. | H1,H2 |
| H002 | `/attempts [RUN_UUID [OFFSET]]`; `/attempts all OFFSET` | Provider attempt history, not permission to repeat an effect. | H9 |
| H003 | `/inbox` | Lists supported inbox syntax; passive metadata panel. | H9 |
| H004 | `/inbox destinations` | Discover destination IDs. | H9 |
| H005 | `/inbox list DESTINATION [AFTER]` | Bounded page (50), explicit cursor. | H9 |
| H006 | `/inbox open DESTINATION EVENT` (`inspect` alias) | Metadata only; does not jump, open URL, or answer a request. | H9 |
| H007 | `/inbox seen DESTINATION EVENT` | Receipt, not resolution of an owner decision. | H9 |
| H008 | `/inbox dismiss DESTINATION EVENT` | Dismiss receipt, not denial/cancellation. | H9 |
| H009 | `/account [search]` | Executing-host private account picker/sign-in; next-run selection. | H5 |
| H010 | `/vessels` | Private connections UI; Ctrl+G/header alternative. | H7 |
| H011 | `/new [absolute workspace]` | Configuration draft; first send creates a voyage. | H1,H3 |
| H012 | `/use UUID` | UUID must identify exactly one route-qualified view; F2 avoids ID entry. | H1,H3 |
| H013 | `/rename NAME` | Revision-bound rename. | H1 |
| H014 | `/branch [NAME]` | Separate voyage, not filesystem rollback. | H1,H3 |
| H015 | `/model [ID]` | Picker or next-turn override; existing thinking/service overrides require review. | H5 |
| H016 | `/thinking [EFFORT]` | Picker; `inherit` clears explicit override. Not a reasoning-display toggle. | H5 |
| H017 | `/service [TIER]` | Picker; inherit and explicit default have different semantics. | H5 |
| H018 | `/models` | Read live executing-host model controls. | H1,H5 |
| H019 | `/configure ABSOLUTE_PATH` | Load next-turn executing-host configuration, not a local-client path. | H1 |
| H020 | `/tools` | Actual runtime tool inventory. | H1,H6 |
| H021 | `/tool NAME JSON_ARGUMENTS` | Runtime-validated operator execution; active-run ExecuteTool versus idle OperatorTool. | H1,H6 |
| H022 | `/policy` | Inspect effective policy. | H1,H6 |
| H023 | `/access [read_only\|approval\|unrestricted]` | Reviewed change within owner authority; not OS sandbox control. | H5 |
| H024 | `/todos` | Task controls; typed management through F8. | H1,H6 |
| H025 | `/subagents` | Delegated-work controls; typed management through F8. | H1,H6 |
| H026 | `/workflows` | Saved workflow trust/input/review/run UI. Earlier special dispatch wins over generic controls branch. | H1,H9 |
| H027 | `/host_resources` | Executing-host cleanup metadata; does not attest cleanup. | H1,H6 |
| H028 | `/terminal` or `/terminals` | Named program inventory; F3 alternative. | H1,H7 |
| H029 | `/terminal UUID` | Exact live terminal attachment; not shell creation. | H1,H7 |
| H030 | `/browser` or `/browser open` | Explicit local browser companion; maximum four retained local browser resources. | H8 |
| H031 | `/browser status` | Observe companion state. | H8 |
| H032 | `/browser takeover` | Request human control; companion must confirm automation fencing. | H8 |
| H033 | `/browser private` | Request private control; no assumed immediate cutoff from request alone. | H8 |
| H034 | `/browser close` | Stop local resource; voyage continues; unresolved cleanup retained. | H8 |
| H035 | `/browser reconcile` | Only after closing; reconcile observed cleanup without repeating uncertain effects. | H8 |
| H036 | `/conversation` | Return from a control panel to transcript. | H1 |
| H037 | `/approve UUID` | Respond to one observed permission request; no blanket authority. | H1,H6 |
| H038 | `/deny UUID` | Deny one request. | H1,H6 |
| H039 | `/answer UUID TEXT` | Question response, not a secret-entry surface. | H1,H6 |
| H040 | `/cancel` | Exact observed active run; requested cancellation is not cleanup completion. | H1 |
| H041 | `/clear FULL_VOYAGE_UUID` | Explicit target confirmation; conversation reset, not filesystem undo. | H1 |
| H042 | `/compact N` | Keep recent N unchanged; retain older user messages and extract older assistant/tool context; canonical history unchanged, no generated summary. | H1,H6 |
| H043 | `/archive` | Archive current voyage; lifecycle guards apply. | H1,H3 |
| H044 | `/archived` | Browse archived views. | H1,H3 |
| H045 | `/voyages` | Return to current views. | H1,H3 |
| H046 | `/restore` | Restore archived voyage. | H1,H3 |
| H047 | `/delete FULL_VOYAGE_UUID` | Confirm history deletion; cannot recall external copies. | H1,H6 |
| H048 | `/export PATH` | Save conversation; local export is distinct from model-host configuration path. | H1,H4 |
| H049 | `/receipt` | Resolve the original pending command, not an automatic resubmission. | H1 |
| H050 | `/quit` | Detach Helm; does not cancel independent voyage execution. | H1,H2 |

## Keyboard, pointer, and modal actions

Each row represents one action or exact inverse pair, not just a feature label.
Context changes are intentional and need scenario verification.

| ID | Action / entry | Observable source contract / failure expectation | Evidence |
|---|---|---|---|
| H051 | Send: Enter | Plain message submits when idle, steers observed active run; retain pending identity under uncertain response. | H1 |
| H052 | Newline: Alt/Shift+Enter | Inserts newline rather than sending where terminal reports modifiers. | H2 |
| H053 | Complete: `/`, Up/Down, Tab | Completion edits draft; Enter dispatches. Esc dismisses completion first. | H1,H2 |
| H054 | Insert/delete text, Backspace/Delete | Grapheme-aware editor; owned image markers are atomic. | H2 |
| H055 | Left/Right; Home/End | Move in draft; Ctrl+Home/End differ between new draft and existing voyage. | H2 |
| H056 | Shift+movement; Ctrl+A | Extend selection/select draft; paste/typing replace selection. | H2 |
| H057 | Ctrl/Alt+Left/Right | Word movement (whitespace-defined). | H2 |
| H058 | Ctrl/Alt+Backspace/Delete | Word deletion. | H2 |
| H059 | Up/Down in wrapped draft | Wrapped-row movement; history/navigation have separate context. | H2 |
| H060 | Alt+Up/Down; one-row Up/Down | Recall submitted prompts in existing voyage; new drafts have none. | H2 |
| H061 | Ctrl+X; Alt+Y | Cut/yank private in-memory text, not OS clipboard; excludes owned images. | H2 |
| H062 | Ctrl+Z; Ctrl+Y/Ctrl+Shift+Z | Local text undo/redo, 64 snapshots, nonpersistent; not file/turn rollback. | H2 |
| H063 | Bracketed paste | Literal payload, selection replacement; private panels intercept first. | H2,H8 |
| H064 | Ctrl+V/Alt+V/Shift+Insert | Explicit clipboard acquisition; failure/cancel retains draft; utility availability matters. | H8 |
| H065 | Image path paste/drag-drop | Local PNG/JPEG/WebP attachment; literal marker text is not image ownership. | H8 |
| H066 | Remove image marker | Removes corresponding owned image; unrelated text retained. | H8 |
| H067 | Alt+P preview toggle | Off initially, metadata fallback, no new capture/read/upload. | H8 |
| H068 | Esc during acquisition | Cancel clipboard work, not run cancellation. | H8 |
| H069 | Ctrl+N | New unsent configuration draft; workspace selected on executing host. | H3 |
| H070 | F2 finder | Search voyages/drafts by name, select exact identity; stale selection refuses. | H3 |
| H071 | Tab/Shift+Tab | Voyage traversal only when completion/form scope does not consume it. | H2,H3 |
| H072 | Sidebar Up/Down/click | Empty-composer or sidebar-focus navigation; retain per-view drafts. | H2,H3 |
| H073 | Drag sidebar divider | Resize; modal/page/resize invalidates obsolete pointer targets. | H2,H3 |
| H074 | F9 / sidebar ⋮ / Right then Enter | Open Actions even when sidebar is narrow; below 20×10 action area source asks to enlarge. | H3 |
| H075 | Actions Access → mode | Read only / Ask first / Unrestricted; full review before confirm, no root broadening. | H3,H5 |
| H076 | Actions Rename | Separate editor, does not commandeer unsent composer. | H3 |
| H077 | Actions Archive / Restore | Availability depends on observed lifecycle. | H3 |
| H078 | Actions Branch | New voyage, optional title, not historical message tree. | H3 |
| H079 | Actions Cancel current run | Exact observed run/target guards. | H3 |
| H080 | Actions Details | Read selected identity and process details. | H3 |
| H081 | Actions Clear | Type `CLEAR`; reviewed revision; no workspace rollback. | H3,H6 |
| H082 | Actions Compact | Type `KEEP N`, 1–100000; extractive working context, not model-generated summary. | H3,H6 |
| H083 | Actions Delete | Type `DELETE`; preserve unsent composer, cannot erase exports. | H3,H6 |
| H084 | F5 current/archive | Changes catalogue filter, not archive mutation. | H2,H3 |
| H085 | F1 help, Up/Down/Page keys, Esc | Scroll/close help; help text itself needs context-accurate shortcut wording. | H2 |
| H086 | F8 Explore | Eight entries: Run a tool, Access, Manage tasks, Delegated work, Saved workflows, Models, This machine, Policy details. | H6 |
| H087 | F8 typed tool action | Schema fields/enums/booleans/lists, task/agent references; review then ordinary dispatch. Dynamic leaves come from executing runtime, not a hard-coded competitor parity list. | H6 |
| H088 | Permission focus/choose/confirm | Up/Down/Tab choices, Enter response; stale/expired request cannot be answered. | H6 |
| H089 | Esc at permission request | **Denies**, despite generic help saying Esc goes back. | H6 |
| H090 | Question choose/custom/submit | Bounded custom text; not a password surface. | H6 |
| H091 | Esc at question/custom answer | Question skips; custom editor returns to choices preserving draft. | H6 |
| H092 | Access review scroll | Confirm disabled until last review lines visible. | H5,H6 |
| H093 | PageUp/PageDown / wheel | Read transcript with anchor; older loading is bounded. | H4 |
| H094 | Ctrl+Home | Load earlier messages; not necessarily entire history at once. | H4 |
| H095 | Ctrl+End | Return to latest output/follow state. | H4 |
| H096 | Ctrl+F, text, Enter | Find within loaded messages; no proof of exhaustive history search. | H4 |
| H097 | Esc in search | Close search; retain composer. | H4 |
| H098 | Ctrl+T | Expand/collapse activity groups, reconcile reading anchor. | H4 |
| H099 | Double-click saved tool call | Expand/collapse persisted detail; not live argument-generation parity. | H4 |
| H100 | Click coordination sender | Open attributed source target; not ordinary text becoming executable. | H4 |
| H101 | Model/Thinking/Service click | Search capability-backed choices; unknown entitlement stays unknown. | H5 |
| H102 | Change model with overrides | Explicit keep/reset review; cancel retains prior settings. | H5 |
| H103 | Account picker select/search | Executing-host account; running vs next account can differ. | H5 |
| H104 | Account usage refresh / default | F5 refreshes private usage observations; F6 sets default account; observations are not inferred live billing. | H5 |
| H105 | Account setup/sign-in/recover | Named connections/accounts, private device flow; retry/recovery must keep admitted operation identity. API secrets belong in host private CLI. | H5 |
| H106 | Ctrl+G / header Vessels | Open private connection management before clipboard/composer routing. | H7 |
| H107 | Pair / import connection | Endpoint, invitation for pairing, alias, reconnect toggle, validation/review. Credentials are not prompt text. | H7 |
| H108 | Rename connection | Alias editor; Enter submit or Tab to explicit action. | H7 |
| H109 | Reconnect / access preview | Current route identity; remembered connection does not broaden Voyage policy. | H7 |
| H110 | Forget connection | Starts on Back; explicit save/confirm activation; preserve unresolved setup intent. | H7 |
| H111 | Form Tab/Shift+Tab, click, Ctrl+U | Scoped focus/field clearing; paste on action consumed, not inserted elsewhere. | H7 |
| H112 | F3 inventory, Up/Down, Enter | Attach named live program; stale inventory is not proof of liveness. | H7 |
| H113 | Private terminal direct input | Permanent model capture/write cutoff for that terminal; disclosed prior output cannot be recalled. | H7 |
| H114 | Ctrl+] while attached | Detach to retained composer, not stop child. | H7 |
| H115 | Ctrl+C while attached | Sent to child, unlike global Helm quit behavior. | H7 |
| H116 | Disconnect during terminal input | Not buffered/replayed; no terminal survival claim across owner restart. | H7 |
| H117 | F6 local browser companion | Local consent/capture and remote execution policy both apply. | H8 |
| H118 | Return browser to agent | Explicit review in companion, deliberately no blind slash resume. | H8 |
| H119 | Workflow select | Inventory → details/trust review. | H9 |
| H120 | Workflow trust (`t`) | Explicit trust action, not selection alone. | H9 |
| H121 | Workflow input/Enter | Sequential parameter validation; secret bindings are not chat input. | H9 |
| H122 | Workflow confirm (`y`)/Esc | Explicit dispatch / leave, retain admitted operation tracking. | H9 |
| H123 | Ctrl+C/Ctrl+Q globally | Quit Helm, **not** stop voyage; private terminal is a separate context. | H2,H7 |
| H124 | Unknown slash/invalid JSON/stale target | Refuse with error, do not send as an ordinary prompt or substitute another effect. | H1 |

| H125 | Vessels `w`: new on selected host | Opens host-scoped draft, not agent execution. | H7 |
| H126 | Vessels `v` / `l`: selected host / all | Catalogue filtering; absent remote selection selects local scope. | H7 |
| H127 | Vessels `c` / `d`: connect / disconnect | Explicit route activation/detachment, including local route; disconnect does not cancel voyage. | H7 |
| H128 | Vessels `u`: retry unavailable | Retry route observations, not uncertain commands. | H7 |
| H129 | Vessels `n`: replace access | New immutable connection identity; old drafts/pending deliveries remain separate. | H7 |
| H130 | Vessels `t`: automatic reconnect | Toggle preference, not permission expansion. | H7 |
| H131 | Vessels `o`: restore forgotten | Restore original credential with auto-reconnect off; explicit connect observes receipts, no replay. | H7 |
| H132 | Vessels `p`: resume setup | Continue selected pending setup identity. | H7 |
| H133 | Account settings Tab/Shift+Tab, Ctrl+U, F2 | Traverse model/effort/service fields, clear field, use defaults; Enter applies after review, Esc cancels while running work continues. | H5 |
| H134 | Account sign-in `r` | Poll retained enrollment, or refresh accounts after succeeded enrollment. | H5 |
| H135 | Account sign-in `n` | Explicit restart enrollment through lifecycle handling, not implicit effect retry. | H5 |
| H136 | Account sign-in `c` | Cancel tracked enrollment; requested is not observed cancellation. | H5 |
| H137 | Account sign-in `o` | Explicit fixed provider URL launch only with active private material; no automatic browser launch. | H5 |
| H138 | Account list device/API-key help entries | Device connection/alias selection; API key entry explains executing-host private CLI and refuses chat-secret model. | H5 |

## Specific findings from this inventory

1. **Cancellation muscle memory is hazardous (H089, H115, H123).** Generic Esc help
   says “Go back without losing your draft,” but permission Esc denies and question
   Esc skips. Ctrl+C means quit in Helm and interrupt inside attached programs.
   Context-visible labels and tests are required; do not copy another CLI's key
   mapping without preserving detach-versus-stop semantics.
2. **Help history wording is less precise than editor behavior (H059–60).** Help
   says Up/Down recall earlier messages while editing, while wrapped-row movement,
   empty-composer navigation and Alt+Up/Down introduce important exceptions.
3. **“Compact” is easy to misinterpret (H042/H082).** Label it as retaining recent
   context, with explicit omitted-message and canonical-history consequences.
4. **Find is loaded-history search (H096).** Show scope and offer explicit older
   loading; do not present zero matches as an exhaustive result.
5. **The fixed command set is only one layer (H087).** Runtime-advertised tools,
   remote account capabilities and workflow definitions require instance-specific
   inventories. A static document cannot certify all possible extension actions.
6. **Small-window access is bounded (H074).** F9 makes the entry available, but
   the action renderer still has a minimum size. This is a source-observed bound,
   not a measured usability score.
