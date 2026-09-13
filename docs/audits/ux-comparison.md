# Action-by-action UX audit: Helm, Pi, OpenCode, Codex CLI

Issue: [#271](https://github.com/o-psi/voyage/issues/271). Research date:
2026-09-13 UTC. **This is a comprehensive source-level action audit, not a
completed hands-on usability study or an implementation delivery.**

## Read this first

Helm has substantial operational capability, but capability breadth is not the
same thing as a coherent interaction model. The strongest opportunities are:

1. **Make intent and state unmistakable:** send versus steer versus follow-up;
   stop versus detach; pending versus delivered; omitted context versus summary;
   conversation branching versus file rollback.
2. **Reduce discovery fragmentation:** command completion, F8 Explore, F9 Actions,
   sidebar, account controls and private panels are distinct discovery surfaces.
   Build one searchable action catalogue backed by the real handlers, while
   preserving private input and execution authority boundaries.
3. **Make coding work easier to inspect:** live tool-call construction, separate
   reasoning visibility, first-class scoped diffs, easy response copy/export and
   historical message navigation deserve attention before adding more controls.
4. **Keep Helm's advantages:** independent voyages, explicit executing-host
   identity, durable command recovery, private terminal takeover and opt-in local
   browser consent. Do not replace these with a local-CLI mental model.

There is **no defensible overall winner or numeric usability score** from source
alone. Pi provides especially useful queue/tree/editor patterns; OpenCode provides
broad palette and session-inspection patterns; Codex provides coding-review,
permission and session-recovery patterns. All three have context-dependent keys,
configuration gates and their own complexity.

## Evidence, versions, and scope

| Product | Inspected source | Surface |
|---|---|---|
| Helm | Base `09fffcaca7fcc868b8f05eae509042f1a977e796`; working-tree render change present | Current full-screen TUI, fixed commands, CLI leaves, private/remote operator controls |
| Pi | `71dca871bc80b6bc97be37f0ca3189399d651fff` | `packages/coding-agent`, TUI/agent dependencies; requested `badlogic/pi-mono` resolves to `earendil-works/pi` in observed GitHub metadata |
| OpenCode | `95daf90670b7c039c436c85537da5fbfe2205b41` | Terminal UI and supporting session/config handlers, not desktop/web |
| Codex CLI | `36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` | Rust terminal/CLI and supporting core; experimental/platform gates are recorded separately |

These are inspected upstream HEAD snapshots, **not claims about installed stable
releases**. They can include newer or gated functionality. Each competitor appendix
contains immutable source links. Local Helm links are delivery-relative so the audit
can be maintained; the base and fingerprint identify the reviewed source snapshot.

Helm `helm/src/**/*.rs` fingerprint at capture:
`758414c2e6247b3ef56fc95f74b6a31efe5ba7e58aa86daeaa2532005438b8ec`.
Computed as SHA-256 of sorted-key JSON mapping relative paths to file SHA-256s.
Local full manifest: ignored `target/ux-audit/helm-source-manifest.json`.
Concurrent unrelated work advanced the base from `cb2fd96` to `09fffca` during
inspection; existing script/test deletions and a staged render edit were not ours.
No assertion depends on those changes having passed runtime checks.

**Performed:** source and documentation inspection, GitHub issue review, public
upstream metadata/archive retrieval, registry/handler enumeration, documentation
consistency checks. **Not performed:** interactive four-product journeys, screenshot
comparison, timing/keystroke measurements, screen-reader testing, native platform
checks, live authentication or paid model calls. Existing documented tests are
historical context, not newly executed evidence.

### Inventory and evidence index

- [Helm: 138 fixed command/UI action rows](ux-helm-actions.md), including aliases,
  modal consequences, discovery and recovery. H001–H138 are stable review IDs.
- [Helm CLI leaf inventory](ux-helm-cli-actions.md), including nested setup,
  provider, process, notification, workflow and administration grammar.
- [Pi commands, key registry, selectors, settings and runtime semantics](ux-pi-actions.md).
- [OpenCode palette/slash/key/session actions](ux-opencode-actions.md).
- [Codex CLI slash/key/CLI/runtime actions and gates](ux-codex-actions.md).

“Comprehensive” means fixed core entry points and meaningful action families were
inventoried, not that every runtime tool schema, user extension, plugin, custom
workflow or terminal/OS combination was exercised. Competitor-only actions remain
in their appendices rather than disappearing from a Helm-first matrix. Do not infer
CLI parity from the TUI comparison or turn an unbound action into a default key.

### How to interpret the comparison

Every cell below is **source-supported**, unless marked **NV** (not verified) or
**NI** (no matching first-class action found in inspected fixed UI). NI is not proof
that a shell, plugin or agent tool could not perform the task. **Different** means
similar entry names have materially different effects. A gate is not default
availability. The action inventories provide the exact command, key and source
for the compressed descriptions below.

## Comparative action matrix

### A. Start, discover, and compose

| Action | Helm | Pi | OpenCode | Codex CLI | Assessment / next Helm check |
|---|---|---|---|---|---|
| Start interactive work | Helm attaches through Vessel; unsent config drafts are not sessions | Interactive coding agent, session persistence | TUI with session/project context | TUI with onboarding/trust/config context | H011/H069: distinguish ready-to-compose from created/running |
| One-shot/headless run | `helm run`; separate managed/workflow surfaces | Print/JSON/RPC modes | Run/serve/attach CLI surfaces | Exec/exec resume/review CLI | Do not count headless commands as keyboard journeys; see CLI leaves |
| Choose workspace | Executing-host workspace selection | Cwd/session directory semantics | Directory/project/session context | Startup directory, session cwd and worktree flows | H011: always label which machine owns path |
| Discover actions | `/` completion + F1 + F8 + F9 + private panels | `/` + `/hotkeys` + settings; extension commands | Command palette + slash aliases + keybinding registry | Slash menu + help/keymap; feature-filtered commands | Helm's split catalogue adds search burden; no measured speed claim |
| Change keybindings | NI unified in-app configurable keymap | Keybindings JSON and `/hotkeys` | Configurable bindings, leader patterns | `/keymap`, `/vim`, configurable action registry at pin | Add effective per-context key help before wholesale rebinding |
| Submit ordinary prompt | Enter; running state changes it to Steer | Enter; while streaming queues steering | Submit with session status handling | Submit/steer with turn state handling | H051: show delivery intent before send |
| Explicit follow-up after current run | NI dedicated composer queue action | Alt+Enter follow-up queue | Busy-session prompt handling; exact queue semantics in appendix | Enter submit/steer, Tab queue (context-sensitive); queued-message UI | Separate “after completion” from “steer now”; don't label all as send |
| Inspect/edit queued message | Pending command receipt is not editable message queue | Alt+Up retrieves queued messages; one-at-time/all settings | See prompt history/stash and session behavior; not equivalent to durable receipts | Queue editor actions | Add cancellation/edit boundary only before immutable admission |
| Multiline newline | Alt/Shift+Enter | Shift+Enter/Ctrl+J | Keybinding-configured newline alternatives | Keymap/newline actions; terminal protocol dependent | Modifier transmission needs actual terminal tests |
| Move/select Unicode text | Grapheme movement, selection, word edit | TUI editor registry | Prompt editor/keybindings | Textarea/editor/keymap | H054–59: test emoji, ZWJ, wrapping, mixed-width text |
| Recall prompt | Alt+Up/Down; one-row context also Up/Down | History keys | Prompt history | Composer history | H060: help currently oversimplifies context |
| Undo/redo draft | Ctrl+Z / Ctrl+Y or Ctrl+Shift+Z, bounded, nonpersistent | Editor undo; not filesystem undo | Input undo/redo | Editor actions distinct from rewind | Label scope; test attachment ownership separately |
| Cut/copy/yank draft | Ctrl+X/Alt+Y private text buffer | Kill/yank/copy registry | Editor/clipboard actions | Selection/editor/clipboard actions | H061 is not OS clipboard; disclose this difference |
| Paste multiline text | Bracketed paste and explicit acquisition | Large paste blocks/editor handling | Paste with prompt handling | Paste burst handling / large paste representation | Test failed acquisition and stale destination, not just happy path |
| Attach image | Local image bytes via clipboard/path | Image paste and file arguments | Clipboard/file attachment actions | Image paste and CLI image flags | Terminal preview support is separate from model capability |
| Preview/remove attachment | Owned atomic markers, optional Alt+P bounded preview | Image rendering toggle and editor attachments | Prompt attachment UI | Image composer representation | H065–67: losing preview must not lose attachment |
| Mention file/symbol | NI dedicated `@` composer file/symbol picker in fixed completion | `@` file completion | `@` file/agent discovery | `@` file mention/fuzzy finder | High-value coding ergonomics gap; shell/path paste is not equivalent |
| Open external editor | NI fixed composer action | Ctrl+G external editor | External editor action | External editor action | Candidate P2; preserve text/attachments and cancellation on round-trip |
| Run shell from composer | Explicit operator tool, not implicit shell prefix | `!` and `!!` distinguish context inclusion | Shell-mode prompt | Shell command/editor support | Do not add prefixes that bypass Voyage policy or host labels |
| Stash prompt | Per-voyage retained drafts; NI named stash menu | Queue retrieval/editor mechanisms | Stash/stash-pop/list prompt actions | Per-thread drafts/queue editing | Retention already valuable; assess need before copying stash complexity |

### B. Observe and control execution

| Action | Helm | Pi | OpenCode | Codex CLI | Assessment / next Helm check |
|---|---|---|---|---|---|
| See active state | Voyage/process/run and provider attempt observations | Streaming indicator, footer, tool output | Session busy/status and title/tool UI | Status/turn/tool UI | Distinguish liveness from forward progress |
| See model reasoning | `/thinking` config is not display; live display tracked in #255 | Reasoning display toggle separate from effort | Reasoning display modes | Reasoning summary/raw display controls with gates | Keep setting and display separate; don't invent unavailable reasoning |
| Watch tool arguments form | Full parity not established; #255 open | Partial tool UI update lifecycle | Tool parts rendered as state evolves | Tool/exec/MCP activity rendering | Existing saved-detail expansion is not live-generation evidence |
| Watch tool output | Persisted/live activity and owned process boundaries | Incremental tool updates | Tool output/detail toggles | Execution output/tool cells | Compare truncation, ordering and interruption with same fixture |
| Expand/collapse details | Ctrl+T, double-click saved call | Ctrl+O tool expansion, thinking toggle | Tool details/generic-output/conceal toggles | Transcript/raw/reasoning display controls | H098–99: preserve read anchor across reconciliation |
| Read while new output arrives | Transcript anchoring and bounded older loading | Inline/alternate-screen navigation | Page/half-page/line/message navigation | Transcript view/scrolling | Need real timing/layout evidence before claiming stability |
| Stop current model run | `/cancel` or F9 action | Esc abort | Session interrupt action | Interrupt action | Helm lacks same obvious Esc stop convention; expose Stop without changing private controls blindly |
| Quit/detach client | Ctrl+C/Ctrl+Q, `/quit`; voyage continues | Quit/shutdown session process | Exit/connected server distinctions | Exit/detach session behavior | Helm must explicitly say “work continues” |
| Interrupt attached program | Ctrl+C goes to child | Shell execution abort paths | Tool/session interrupt | Exec/tool interrupt | Not the same target as cancelling whole voyage |
| Recover transient provider error | Bounded durable attempt recovery; `/attempts` | Auto retry settings and abort | Retry/status behavior | Stream retry/error handling | Display reason/count/deadline and eventual outcome, never replay uncertain tools |
| Recover disconnect | Reconnect owner; original receipt identity | Local agent/session process model | Server/attach architecture differs | Local/remote control configurations differ | Architecture is not usability credit without an executable recovery journey |
| Resolve uncertain command | `/receipt`, retained command ID | No equivalent Voyage receipt protocol | NI matching Helm protocol | NI matching Helm protocol | Keep receipt recovery accessible from pending state, not only UUID commands |
| Approve/deny execution | Owner policy, scoped request, typed/panel response | No core execution approval/sandbox equivalent; resource trust differs | Permission panel/rules | Permissions, approval and sandbox controls | Pi convenience cannot be treated as equivalent safety |
| Dismiss permission dialog | Esc denies | Core comparison N/A | Reject/dismiss behavior in panel | Approval response actions | H089 contradicts generic “Esc back” mental model |
| Answer structured question | Choices/custom/skip | Extension-provided UI, not same core question tool | Question tool UI | Request-user-input/elicitation paths | Distinguish answer, skip, denial and timeout |
| Enter a secret | Host private CLI/terminal, never question/chat | Provider login flow; extensions differ | Provider connect/auth flow | Login/auth flow | Never compare generic input widgets as equivalent secret boundaries |
| Change execution permissions | Reviewed Access within runtime ceiling; app policy not OS sandbox | Resource trust is different | Permission config/rules | Sandbox + approval controls; platform/gates | Show current and next authority plus affected host |

### C. Navigate, preserve, and inspect work

| Action | Helm | Pi | OpenCode | Codex CLI | Assessment / next Helm check |
|---|---|---|---|---|---|
| Find/switch session | F2 searchable route-qualified voyages/drafts; Tab | `/resume`, session selector | Session list/palette | `/resume` / picker | Test duplicate titles across hosts and preserved draft |
| Rename | Slash/F9 | `/name` | `/rename` | `/rename` | Low conceptual gap; stale target and title wrapping still matter |
| New conversation | Draft then first-send creation | `/new` | New session | `/new`, `/clear` semantics | Do not conflate clearing history and creating identity |
| Branch/fork | `/branch` separate voyage | `/tree`, `/fork`, `/clone` differ | Fork selected historical message | `/fork`, rewind/backtrack/session worktree flows | Helm branch not equivalent to message-level tree navigation |
| Navigate historical branch tree | NI dedicated conversation tree | First-class `/tree` with filters/labels | Timeline/fork rather than same tree | Backtrack/fork rather than Pi tree | Candidate P2; maintain canonical owner and context disclosure |
| Jump to message | Search/earlier/latest | Semantic prompt jumps/tree | Timeline and message jumps | Transcript/backtrack actions | Add next/previous user message before elaborate tree UI |
| Search conversation | Ctrl+F **loaded messages** | Alternate-screen search/tree search | Timeline/session search surfaces | Transcript/session selection capabilities; see inventory | Always expose search scope; cross-history parity NV |
| Copy last response | NI fixed first-class response-copy action found | `/copy` | Copy last assistant message | `/copy` | Small, high-value action; separate canonical text from styled rendering |
| Copy full transcript | Local export, not same as clipboard copy | `/export` HTML and copy response | Copy transcript | Export/transcript actions | Disclose scope, omitted content and sensitive material |
| Export | `/export` local Markdown destination | HTML export | Transcript export | Markdown export at pin | Formats and remote/local destination differ |
| Public share | NI built-in upload/share | Share destinations differ; see warning in Pi appendix | Share/unshare session URL | Local export; other app/share surfaces not assumed | Not priority parity; uploading needs explicit destination/audience consent |
| Compact context | Keep N, omission marker, canonical history retained | Generated summary / automatic compaction | Session summarization | Context compaction | **Material semantic gap**; “Compact” should not suggest summary parity |
| Clear conversation | Explicit UUID or `CLEAR` review | New/tree context flows | New/undo session flows differ | New/clear session behavior | State identity/history consequences before confirmation |
| Archive/restore | Explicit lifecycle and catalogue | Session manager operations differ | Session listing/deletion; no assumed archive equivalence | Archive command at pin | Preserve cleanup obligations and exact target |
| Delete | Explicit confirmation, cannot recall copies | Session selector deletion | Session delete UI | Delete command with gating/state constraints | Actual copy/remote deletion is not implied |
| Review working-tree diff | NI dedicated fixed TUI diff/review action; tools can inspect | No built-in review parity assumed; tools/extensions | File/diff/session undo UI | `/diff` (unstaged + untracked, not staged-only), `/review` | First-class read-only scoped diff is high-value P1 |
| Rewind files/turn | Composer undo/branch do not restore workspace | Tree does not mean file rollback | Undo/redo includes snapshot/revert semantics | No `/undo` in this pin; conversation backtrack and editor undo are distinct | Never market a generic Undo without naming affected state |
| Create isolated worktree | Runtime tool/subagent mechanism, not fixed coding-session flow | Shell/extensions | Worktree/workspace features depend on surface/config | `/worktree` and CLI flows at pin | Useful only with lifecycle, dirty-work preservation and failure recovery |

### D. Configure, extend, and operate beyond a local coding session

| Action | Helm | Pi | OpenCode | Codex CLI | Assessment / next Helm check |
|---|---|---|---|---|---|
| Select model | Capability-backed picker / `/model` | Picker/default/scoped cycling | Model list/recent/favorite cycling | `/model` | Entitlement unknown ≠ unsupported |
| Change reasoning effort | `/thinking`, explicit override review | Cycle thinking / settings | Model variant cycling | Model/effort selection | Show effective versus inherited value |
| Change service/cost preference | `/service`, host/provider distinctions | Provider/model settings | Provider/model configuration | Fast/service feature behavior with gates | Do not imply identical billing contracts |
| Inspect usage | Private host account observations and provider attempts | Footer/session counters/cost/context | Session status/context/sidebar | `/status`, `/usage` at pin | Timestamp/cache/unknown labeling matters more than apparent precision |
| Login/logout/provider setup | Host provider CLI plus private account UI | `/login`, `/logout`, bundled provider helpers | `/connect`, auth/provider CLI | Login/logout/provider CLI | Host credentials must not migrate because a picker feels local |
| Review settings | Model controls, `/configure`, CLI config, separate panels | `/settings` plus JSON-only settings | Config/keybindings/theme palette | Config/features/keymap/model commands | Candidate searchable settings map, not one giant unsafely shared form |
| Load project guidance | Onboard/host configuration/runtime instructions | AGENTS/skills/templates/resources trust | Init/agents/skills/config | `/init`, skills/instructions/import gates | Show source/provenance and precedence; external content is not authority |
| Reload extensions/tools | Runtime-authoritative registry; package/workflow CLI | `/reload`, package manager, extensions | Plugins/MCP/custom commands | Skills/MCP/plugins/apps with gates | Never imply plugins are all installed or universally safe |
| Plan/review modes | User prompt/workflow/runtime tools; NI matching fixed mode switch | Prompt/extensions rather than assumed built-in mode | Agent selection (including plan/build configuration) | `/plan`, `/review`, experimental goal/other modes | Define deliverable and mutation rights, not merely a badge |
| Manage subagents | F8 typed runtime tools, durable bounded accounting | Extensions, not core parity | Agent/child session navigation/background subagents | Agent picker/multi-agent features and gates | Need parent/child outcome and cleanup visibility, not just number spawned |
| Manage tasks | `/todos`, F8 typed actions | Tools/extensions | Task/todo runtime and display | Plan/task display/runtime | Compare actual editing/ownership, not presence of a checklist |
| Run saved workflow | `/workflows`: trust → inputs → confirm | Templates/extensions/packages | Custom commands/skills/plugins | Skills/prompts/hooks/plugins | Helm explicit review is valuable; measure the extra navigation cost |
| Connect remote execution host | Private Vessels panel, scoped routes | No matching Vessel surface | Attach/server and remote options, different authority model | Remote/app-server modes, different authority model | Separate comparison, not simple feature checkmark |
| Pair/import/rename/reconnect/forget host | Scoped forms, Back-first destructive review | N/A to core | Not equivalent to provider connect | Not equivalent to provider login | H107–11 require dedicated private-state journeys |
| Take over named terminal | F3, direct local human path, permanent model-capture cutoff | No matching core guarantee | Shell execution isn't same guarantee | Exec interaction isn't same guarantee | Preserve as Helm differentiator; test privacy cutoff and reconnect |
| Share local browser | F6 companion, opt-in dual authorization, takeover/private/close/reconcile | Browser extension capability not equivalent | Browser/MCP capability not equivalent | Browser/MCP/app capability not equivalent | Do not adopt separately networked general browser executor |
| Observe notifications | Passive inbox metadata/receipts | UI/extension notifications | Toast/notification configuration | Status/notification hooks | Read/seen/dismiss must not answer permission or attest cleanup |
| Diagnose/operate host | Doctor, process, sessions, account, policy, notifications and other CLI leaves | CLI/config/debug/package surfaces | Debug/serve/auth/MCP/config surfaces | Debug/exec/MCP/features/sandbox surfaces | Full leaves in appendices; don't force admin grammar into composer |

## Prioritized findings and acceptance targets

Priorities are audit recommendations, **not claims of reproduced bugs or scheduled
implementation**. P0 means potential intent/safety confusion; P1 is daily-workflow
friction; P2 is a larger or preference-dependent enhancement.

| Finding | Priority / confidence | Evidence | Acceptance target |
|---|---|---|---|
| U01: Stop, detach, deny and back are conflated by key expectations | P0 / high source confidence | H089/H091/H115/H123; Pi Esc abort; competitor interrupt registries | Every active context names Esc/Ctrl+C outcome; one discoverable Stop action names exact run; detach says work continues; no changed privacy routing |
| U02: Send does not expose steering/follow-up intent clearly enough as a unified contract | P1 / high semantics, UX impact hypothesis | H051, Pi queues, Codex queue controls | Composer shows idle submit vs active steer; admitted/delivered distinct; explicit after-run queue only if owner supports it; recover unsent text safely |
| U03: Compact label encourages false equivalence to summarization | P1 / high | H042/H082 versus all three compaction implementations | Preview retained/omitted scope and canonical-history effect; rename current action or implement separately specified generated summarization |
| U04: Action discovery split across too many surfaces | P1 / high topology, unmeasured cost | H001/H053/H074/H086/H106; competitor catalogues | Searchable action catalogue with scope, shortcut, disabled reason and destination; generated from handlers; private fields never leak to general composer |
| U05: Live tool-call generation/reasoning inspection incomplete | P1 / open implementation scope | [#255](https://github.com/o-psi/voyage/issues/255), H098–99, competitor tool lifecycle | Slow/interleaved provisional calls visible before execution; no premature execution/duplicate final calls; reasoning-display capability is separate from effort |
| U06: Coding result inspection requires too much tool/chat indirection | P1 / NI fixed UI | Diff/review/copy/mention rows above | Add first-class copy response and read-only scoped diff/file navigation; staged/untracked/binary/remote-host scope explicit; no silent rollback |
| U07: Search scope and navigation granularity need clearer affordances | P1 / high scope, usability hypothesis | H093–97, Pi tree/search, OpenCode timeline | Label loaded-history search; offer earlier fetch and next/previous user-message jumps; stable anchors while streaming and resizing |
| U08: Permission timeout must not look like user denial | P0 / existing open issue, not newly reproduced | [#258](https://github.com/o-psi/voyage/issues/258) | Distinct denied/expired/cancelled/failed states in transcript and request panel; never attribute expiry to human choice |
| U09: Remote/local/next-turn scope needs consistent visible grammar | P0 / architectural risk, not new exploit | H009/H019/H023/H048/H107/H113/H117 | Each consequential action names executing host, target voyage and effective time; private auth/browser consent not generalized into Voyage authority |
| U10: Context-aware help and terminal fallback need executable acceptance | P1 / documented/source mismatch | H055/H060/H074/H085; terminal-dependent competitor chords | Help derived from effective scope; all retained actions reachable keyboard-only at supported sizes; no unknown-key fallback sends text |
| U11: Historical context branching is less direct | P2 / high NI finding | H014/H078 versus Pi tree, OpenCode timeline/fork, Codex backtrack | First add historical-message branch preview; clearly distinguish new identity/context from filesystem change; preserve canonical history |
| U12: Configuration/keymap/extension discoverability can improve | P2 / high inventory, preference-dependent | Configuration rows and leaf appendices | Show effective values, source and next-run timing; list extension provenance/gates; configurable keys only with collision/focus validation |

**Recommended order:** U01/U08/U09 semantics and existing failure fixes; then
U02/U03/U04/U05/U06/U07/U10 daily-loop improvements; then U11/U12. Copy useful
patterns, not competitor keybindings wholesale. No code change is included here.

## Executable follow-up: not run in this audit

Use an offline scripted provider and disposable workspace for the shared journeys.
A real provider trial is a separate approved account/budget task. Record exact
binary version, config/features, terminal capabilities, screen dimensions and
initial state. Do not compare a default build against another product's enabled
experimental features without marking that difference.

For each case record: entry discovery, actions/keystrokes, focus, feedback latency,
final visible state, persisted outcome, recovery path, screenshots or terminal
capture where safe, and pass/fail/blocked. Never capture private terminal input,
credentials or browser-private frames.

| Case | Actions / variants | Required oracle |
|---|---|---|
| X01 First task | Fresh setup → workspace → account/model → multiline prompt → send | Correct host/identity, exactly one task, draft retained on refusal |
| X02 Composer stress | Unicode, selection, cut/yank, undo/redo, long paste, 4 images, cancelled/failed acquisition | Exact authored text/owned attachments; no stale insertion or implicit send |
| X03 Busy messaging | Type during stream, steer, after-run follow-up where supported, edit pending text | Explicit queue/admission/delivery state; no duplicate message or silent loss |
| X04 Stop/detach | Esc/Ctrl+C in composer, permission, question, terminal; stop via action; reconnect | Correct target and meaning; detached voyage not reported cancelled; denial attribution exact |
| X05 Tool visibility | Slow argument chunks, interleaved tools, long output, malformed/interrupted call | Provisional versus final distinguished; no premature tool execution; canonical final text |
| X06 Permission outcomes | Approve, deny, Esc, expiry, stale request, policy downgrade | No authority broadening; denied versus timed out distinct; safe draft/focus |
| X07 Questions | Choice/custom/back/skip, multiline paste, invalid/stale response | Only intended answer; no secret prompt; expired request not answered |
| X08 Read while running | Scroll/search/load older/expand call/resize then latest | Stable anchor; truthful search scope; no duplicate history after reconcile |
| X09 Sessions | Two same-title sessions on different hosts; draft switch; rename/branch/archive/restore | Exact identity, composer preserved, no implicit session creation or wrong target |
| X10 Compact/clear/delete | Cancel review; stale revision; correct confirmation | Declared context/history effects only, no file rollback, no external-copy erasure claim |
| X11 Coding inspection | Dirty staged/unstaged/untracked/binary files; diff/review/copy/export/undo where available | Scope disclosed; no overwritten user edits; canonical text versus terminal styling |
| X12 Failure recovery | Lost command response, transport disconnect, transient provider error, owner death | Recover original identity; bounded retry; no uncertain external-effect replay or false survival |
| X13 Host-private work | Pair/import/forget cancel; account enrollment interrupted; named terminal takeover | No secrets in model history; cutoff observed; unresolved setup identity retained |
| X14 Browser | Open/share, takeover/private, disconnect, close/reconcile, failed cleanup | Both authorization boundaries; no hidden capture or uncertain replay |
| X15 Workflow/operator | Unknown schema, task references, trust cancel, missing input, rejected dispatch | No fabricated capability, no automatic retry under new command ID |
| X16 Layout/accessibility | 80×24, 120×40, very narrow, resize mid-modal, keyboard-only, no-color/16/256/truecolor | Reachability/focus/non-color cues; explicit unsupported size; no native-platform overclaim |

Run shared cases in all four products; mark H-specific private host/browser cases
not applicable to products without that boundary. Add new instance-specific rows
for installed plugins, user commands and runtime tool schemas. Existing
[UX readiness](../ux-readiness.md) remains the executable acceptance owner; this
audit does not close that work or #255/#258.

## Maintenance and verification

Audit-only publication: Markdown sources, links, command registries, CLI leaf
manifest, action IDs and diff were checked. Final static validation covered **739
pinned source citations**, **87 relative links**, all **40 Helm completion commands**
and **138 unique Helm action IDs**, with no missing paths, out-of-bounds cited lines
or inconsistent Markdown table columns. Source links were checked against downloaded
pinned trees, not individually fetched over HTTP. The verification record is retained
in ignored `target/ux-audit/validation.json`. No Rust source/manifests/tests changed
by this delivery, so Rust compilation and coverage were not rerun; the prior
coverage measurement is not evidence for this audit. Detailed downloads/reports
remain ignored under `target/ux-audit/`; compact source manifests and immutable
links are included in the appendices. Update pins and re-enumerate actions before
using this document as a future release-parity claim.
