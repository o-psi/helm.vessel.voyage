# Comparative UX findings: evidence review and journey synthesis

Follow-up to [the original comparison](ux-comparison.md) and
[#272](https://github.com/o-psi/voyage/issues/272). Review date: 2026-09-13 UTC.

## What this review changes

The earlier inventory establishes many controls, but does not establish that one
product is easier to use. The relevant question is **how an interface carries a
person from intention to outcome while keeping their place, attention and control**.
A command-count comparison cannot answer that question.

This review rechecks the existing Pi, OpenCode and Codex comparisons against their
original pinned source trees. It does not broaden the product sample or silently
update versions. It follows renderer, focus, dispatch and recovery code where
needed. **No four-product interactive journeys were executed.** Screen arrangements
below are source-derived; perceived clarity, latency, focus surprise and interaction
cost remain hypotheses to test. This is a corrected design assessment, not a
hands-on usability verdict.

### Evidence rules

- **Confirmed mechanics:** a reached handler and relevant state/render path, not
  just a declared key or documentation label.
- **Qualified comparison:** same user intent, different scope/default/screen model;
  explain the difference rather than award a feature point.
- **UX hypothesis:** plausible friction grounded in mechanics, not a reproduced
  usability result. Recommendations remain design proposals.
- **Unverified:** no sufficient evidence. Do not convert this to absence or success.

Pins: Helm `f068d7966394e88d2520471b432f3e385ccdfcd9`, Pi
`71dca871bc80b6bc97be37f0ca3189399d651fff`, OpenCode
`95daf90670b7c039c436c85537da5fbfe2205b41`, Codex
`36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564`. These are development snapshots,
not matched stable releases. Helm's unrelated staged render comments were excluded
from line citations by reading the committed file. Existing dirty work was preserved.

## Corrections to the earlier conclusions

| ID | Earlier claim or implication | Rechecked finding | Consequence for the audit |
|---|---|---|---|
| R01 | More controls/citations establish a comprehensive UX assessment | The inventories describe mechanics but do not measure discovery, attention, task success or screen comprehension. | Keep inventories as evidence. Assess journeys and synthesize recurring friction; do not count rows as acceptance. |
| R02 | Helm needs contextual Esc and pending-response guidance as though none exists | Request renderers already say **Esc Deny**, **Esc Skip**, **Esc Back**, show expiry and pending response text. The footer already says **Pending · Checking automatically · Text preserved** and **Ctrl+C Leave**. Generic help is less precise. [H1–H3] | Narrow U01/U10: reconcile global/contextual guidance and test focus transitions. Do not prescribe rebuilding existing labels. |
| R03 | Sending while active is just a missing queue feature | Helm dispatches Steer for an observed active run, while the footer still says **Enter Send** or **Next-turn settings · Enter Send**. [H1,H4] | The immediate confirmed mismatch is visible intent versus dispatched intent. Expose that before designing a new queue protocol. |
| R04 | An approval popup is simply another screen/key mapping | Helm auto-focuses eligible requests; review replaces the composer with saved-draft text and opens a right pane sized to half the terminal width, capped at 64 columns, preserving the conversation pane. Navigation yields space as width narrows. [H2,H5] | The meaningful hypothesis is surprise during typing and loss of conversation context. Test arrival mid-composition, not just opening a request deliberately. No accidental approval is claimed. |
| R05 | Helm compact means only “keep last N and omit everything older” | The working-context reducer retains user messages, extracts older assistant/tool content, preserves references and recorded outcomes, and groups complete tool exchanges. Canonical history is unchanged. It is not model-generated summarization. [H6] | Correct the simplistic cutoff description. Compare usefulness, provenance and cost of retained context, not just matching command names. |
| R06 | A single searchable catalogue is already the demonstrated solution | A unified catalogue is a design hypothesis. Contextual controls may be faster for frequent actions; private setup and destructive review need distinct surfaces. | Test finding and completing tasks. Prefer shared vocabulary and discoverability without forcing every workflow through one modal. |
| R07 | Model reasoning display is uniform across competitors | Effort, provider-supplied reasoning summaries, raw-output modes and hidden/displayed thinking blocks are different. Capability and feature gates apply. | Compare the operator's ability to understand progress, not quantity of internal text exposed. Never invent absent provider reasoning. |
| R08 | Recovery machinery itself is a UX advantage | Durable command identity is valuable correctness infrastructure; it does not prove the user understands the banner or can recover. Helm already performs automatic receipt checks. [H7] | Evaluate the normal recovery screen and next action before suggesting more UUID/receipt controls. Retain exactly-once/uncertainty protections. |

## Compare whole experiences, not isolated commands

The following is a design synthesis grounded in the pinned mechanics. Competitor
specific corrections and evidence are recorded below. “Risk” means an interaction
hypothesis unless explicitly stated otherwise.

| Journey / user intent | Helm current screen and transition | Competitor approaches and meaningful trade-off | Assessment and proposed Helm experience | Acceptance observation needed |
|---|---|---|---|---|
| **Open and orient:** “Where am I, and can I start?” | Persistent voyage navigation at sufficient width; heading/state, conversation, composer/inference controls and footer. Below 40×18 it requests enlargement. Draft setup differs from an existing voyage. [H1,H5] | Pi concentrates on a coding conversation with editor/footer and temporary selectors, with regular and optional fullscreen layouts. OpenCode separates home/composer from session route and uses modal discovery. Codex couples conversation/composer with contextual bottom panes and optional onboarding. These are different information hierarchies, not just themes. | Keep host/voyage identity immediately legible, but let composing be the primary task. Do not make “understand the infrastructure” a prerequisite for the first useful prompt. Avoid assuming that hiding all navigation is better. | Fresh and configured launch at 80×24/120×40: can the user identify host/project, focus, and next action without help? Record screens and actions, not impressions alone. |
| **Compose:** “Express my task and attach the right context.” | Editor, optional image previews/metadata and next-turn controls share bounded space. Completion, history, navigation and private panels have competing input scopes. | Pi's temporary selectors preserve a compact chat surface; OpenCode file/agent completion and external editing support context gathering; Codex separates composer actions from generic editor bindings. Shared chords do not imply shared behavior. | Preserve one recognizable composition area and exact authored text. Put file/attachment selection where its inclusion is visible. Make focus/context changes apparent; do not advertise a key just because a lower-level editor defines it. | Paste multiline/Unicode, attach an image, open completion, recall text, cancel a picker and resume typing. Verify text, attachments, focus and number of context switches. |
| **Send and wait:** “Did it get my instruction, and what is happening?” | Footer says Send; owner may submit or steer. Pending identity and provider/run states are available, with tool activity in the transcript. | Pi makes steering/follow-up queues visible near the editor. Codex has distinct submit/queue routes with state-dependent handling. OpenCode busy-session behavior must be traced through the actual prompt handler, not inferred from a key declaration. | A stable status area should explain **received → queued/steering → executing/waiting → finished** without requiring diagnostic navigation. Live tool detail is subordinate to that story. | Slow text/tool arguments, long tool execution and permission wait: ask what state the user thinks the task is in, then compare with recorded owner events. |
| **Intervene:** “Correct this, or stop it.” | Enter during a run steers; explicit Cancel is in slash/Actions; Ctrl+C detaches Helm and work continues. | Pi Esc abort and follow-up/dequeue affordances favor an active editor loop. OpenCode has guarded/double-Esc interrupt. Codex routes Esc, queue editing and submission according to active state. All require context qualification. | Keep a discoverable exact-run Stop near the busy state. Label the composer action by intent. Preserve detach as a separate operation. Do not copy Esc semantics wholesale across decisions and private terminals. | Type a correction, queue where supported, stop, detach and reconnect. Verify target, draft retention, delivered instruction and actual cleanup; request admission is not completion. |
| **Decide:** “What needs my attention, and what does approval allow?” | Eligible decision takes focus. It uses a right-hand pane while retaining the conversation; the composer becomes saved-draft text and is no longer the active editor. Narrow layouts squeeze both panes. Contextual footer describes deny/skip/back. | OpenCode permissions/questions displace the prompt region; Codex bottom-pane overlays route input before the composer. Pi project trust/extension dialogs are not an execution-approval equivalent. The comparison is interruption design plus authority scope, not popup aesthetics. | Show who/what is asking, exact consequence and affected host before activation. Make arrival/selection/confirmation distinct. Preserve reading context where feasible; never trade away private-input isolation for a seamless-looking modal. | Request arrives while typing, completion is open or transcript is scrolled. Test Enter/Esc and pasted input against the newly focused state; check expired and replaced requests. |
| **Inspect:** “What changed, and can I trust the result?” | Transcript and expandable tool detail are primary. No first-class fixed diff/copy-response/file-mention action was identified. A tool can inspect files but requires another conversational step. | OpenCode integrates timeline/message actions and a dedicated diff viewer; Codex separates deterministic `/diff` from model `/review`; Pi emphasizes transcript/tools and tree navigation, not equivalent built-in workspace rollback. | Distinguish **read answer**, **inspect workspace changes**, **review quality**, and **undo**. Add direct inspection only with declared scope; preserve the transcript as evidence, not the only navigation mechanism. | Complete a multi-file task containing staged, unstaged, untracked and binary changes. Can the user locate affected files, identify scope, copy canonical text and return to their reading position? |
| **Switch and return:** “Pick up where I left off.” | Persistent catalogue/F2 selects exact route-qualified voyage, retains per-view composer. Branch creates another voyage; no equivalent message tree was found. | Pi tree preserves alternate conversational paths; OpenCode timeline/fork anchors actions to a message; Codex resume/backtrack uses contextual picker and draft-preservation behavior. Multiple open workspaces and historical branches solve different problems. | Preserve working memory: draft, read anchor, running state, host and selected task. First make switching predictable; add historical branching only with a preview of the selected context and new identity. | Switch same-title voyages across hosts while one runs and one has attachments; return to both. Separately branch from a historical message and verify no implied file rollback. |
| **Recover:** “What failed, what survived, what do I do next?” | Reconnecting/recovery/pending banners and automatic original-receipt checks; durable owner controls remain authoritative. | Pi retry/abort is inference recovery; OpenCode reports ordinary send failure after clearing the prompt; early session-create failure preserves it; Codex steering-race recovery and interruption handling have specific eligibility guards. None establishes universal exactly-once external effects. | Present retained facts and one safe next step; hide diagnostic IDs until needed, but keep them durable. A restore/retry banner must not imply process survival or undone effects. | Lose a response after admission, disconnect during a tool and restart an owner separately. Verify authored text, effect uncertainty, pending identity and comprehensible user-visible next action. |


## Pi: editor continuity, not a miniature voyage manager

| Finding | Actual interaction and screen consequence | Fair comparative conclusion |
|---|---|---|
| **P1: Distinct intents, lossy queue editing** | Busy Enter steers; busy Alt+Enter queues follow-up. Pending rows are labeled Steering/Follow-up near the editor. Alt+Up drains both queues into **one draft**, steering text first, followed by existing draft text. Original item boundaries/delivery modes are no longer editable queue objects. Idle Alt+Enter submits normally. [P-A] | Borrow visible delivery intent, not an imagined per-item durable queue. Recovery to text is useful but cannot stand in for admission/receipt tracking. |
| **P2: Esc depends on focused surface and operation** | Completion, selectors and fullscreen search consume Esc before normal interruption. Ordinary streaming Esc restores queued text and requests abort; retry and compaction install their own cancellation handlers. [P-B] | Pi does not establish a universally available Stop key. Compare whether the active screen reveals the cancellation target. |
| **P3: Two screen modes, one editorial center** | The default regular layout stacks header, chat, pending/bashed output, extension widgets, editor and footer. Optional fullscreen docks the editor/pending region under a primary scroll view and enables a different search/scroll contract. Selectors replace/focus the editor area. [P-C] | Simplicity comes from continuity around composing, not fewer capabilities. Do not credit optional fullscreen search as the default experience or equate terminal scrollback with Helm's bounded history loading. |
| **P4: Tree navigation changes the working context** | Browsing a tree can coexist with streaming. Committing another historical point may stop the response, optionally summarize, switch context, and replace the draft with the selected user message. This is not file restoration. [P-D] | Tree is both inspection and re-authoring. An attractive history view has consequential commit semantics; test draft/queue preservation through navigation, not only the tree screenshot. |
| **P5: Resume is not concurrent voyage switching** | Resume validates the destination/cwd, aborts and disposes the outgoing runtime, then binds another. The former local agent is not left running in the background. [P-E] | Compare find/resume ergonomics, but assess Helm's concurrent switching separately. A shared word “session” conceals different lifecycle costs. |

Pi's project-trust gate concerns project resource loading, not per-tool execution
approval. An untrusted project can still be worked on with ordinary tools under
the user's local authority; skip project resources is not read-only containment.
Retry/compaction also have distinct cancellation targets: cancelling compaction
does not necessarily cancel queued prompts. These are important qualifications,
not proof that Pi is unsafe or that Helm is easier to recover.

- **P-A:** [submission/follow-up](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L4125-L4154), [pending labels and bulk restoration](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L4363-L4405).
- **P-B:** [ordinary Escape handler](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L2852-L2877), [focused editor interception](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/components/custom-editor.ts#L88-L115).
- **P-C:** [regular/fullscreen component composition](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L874-L903), [selector focus](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L4529-L4551).
- **P-D:** [tree navigation commit](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L5250-L5316).
- **P-E:** [runtime retirement and resume](https://github.com/badlogic/pi-mono/blob/71dca871bc80b6bc97be37f0ca3189399d651fff/packages/coding-agent/src/core/agent-session-runtime.ts#L167-L223).


## OpenCode: integrated local coding, with important broken comparisons

| Finding | Actual interaction and screen consequence | Fair comparative conclusion |
|---|---|---|
| **O1: Declared queue key is not a default TUI journey** | Default `packages/tui` declares/maps `session_queued_prompts`, but has no corresponding registered action. The queue editor is in the optional mini/run footer. Default Leader+Q is also an Exit binding. [O-A] | **Withdraw our issue-table claim that Leader+Q opens queue management in the default TUI.** This was a surface/registration error, not mere wording. Do not press it expecting a queue editor. |
| **O2: Failed ordinary send does not automatically restore the draft** | Initiating ordinary submission appends history and clears the prompt. Its asynchronous rejection handler displays a “Failed to send prompt” toast; it does not restore draft/agent/model or remove an optimistic message. Failure to create a new session occurs earlier and does preserve the draft. [O-B] | **Withdraw the appendix's optimistic-message restoration claim.** History recall is a separate manual recovery path. Distinguish failure before session creation from failure after prompt clearing. |
| **O3: Last-turn diff is not a complete normal journey** | The dedicated viewer has working-tree/conditional branch sources. Ordinary `/diff` provides no message ID; switching to Last turn forwards that absent ID; the backend returns an empty list without one. [O-C] | Credit the actual working-tree diff route, not an end-to-end last-turn feature. The source path is incomplete; its live empty/error presentation is not reproduced here. |
| **O4: Decisions displace composition** | Home presents centered logo/composer; session route adds transcript/context. Dialogs blur/restore prior focus. Pending permissions/questions replace the composer; permissions take priority. Rejection feedback is offered for child-session requests, while root rejection replies immediately. [O-D] | Compare the continuity and interruption cost of that replacement with Helm's side review and Codex's bottom pane. Do not describe feedback as universal or mistake a popup for an additional chat message. |
| **O5: Historical navigation is scoped** | Session search filters titles; timeline searches labels derived from loaded user messages. Message actions can copy/fork/revert; the Modified Files sidebar is not itself a file-click-to-diff navigator. [O-E] | This is useful navigation, not demonstrated full-text conversation search or direct drill-down from every file label. Assess the journey that actually connects those surfaces. |

The coherent positive lesson is **a shared dialog/focus vocabulary and direct
local coding inspection**, not complete recovery or queue superiority. Source paths
also reveal costs: composer replacement during decisions, cleared input after
failed send, and an incomplete ordinary last-turn diff route. Those limitations
belong in the same comparison as the attractive palette and viewer.

- **O-A:** [default/mini launch split](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/opencode/src/cli/cmd/tui.ts#L123-L174), [key defaults including exit](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/config/keybind.ts#L36-L104). The bounded absence was checked by searching registration/dispatch references throughout `packages/tui/src`; the declaration and mapping are not implementations.
- **O-B:** [ordinary prompt catch and clearing](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/component/prompt/index.tsx#L1092-L1146), [pre-send guards/session creation](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/component/prompt/index.tsx#L930-L1024).
- **O-C:** [viewer last-turn loader](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/feature-plugins/system/diff-viewer.tsx#L114-L122), [source switch](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/feature-plugins/system/diff-viewer.tsx#L701-L725), [ordinary opener](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/feature-plugins/system/diff-viewer.tsx#L1061-L1066), [backend missing-ID result](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/opencode/src/session/summary.ts#L129-L141).
- **O-D:** [home composition](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/routes/home.tsx#L70-L93), [dialog focus lifecycle](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/ui/dialog.tsx#L85-L155). [request rejection routing](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/routes/session/permission.tsx#L349-L394) establishes the root/child distinction; additional context is retained in the corrected [OpenCode appendix](ux-opencode-actions.md).
- **O-E:** [sidebar file behavior](https://github.com/anomalyco/opencode/blob/95daf90670b7c039c436c85537da5fbfe2205b41/packages/tui/src/feature-plugins/sidebar/files.tsx#L14-L45); [timeline and session routes](ux-opencode-actions.md).

## Codex: separate editing, inspection, decisions and recovery

| Finding | Actual interaction and screen consequence | Fair comparative conclusion |
|---|---|---|
| **C1: Visible composer is not always ready to submit** | Startup offers an editable draft with initializing/resuming/forking context; Enter and Tab do not admit work there. Later readiness and trust checks are distinct. [C-A] | The user can prepare while waiting, but needs to understand why Enter does nothing. Compare readiness feedback rather than number of onboarding steps. |
| **C2: Tab is state-dependent, not unconditional queue** | Outside completion, the queue key invokes submission with `is_task_running` or forced-queue state. Ordinary idle Tab submits; active/forced-queue Tab queues. Pending preview labels queued follow-ups and rejected steering separately. [C-B] | **Correct “Tab queues” shorthand.** The interaction coherently separates busy intent but is not a universal save-for-later action. |
| **C3: Repeat-to-quit branch is disabled** | `DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED` is false. Ctrl+C first allows focused cancellation, then clears a nonempty draft; empty input interrupts cancellable work or quits immediately when idle. [C-C] | **Withdraw repeat-to-exit as a default behavior.** Focus/state distinctions remain; copying this into Helm would still require preserving detach semantics. |
| **C4: Raw tool output is not reasoning** | Main transcript uses compact activity; reasoning summary detail is transcript-only, with derived status text in the main view. Raw-output mode expands tool output; raw reasoning requires separate configuration. [C-D] | **Correct the raw/reasoning conflation.** Compare useful progress and inspection access, not supposed universal transparency of model internals. |
| **C5: Diff scope warning is real** | `/diff` covers tracked unstaged plus untracked files, not staged-only changes; result opens a static pager. “No changes detected” can be overbroad relative to the workspace's total state. [C-E] | Keep the scope warning. A dedicated command is valuable only when its result names what it inspected; Codex is not a gold standard for every diff edge case. |
| **C6: Read-only recovery is a deliberate screen state** | Another active writer can yield a readable snapshot with a disabled composer. Interrupted/recovered queues are retained without automatically draining into execution. [C-F] | Credit truthful read access and explicit recovered intent, not takeover or process survival. The extra step is a safety trade-off, not automatically poor usability. |

Codex's useful pattern is **separation of editing, read-only inspection, authority
decisions and recovered intent**, with explicit transitions between them. Costs
include learning when the composer is not submitting, changing key meaning by
focus, and entering a separate transcript/pager for detail. Neither the separation
nor its costs establish a winner without actual journey observations.

- **C-A:** [startup input handling](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/startup_draft.rs#L391-L424), [startup screen](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/startup_draft.rs#L470-L523).
- **C-B:** [submit/queue branch](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L3562-L3569), [pending preview](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/pending_input_preview.rs#L119-L165).
- **C-C:** [disabled double-quit constant](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/mod.rs#L205), [Ctrl+C routing](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L530-L590).
- **C-D:** [reasoning summary display separation](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/history_cell/messages.rs#L359-L379), [raw tool output state](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget.rs#L1654-L1685).
- **C-E:** [diff argv](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/get_git_diff.rs#L18-L120), [pager result](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/event_dispatch.rs#L1007-L1025).
- **C-F:** [queue drain guards](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/input_flow.rs#L200-L219); [writer conflict fallback](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/session_lifecycle.rs#L1253-L1273) and [read-only screen state](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/session_lifecycle.rs#L341-L388).

## Coherent direction for Helm

The useful lesson is not “copy Pi's keys, OpenCode's palette and Codex's dialogs.”
That would combine three competing interaction models.

A coherent candidate for Helm is **a stable conversation workspace with persistent
identity, an intent-aware composer, and bounded task-focused review surfaces**:

1. **Identity stays stable:** host, voyage and current/next settings remain legible
   without requiring every screen to repeat infrastructure detail.
2. **The composer retains its role:** its visible action tells the user whether
   they are submitting, steering, answering or waiting. A decision has explicit
   focus and does not silently masquerade as chat.
3. **Progress tells one story:** ordinary status explains what is happening;
   expandable detail answers why. Tool text and reasoning are evidence, not the
   sole progress indicator.
4. **Inspection is direct:** answer copy, scoped changes and message navigation
   are not contingent on asking the model another question.
5. **Recovery preserves both truth and place:** exact command identity and effect
   uncertainty survive alongside the draft, selection and reading position.

These are proposed design constraints, not an approved mockup or implementation.
Validate the journey before standardizing the components. Measure task completion,
wrong-target/accidental-action rate, retained context, discovery detours and recovery
success rather than counting feature parity or assuming fewer keys always wins.

## Helm evidence

All source links below are pinned to the reviewed committed version.

- **H1:** [main layout and footer](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/render.rs#L170-L420): Send/pending/Leave hints, bounded layout and composer displacement.
- **H2:** [decision focus selection](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/interactions/mod.rs#L88-L125) and [input routing](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/interactions/input.rs#L1-L218).
- **H3:** [request footer](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/interactions/render.rs#L220-L235), [general help](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/presentation.rs#L123).
- **H4:** [active-run steering versus submission](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/actions.rs#L289-L311).
- **H5:** [layout modes](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/render.rs#L218-L276), [composer replacement during review](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/render.rs#L355-L374).
- **H6:** [manual extractive compaction](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/voyage/src/context/working.rs#L138-L259), [canonical history unchanged](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/voyage/src/session/outcomes.rs#L261-L280).
- **H7:** [automatic command recovery](https://github.com/o-psi/helm.vessel.voyage/blob/f068d7966394e88d2520471b432f3e385ccdfcd9/helm/src/process_client/ui/reconcile.rs#L1-L80).

## Verification and remaining work

Rechecked the three competitor source traces and incorporated the independent
findings above; retained working reports under ignored `target/ux-audit/`.
Documentation validation checks pinned source paths/line bounds, local links,
Markdown tables and the retained command/action inventory. A valid citation is
not semantic proof: this review also followed caller/handler chains, discovering
incorrect earlier claims despite their valid source links.

No Rust source/manifests/tests were changed by this delivery. No build, Rust
coverage, upstream execution, paid provider call, screenshot journey, timing or
native-platform validation was performed. #272 remains open for implementation
and executable acceptance. The next useful evidence is X01–X16 in the original
audit, exercised by journey with the corrections here—not another larger registry.
