[Comparative audit and evidence limits](ux-comparison.md)

# Codex CLI terminal UX audit for voyage #271

## Pin, scope and evidence discipline

- **Upstream:** public [`openai/codex`](https://github.com/openai/codex), current HEAD resolved once through GitHub API to **`36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564`**. [Commit](https://github.com/openai/codex/commit/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564), upstream committer timestamp **2026-09-13T13:06:40Z**. This is a development HEAD audit, not a claim about a published release or every user's installed binary. Audit date: **2026-09-13 UTC**.
- **Subject:** Rust Codex terminal TUI, its command-line entry point, and backend source needed to explain those terminal actions. Web, desktop app UX and IDE extension UX are excluded. Commands that launch/integrate with those products are recorded only as terminal entry points, not evidence of their destination UX.
- **Evidence:** static source inspection and public GitHub metadata retrieval. No Codex binary, TUI journey, test suite, provider request, auth flow, tool command, sandbox or OS security check was executed. “Shows,” “sends,” “refuses” and “supports” below describe source paths, **not observed live interactions**. Upstream tests are corroborating implementation evidence, not passing tests from this audit.
- **Scope control:** issue #271 was read, not modified. Existing checkout modifications were observed and not touched. Downloads/extracted upstream source live only under `target/ux-audit/codex/`; the requested final report is `target/ux-audit/codex-report.md`. No tracked files, issues, commits or publication were changed for this read-only task.
- **Notation:** default = compiled source/default feature configuration, still subject to account/model/managed policy and terminal capabilities; gated = feature/platform/backend/auth prerequisite; experimental = source explicitly labels an experimental/under-development surface. A stable feature can be default-off; presence in an enum is not proof of menu visibility. **Absent** is only asserted against a named authoritative inventory; **unverified** means not established by this review. Unknown live behavior is never treated as absent.

## Executive findings

1. This pin has a substantially larger terminal command surface than older Codex cheat sheets: enumerate the source enum and parser, not remembered commands. Context-sensitive menu visibility, active-turn restrictions, inline arguments, aliases and feature defaults are separate dimensions.
2. Sessions have searchable preview/density/filter controls, preserved drafts, source-preserving prompt branching, destructive confirmations and read-only fallback for another active writer. Branching conversation is not filesystem rollback.
3. Approvals, sandbox rules, questions and steering are distinct user decisions. A click to view/select/navigate is not permission to execute. Terminal security/recovery should be compared with Helm's execution-host and private-input boundaries, not copied from a superficially similar popup.
4. `/diff` is locally computed rather than a model review, and this pin's actual Git argv covers tracked **unstaged** plus untracked differences, not staged-only changes. Source-level scope details matter more than generic “review changes” labels.
5. Extensive feature, platform, auth, model and remote-backend conditions prevent a credible all-default runtime claim. The report is a reproducible action inventory and scenario backlog, not hands-on certification or a feature-count ranking.

## Navigation

- [Slash commands and editor](#slash-commands-and-editor-source-audit)
- [Session, resume, fork, review, diff and undo](#session-resume-fork-review-diff-and-undo-actions)
- [Approvals, streaming, queue and recovery](#runtime-interaction-source-audit)
- [CLI flags, auth, config, MCP and skills](#cli-config-source-audit)
- [Comparative acceptance backlog](#comparative-acceptance-backlog-unexecuted)
- [Source manifest and verification](#source-manifest-and-verification)

---

## Slash commands and editor source audit

<a id="slash-commands-and-editor-source-audit"></a>

Codex terminal action inventory — #271 fragment

## Scope, provenance and reading rules

Read-only source audit of `openai/codex` at **`36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564`**, supplied tree `/home/psi/voyage/target/ux-audit/codex/openai-codex-36f0dbe`. Adjacent `commit.json` reports this exact SHA. This fragment owns only `target/ux-audit/codex/slash-editor.md`. No implementation, issue mutation, provider calls, runtime tests, builds, downloads or hands-on platform certification were performed. Issue #271 was read for scope only. The archive tree has no Git metadata; commit metadata corroborates the supplied pin, not an independent remote/tree-integrity certification.

**D** = default discoverable at the enum/filter layer, not guaranteed to execute successfully. **G** = explicit compile/platform/config/auth/context gate. **Unknown** = actual deployment configuration, model catalog, backend capabilities, clipboard/editor/terminal availability and end-to-end success were not exercised. “Open”, “emit”, “submit”, and “request” below describe the observed handler, not completed downstream effects. Default feature values can be overridden. Busy/side columns are enum permissions, further constrained by dispatch. This inventory covers **59 / 59 enum variants** in presentation order; aliases are additional spellings, not separate variants.

## Common discovery, parsing and availability contract

- Enum declaration order is popup presentation order, intentionally not alphabetical; serialized names/aliases and descriptions live in [slash_command.rs:7–156](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/slash_command.rs#L7-L156).
- Static visibility hides Copy on Android, App outside macOS/Windows, and Rollout/TestApproval outside debug builds. **DebugConfig, MemoryDrop and MemoryUpdate are not debug-build gated** despite their names. [slash_command.rs:266–283](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/slash_command.rs#L266-L283)
- Composer and popup share filtering: sandbox elevation, Plan collaboration mode, Apps, Plugins, Usage auth, Goal, Voice, Worktree and side availability. Typed lookup deliberately ignores side and Usage-auth hiding so the handler can explain refusal; other filtered commands are not found. Goal also accepts `g` + one or more `o` + `al` (e.g. `/gooal`). [bottom_pane/slash_commands.rs:57–156](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/slash_commands.rs#L57-L156)
- Actual flag wiring: restricted-token Windows elevation; Plugins/Goals features; backend auth; realtime availability for thread; Worktrees plus local operations. [chatwidget/slash_dispatch.rs:1120–1159](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L1120-L1159) Apps additionally requires a ChatGPT account: [chatwidget/connectors.rs:150–163](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/connectors.rs#L150-L163). Collaboration modes are hardcoded enabled here: [chatwidget/settings.rs:384–386](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/settings.rs#L384-L386). Worktrees/Plugins/Goals/realtime source defaults are true; memories is experimental/default false; these do not prove effective runtime configuration. [codex-rs/features/src/lib.rs:1108–1125](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1108-L1125) [codex-rs/features/src/lib.rs:1256–1262](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1256-L1262) [codex-rs/features/src/lib.rs:1382–1387](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1382-L1387) [codex-rs/features/src/lib.rs:1616–1622](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1616-L1622) [codex-rs/features/src/lib.rs:1652–1658](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1652-L1658) [codex-rs/features/src/lib.rs:1700–1706](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1700-L1706)
- Inline-argument support is an explicit whitelist (including `/pwd`, whose argument handler rejects args). `—` means no supported inline-argument route, **not** a free-form prompt parameter. Leading-space input and slash tokens containing another `/` bypass command treatment in relevant parsing paths; shell mode disables slash parsing. Unknown-command validation is distinct from ordinary message submission. [slash_command.rs:159–201](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/slash_command.rs#L159-L201) [bottom_pane/chat_composer/slash_input.rs:49–155](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs#L49-L155)
- Busy rejection prints `'/…' is disabled while a task is in progress`, drains pending submission state, redraws. Review always blocks Side/Btw. Resume/Cd additionally block pending-start/agent-running state; Export blocks during queue-autosend suppression. Side allows only Copy, Agents, Export, Raw, Diff, Mention, Status, Pwd, Usage, Ide; other typed commands explain refusal and how to switch back. [chatwidget/slash_dispatch.rs:108–168](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L108-L168)
- Slash history is staged/recorded through dispatch wrappers; inline draft preparation preserves structured arguments for Plan/Goal/Side/Btw and carefully handles other argument text. Queued commands are reparsed with current availability, not assumed authorized by their old popup state. [chatwidget/slash_dispatch.rs:41–83](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L41-L83) [chatwidget/slash_dispatch.rs:593–714](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L593-L714) [chatwidget/slash_dispatch.rs:1057–1120](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L1057-L1120)

## Every slash enum command

Busy/side: **Y** = allowed by enum, **N** = denied by enum (not the only runtime check). All rows inherit the common gates above. Each evidence link points to its no-args dispatch arm; inline handlers are additionally anchored immediately after the table.

| Enum / slash spelling | Visibility / gate | Busy / side | Inline args and result | Bare handler / outcome | Evidence |
|---|---|---|---|---|---|
| `Model` — `/model` | D | Y / N | — | Open model/reasoning picker. | [dispatch:314–328](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L314-L328) |
| `Ide` — `/ide` | D | Y / Y | on / off / status; invalid → usage | Bare command shows IDE integration/context status; on/off enables/disables context sharing. | [dispatch:507–521](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L507-L521) |
| `Permissions` — `/permissions` | D | Y / N | — | Open permissions picker; policy constraints apply, not an automatic grant. | [dispatch:348–362](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L348-L362) |
| `Keymap` — `/keymap` | D | N / N | debug; invalid → usage | Open keymap picker; debug resolves local tui.keymap and displays bindings or configuration error. | [dispatch:359–373](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L359-L373) |
| `Vim` — `/vim` | D | N / N | — | Toggle composer Vim mode and print enabled/disabled notice. | [dispatch:356–370](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L356-L370) |
| `ElevateSandbox` — `/setup-default-sandbox` | G: Windows restricted-token sandbox only | N / N | — | Check current sandbox is RestrictedToken, find auto approval preset, check policy can_set, then BeginWindowsSandboxElevatedSetup; missing preset/policy failure → error. Non-Windows handler is inert. | [dispatch:362–376](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L362-L376) |
| `Experimental` — `/experimental` | D | N / N | — | Open experimental-feature picker. | [dispatch:414–428](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L414-L428) |
| `AutoReview` — `/approve` | D | Y / N | — | Open automatic-review denial popup (approve command); not a generic unconditional tool approval. | [dispatch:417–431](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L417-L431) |
| `Memories` — `/memories` | D; contents feature-dependent | N / N | — | If MemoryTool disabled, open feature-enable prompt; otherwise open memories settings for use_memories and generate_memories. | [dispatch:420–434](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L420-L434) |
| `Skills` — `/skills` | D | Y / N | — | Open skills menu. Tab dispatches this command immediately (unlike normal completion). | [dispatch:468–482](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L468-L482) |
| `Import` — `/import` | D | N / N | — | Emit OpenExternalAgentConfigMigration. | [dispatch:471–485](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L471-L485) |
| `Hooks` — `/hooks` | D | Y / N | — | Print configured hooks output (not a hooks editing popup). | [dispatch:475–489](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L475-L489) |
| `Review` — `/review` | D | N / N | custom instructions → AppCommand::review(Custom) | Bare opens review picker; inline sends custom review request. | [dispatch:303–317](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L303-L317) |
| `Rename` — `/rename` | D; rename eligibility | Y / N | nonempty name; normalize; empty normalized name → error | Bare asks for session name; inline checks rename allowed and requests thread-name update. | [dispatch:309–323](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L309-L323) |
| `New` — `/new` | D | N / N | name passed to checkout picker | Choose current checkout / new worktree when managed worktrees available; otherwise NewSession directly. | [dispatch:184–198](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L184-L198) |
| `Archive` — `/archive` | D | N / N | — | Confirmation defaults to No; Yes emits ArchiveCurrentThread. | [dispatch:187–201](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L187-L201) |
| `Delete` — `/delete` | D | N / N | — | Confirmation warns irreversible and subagents also deleted; Yes emits DeleteCurrentThread. Local label says exit; remote label says return to command center. | [dispatch:215–229](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L215-L229) |
| `Resume` — `/resume` | D | Y / N | ID or name → ResumeSessionByIdOrName | Bare OpenResumePicker. | [dispatch:251–265](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L251-L265) |
| `Fork` — `/fork` | D | N / N | name passed to checkout picker | Current checkout / new worktree picker if available, otherwise ForkCurrentSession. | [dispatch:254–268](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L254-L268) |
| `Worktree` — `/worktree` | G: Worktrees feature AND local_worktree_operations | N / N | — | Show managed-worktree picker; unavailable guard prints local-session-with-worktrees requirement. | [dispatch:257–271](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L257-L271) |
| `App` — `/app` | G: macOS / Windows; hidden elsewhere | Y / N | — | Emit OpenAppLink (desktop app handoff; completion not established). | [dispatch:260–274](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L260-L274) |
| `Init` — `/init` | D | N / N | — | If cwd/AGENTS.md exists, print skip info; else submit bundled init prompt as a user message. File creation is model work, not guaranteed by dispatch. | [dispatch:270–284](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L270-L284) |
| `Compact` — `/compact` | D | N / N | — | Clear token-usage display, submit AppCommand::compact. | [dispatch:274–288](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L274-L288) |
| `Recap` — `/recap` | D | N / N | — | Request recap; not proof recap has completed. | [dispatch:293–307](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L293-L307) |
| `Plan` — `/plan` | D in this TUI; collaboration flag true, catalog mask required | N / N | prompt; switch Plan then submit, or queue before configured; preserve draft if mask unavailable | Bare changes collaboration mask; unavailable mode prints info. Inline prohibits shell-escape interpretation. | [dispatch:318–332](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L318-L332) |
| `Voice` — `/voice` | G: realtime_conversation_available_for_thread | Y / N | settings → settings view; mute → toggle mic; stop → stop; invalid → usage | Bare toggles realtime conversation. Platform/audio/backend support remains environment-dependent. | [dispatch:336–350](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L336-L350) |
| `Goal` — `/goal` | G: Goals feature (source default true) | Y / N | objective / clear / edit / pause / resume (subcommands exact lowercase) | Bare RequestThreadGoalStatus if thread exists, otherwise usage. Inline clear/pause/resume send goal events; edit opens editor after session starts; objective builds attachment-aware GoalDraft and ConfirmIfExists. Before session start, live objective is queued; queued request without thread prints usage. Feature disabled → error. | [dispatch:321–335](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L321-L335) |
| `Agents` — `/agents` | D | Y / Y | — | OpenAgentsList. | [dispatch:342–356](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L342-L356) |
| `Side` — `/side` | D outside side; forbidden during review | Y / N | prompt → seeded side conversation | Bare requests side conversation without seed; parent thread required, else error. Nested side dispatch rejected. | [dispatch:339–353](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L339-L353) |
| `Btw` — `/btw` | D outside side; forbidden during review | Y / N | same as /side | Same side-conversation handler as /side. | [dispatch:339–353](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L339-L353) |
| `Copy` — `/copy` | G: hidden on Android; D elsewhere | Y / Y | — | Show copy picker (contextual copy targets). | [dispatch:429–443](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L429-L443) |
| `Export` — `/export` | D | N / Y | destination path → ExportTranscript(File) | Bare transcript export popup; inline suppresses queue autosend before event. Dispatch is not a successful file write. | [dispatch:432–446](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L432-L446) |
| `Raw` — `/raw` | D | Y / Y | on / off (case-insensitive); invalid → usage | Bare toggles raw output and emits change; inline sets explicit state. | [dispatch:435–449](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L435-L449) |
| `Diff` — `/diff` | D | Y / Y | — | Show diff-in-progress then asynchronously get workspace Git diff; no repo → no-repository text; missing runner → unavailable text; errors → error text; final RawHistoryLines event. | [dispatch:439–453](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L439-L453) |
| `Mention` — `/mention` | D | Y / Y | — | Insert @ to start mention/file selection. | [dispatch:465–479](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L465-L479) |
| `Status` — `/status` | D | Y / Y | — | Render status. If prefetch appropriate, show refreshing state and emit RefreshRateLimits with request ID; otherwise current output. | [dispatch:478–492](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L478-L492) |
| `Cd` — `/cd` | D; idle primary persistent configured session required | N / N | path → ChangeWorkingDirectory request | Bare usage. Blocked by active turn, queued messages/steers, exec processes, side/ephemeral/direct-input-blocked state or MCP inventory loading. | [dispatch:493–507](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L493-L507) |
| `Pwd` — `/pwd (alias cwd)` | D | Y / Y | arguments deliberately yield Usage: /pwd | Print working directory. /cwd alias. | [dispatch:496–510](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L496-L510) |
| `Usage` — `/usage` | G: popup requires Codex backend auth; typed recognized even without auth | Y / Y | daily / weekly / cumulative; invalid → usage | Bare defaults Daily; no backend auth → login-required error. | [dispatch:502–516](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L502-L516) |
| `DebugConfig` — `/debug-config` | D, NOT debug-build-only | Y / N | — | Print debug configuration output. | [dispatch:510–524](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L510-L524) |
| `Title` — `/title` | D | Y / N | — | Open terminal-title setup. | [dispatch:513–527](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L513-L527) |
| `Statusline` — `/statusline` | D | Y / N | — | Open status-line setup. | [dispatch:516–530](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L516-L530) |
| `Theme` — `/theme` | D | N / N | — | Open theme picker. | [dispatch:519–533](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L519-L533) |
| `Pets` — `/pets (alias pet)` | D | N / N | on / off / hide / hidden / minimize / minimized / mute / unmute / show / status / settings; invalid → usage | Bare pet picker. on/show/unmute→visible; off/mute→muted; hide/hidden→hidden; minimize/minimized→minimized; status→notice; settings→setup. Case-insensitive. | [dispatch:522–536](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L522-L536) |
| `Mcp` — `/mcp` | D | Y / N | verbose (case-insensitive) → full status; invalid → usage | Bare compact MCP status output. | [dispatch:537–551](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L537-L551) |
| `Apps` — `/apps` | G: Apps feature AND ChatGPT account | Y / N | — | Open connectors/app picker. | [dispatch:540–554](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L540-L554) |
| `Plugins` — `/plugins` | G: Plugins feature (source default true) | Y / N | — | Request plugins popup. | [dispatch:543–557](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L543-L557) |
| `Logout` — `/logout` | D | N / N | — | Emit Logout. | [dispatch:426–440](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L426-L440) |
| `Quit` — `/quit` | D | Y / N | — | Request quit without confirmation. | [dispatch:423–437](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L423-L437) |
| `Exit` — `/exit` | D | Y / N | — | Same quit-without-confirmation handler. | [dispatch:423–437](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L423-L437) |
| `Feedback` — `/feedback` | D; feedback_enabled controls actual form | Y / N | — | Enabled: category selection starts feedback flow; disabled: explanatory selection view. | [dispatch:171–185](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L171-L185) |
| `Rollout` — `/rollout` | G: debug_assertions only | Y / N | — | Print current rollout path or path-not-yet-available info. | [dispatch:546–560](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L546-L560) |
| `Ps` — `/ps` | D | Y / N | — | Print background terminals output. | [dispatch:525–539](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L525-L539) |
| `Stop` — `/stop (alias clean)` | D | Y / N | — | Stop unified exec processes; /clean alias. Not generic turn cancellation. | [dispatch:528–542](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L528-L542) |
| `Clear` — `/clear` | D | N / N | nonempty normalized name → ClearUi {name}; whitespace → unnamed | Bare ClearUi {name:None}; distinct from terminal-only Ctrl+L. | [dispatch:248–262](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L248-L262) |
| `TestApproval` — `/test-approval` | G: debug_assertions only | Y / N | — | Inject synthetic apply-patch approval UI for sample /tmp paths; this handler itself is not an applied patch. | [dispatch:559–573](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L559-L573) |
| `MultiAgents` — `/subagents` | D | Y / N | — | /subagents also emits OpenAgentsList. | [dispatch:345–359](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L345-L359) |
| `MemoryDrop` — `/debug-m-drop` | D, NOT debug-build-only | N / N | — | Submit AppCommand::drop_memories then immediately say Memories dropped (UI acknowledgment precedes observed backend outcome). | [dispatch:531–545](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L531-L545) |
| `MemoryUpdate` — `/debug-m-update` | D, NOT debug-build-only | N / N | — | Submit AppCommand::update_memories then announce memory update job triggered. | [dispatch:534–548](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L534-L548) |

### Argument and downstream checkpoints

- Export/Cd/Pwd/Usage/Voice/IDE/MCP: [chatwidget/slash_dispatch.rs:716–770](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L716-L770); exact IDE `on|off|status` switch: [chatwidget/ide_context.rs:35–67](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/ide_context.rs#L35-L67).
- Keymap/Raw/Rename/New/Clear/Fork/Plan: [chatwidget/slash_dispatch.rs:771–877](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L771-L877). New/Fork take a **name**, not invented `--worktree` CLI flags. Checkout choices and direct fallback: [chatwidget/worktree_picker.rs:8–110](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/worktree_picker.rs#L8-L110).
- Goal objective and all clear/edit/pause/resume leaves, attachment draft, pre-session queue and replacement confirmation: [chatwidget/slash_dispatch.rs:878–975](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L878-L975).
- Side/Btw, Review, Resume, every pet synonym, fallback usage: [chatwidget/slash_dispatch.rs:977–1055](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L977-L1055).
- Working-directory change is a request guarded against queued input, running processes, ephemeral/side sessions and inventory loading: [chatwidget/working_directory.rs:7–47](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/working_directory.rs#L7-L47).
- Queued handler result first stops for running/pending user turn or active modal/popup. Otherwise it is explicit: Continue for observational/simple actions; Stop for modal/session-changing/model/permission/goal/quit actions; Cd and Worktree decide conditionally. This is a queue-drain instruction, **not** command success. [chatwidget/slash_dispatch.rs:1161–1245](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L1161-L1245).
- Non-enum service-tier commands are a separate dynamic catalog: visible only with service-tier-command flag; no inline args; allowed during task, not in side conversations; toggle service tier through UI. Names/default catalog **unknown** without model metadata. [bottom_pane/slash_commands.rs:13–52](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/slash_commands.rs#L13-L52) [bottom_pane/slash_commands.rs:129–204](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/slash_commands.rs#L129-L204) [chatwidget/slash_dispatch.rs:50–64](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L50-L64). They are not counted as enum variants.

## Editor action inventory

### Default configurable key actions (complete defaults for editor-adjacent contexts)

The following is mechanically transcribed from `RuntimeKeymap::built_in_defaults`; Rust key constructors are retained to avoid losing modifier distinctions. `plain` means no modifier, `ctrl` Control, `alt` Alt, `shift` Shift, combinations are literal; an empty list means **no default binding**, not no implementation. These are defaults, not the user's effective keymap. Runtime config/chords can replace them, with conflict validation and context-specific fallback. [keymap.rs:598–657](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L598-L657) [keymap/bindings.rs:13–48](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap/bindings.rs#L13-L48) [keymap.rs:1553–1830](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1553-L1830).

| Context.action | Default key binding(s) | Action / scope |
|---|---|---|
| `app.open_agents` | `∅ (unbound)` | open agents (app focus) |
| `app.open_transcript` | `ctrl(KeyCode::Char('t'))` | open transcript (app focus) |
| `app.open_external_editor` | `ctrl(KeyCode::Char('g'))` | open external editor (app focus) |
| `app.copy` | `ctrl(KeyCode::Char('o'))` | copy (app focus) |
| `app.clear_terminal` | `ctrl(KeyCode::Char('l'))` | clear terminal (app focus) |
| `app.toggle_vim_mode` | `∅ (unbound)` | toggle vim mode (app focus) |
| `app.toggle_fast_mode` | `∅ (unbound)` | toggle fast mode (app focus) |
| `app.toggle_raw_output` | `alt(KeyCode::Char('r'))` | toggle raw output (app focus) |
| `app.toggle_side_conversation` | `ctrl(KeyCode::Char('/'))` | toggle side conversation (app focus) |
| `chat.toggle_voice_mute` | `ctrl(KeyCode::Char('x'))` | toggle voice mute (chat focus) |
| `chat.interrupt_turn` | `plain(KeyCode::Esc)` | interrupt turn (chat focus) |
| `chat.decrease_reasoning_effort` | `alt(KeyCode::Char(',')), shift(KeyCode::Down)` | decrease reasoning effort (chat focus) |
| `chat.increase_reasoning_effort` | `alt(KeyCode::Char('.')), shift(KeyCode::Up)` | increase reasoning effort (chat focus) |
| `chat.previous_permission_mode` | `∅ (unbound)` | previous permission mode (chat focus) |
| `chat.next_permission_mode` | `∅ (unbound)` | next permission mode (chat focus) |
| `chat.edit_queued_message` | `alt(KeyCode::Up), shift(KeyCode::Left)` | edit queued message (chat focus) |
| `chat.prompt_stack_back` | `alt(KeyCode::Down), shift(KeyCode::Right)` | prompt stack back (chat focus) |
| `chat.skip_question` | `ctrl(KeyCode::Char(']'))` | skip question (chat focus) |
| `composer.submit` | `plain(KeyCode::Enter)` | submit (composer focus) |
| `composer.queue` | `plain(KeyCode::Tab)` | queue (composer focus) |
| `composer.toggle_shortcuts` | `plain(KeyCode::Char('?')), shift(KeyCode::Char('?'))` | toggle shortcuts (composer focus) |
| `composer.history_search_previous` | `ctrl(KeyCode::Char('r'))` | history search previous (composer focus) |
| `composer.history_search_next` | `ctrl(KeyCode::Char('s'))` | history search next (composer focus) |
| `editor.insert_newline` | `ctrl(KeyCode::Char('j')), ctrl(KeyCode::Char('m')), plain(KeyCode::Enter), shift(KeyCode::Enter), alt(KeyCode::Enter)` | insert newline (editor focus) |
| `editor.move_left` | `plain(KeyCode::Left), ctrl(KeyCode::Char('b'))` | move left (editor focus) |
| `editor.move_right` | `plain(KeyCode::Right), ctrl(KeyCode::Char('f'))` | move right (editor focus) |
| `editor.move_up` | `plain(KeyCode::Up), ctrl(KeyCode::Char('p'))` | move up (editor focus) |
| `editor.move_down` | `plain(KeyCode::Down), ctrl(KeyCode::Char('n'))` | move down (editor focus) |
| `editor.move_word_left` | `alt(KeyCode::Char('b')), raw(KeyBinding::new(KeyCode::Left, KeyModifiers::ALT)), raw(KeyBinding::new(KeyCode::Left, KeyModifiers::CONTROL))` | move word left (editor focus) |
| `editor.move_word_right` | `alt(KeyCode::Char('f')), raw(KeyBinding::new(KeyCode::Right, KeyModifiers::ALT)), raw(KeyBinding::new(KeyCode::Right, KeyModifiers::CONTROL))` | move word right (editor focus) |
| `editor.move_line_start` | `plain(KeyCode::Home), ctrl(KeyCode::Char('a'))` | move line start (editor focus) |
| `editor.move_line_end` | `plain(KeyCode::End), ctrl(KeyCode::Char('e'))` | move line end (editor focus) |
| `editor.delete_backward` | `plain(KeyCode::Backspace), shift(KeyCode::Backspace), ctrl(KeyCode::Char('h'))` | delete backward (editor focus) |
| `editor.delete_forward` | `plain(KeyCode::Delete), shift(KeyCode::Delete), ctrl(KeyCode::Char('d'))` | delete forward (editor focus) |
| `editor.delete_backward_word` | `alt(KeyCode::Backspace), ctrl(KeyCode::Backspace), raw(KeyBinding::new( KeyCode::Backspace, KeyModifiers::CONTROL \| KeyModifiers::SHIFT, )), ctrl(KeyCode::Char('w')), raw(KeyBinding::new( KeyCode::Char('h'), KeyModifiers::CONTROL \| KeyModifiers::ALT, ))` | delete backward word (editor focus) |
| `editor.delete_forward_word` | `alt(KeyCode::Delete), ctrl(KeyCode::Delete), raw(KeyBinding::new( KeyCode::Delete, KeyModifiers::CONTROL \| KeyModifiers::SHIFT, )), alt(KeyCode::Char('d'))` | delete forward word (editor focus) |
| `editor.kill_line_start` | `ctrl(KeyCode::Char('u'))` | kill line start (editor focus) |
| `editor.kill_whole_line` | `∅ (unbound)` | kill whole line (editor focus) |
| `editor.kill_line_end` | `ctrl(KeyCode::Char('k'))` | kill line end (editor focus) |
| `editor.yank` | `ctrl(KeyCode::Char('y'))` | yank (editor focus) |
| `vim_normal.enter_insert` | `plain(KeyCode::Char('i')), plain(KeyCode::Insert)` | enter insert (vim_normal focus) |
| `vim_normal.append_after_cursor` | `plain(KeyCode::Char('a'))` | append after cursor (vim_normal focus) |
| `vim_normal.append_line_end` | `shift(KeyCode::Char('a')), plain(KeyCode::Char('A'))` | append line end (vim_normal focus) |
| `vim_normal.insert_line_start` | `shift(KeyCode::Char('i')), plain(KeyCode::Char('I'))` | insert line start (vim_normal focus) |
| `vim_normal.open_line_below` | `plain(KeyCode::Char('o'))` | open line below (vim_normal focus) |
| `vim_normal.open_line_above` | `shift(KeyCode::Char('o')), plain(KeyCode::Char('O'))` | open line above (vim_normal focus) |
| `vim_normal.enter_replace_mode` | `shift(KeyCode::Char('r')), plain(KeyCode::Char('R'))` | enter replace mode (vim_normal focus) |
| `vim_normal.move_left` | `plain(KeyCode::Char('h')), plain(KeyCode::Left)` | move left (vim_normal focus) |
| `vim_normal.move_right` | `plain(KeyCode::Char('l')), plain(KeyCode::Right)` | move right (vim_normal focus) |
| `vim_normal.move_up` | `plain(KeyCode::Char('k')), plain(KeyCode::Up)` | move up (vim_normal focus) |
| `vim_normal.move_down` | `plain(KeyCode::Char('j')), plain(KeyCode::Down)` | move down (vim_normal focus) |
| `vim_normal.move_word_forward` | `plain(KeyCode::Char('w'))` | move word forward (vim_normal focus) |
| `vim_normal.move_word_backward` | `plain(KeyCode::Char('b'))` | move word backward (vim_normal focus) |
| `vim_normal.move_word_end` | `plain(KeyCode::Char('e'))` | move word end (vim_normal focus) |
| `vim_normal.move_line_start` | `plain(KeyCode::Char('0'))` | move line start (vim_normal focus) |
| `vim_normal.move_line_end` | `plain(KeyCode::Char('$')), shift(KeyCode::Char('$'))` | move line end (vim_normal focus) |
| `vim_normal.find_forward` | `plain(KeyCode::Char('f'))` | find forward (vim_normal focus) |
| `vim_normal.find_backward` | `shift(KeyCode::Char('f'))` | find backward (vim_normal focus) |
| `vim_normal.till_forward` | `plain(KeyCode::Char('t'))` | till forward (vim_normal focus) |
| `vim_normal.till_backward` | `shift(KeyCode::Char('t'))` | till backward (vim_normal focus) |
| `vim_normal.jump_top` | `∅ (unbound)` | jump top (vim_normal focus) |
| `vim_normal.jump_bottom` | `shift(KeyCode::Char('g')), plain(KeyCode::Char('G'))` | jump bottom (vim_normal focus) |
| `vim_normal.delete_char` | `plain(KeyCode::Char('x'))` | delete char (vim_normal focus) |
| `vim_normal.replace_char` | `plain(KeyCode::Char('r'))` | replace char (vim_normal focus) |
| `vim_normal.repeat_last_change` | `plain(KeyCode::Char('.'))` | repeat last change (vim_normal focus) |
| `vim_normal.substitute_char` | `plain(KeyCode::Char('s'))` | substitute char (vim_normal focus) |
| `vim_normal.delete_to_line_end` | `shift(KeyCode::Char('d')), plain(KeyCode::Char('D'))` | delete to line end (vim_normal focus) |
| `vim_normal.change_to_line_end` | `shift(KeyCode::Char('c')), plain(KeyCode::Char('C'))` | change to line end (vim_normal focus) |
| `vim_normal.yank_line` | `shift(KeyCode::Char('y')), plain(KeyCode::Char('Y'))` | yank line (vim_normal focus) |
| `vim_normal.paste_after` | `plain(KeyCode::Char('p'))` | paste after (vim_normal focus) |
| `vim_normal.start_delete_operator` | `plain(KeyCode::Char('d'))` | start delete operator (vim_normal focus) |
| `vim_normal.start_yank_operator` | `plain(KeyCode::Char('y'))` | start yank operator (vim_normal focus) |
| `vim_normal.start_change_operator` | `plain(KeyCode::Char('c'))` | start change operator (vim_normal focus) |
| `vim_normal.undo` | `plain(KeyCode::Char('u'))` | undo (vim_normal focus) |
| `vim_normal.redo` | `ctrl(KeyCode::Char('r'))` | redo (vim_normal focus) |
| `vim_normal.cancel_operator` | `plain(KeyCode::Esc)` | cancel operator (vim_normal focus) |
| `vim_operator.delete_line` | `plain(KeyCode::Char('d'))` | delete line (vim_operator focus) |
| `vim_operator.yank_line` | `plain(KeyCode::Char('y'))` | yank line (vim_operator focus) |
| `vim_operator.motion_left` | `plain(KeyCode::Char('h'))` | motion left (vim_operator focus) |
| `vim_operator.motion_right` | `plain(KeyCode::Char('l'))` | motion right (vim_operator focus) |
| `vim_operator.motion_up` | `plain(KeyCode::Char('k'))` | motion up (vim_operator focus) |
| `vim_operator.motion_down` | `plain(KeyCode::Char('j'))` | motion down (vim_operator focus) |
| `vim_operator.motion_word_forward` | `plain(KeyCode::Char('w'))` | motion word forward (vim_operator focus) |
| `vim_operator.motion_word_backward` | `plain(KeyCode::Char('b'))` | motion word backward (vim_operator focus) |
| `vim_operator.motion_word_end` | `plain(KeyCode::Char('e'))` | motion word end (vim_operator focus) |
| `vim_operator.motion_line_start` | `plain(KeyCode::Char('0'))` | motion line start (vim_operator focus) |
| `vim_operator.motion_line_end` | `plain(KeyCode::Char('$')), shift(KeyCode::Char('$'))` | motion line end (vim_operator focus) |
| `vim_operator.motion_find_forward` | `plain(KeyCode::Char('f'))` | motion find forward (vim_operator focus) |
| `vim_operator.motion_find_backward` | `shift(KeyCode::Char('f'))` | motion find backward (vim_operator focus) |
| `vim_operator.motion_till_forward` | `plain(KeyCode::Char('t'))` | motion till forward (vim_operator focus) |
| `vim_operator.motion_till_backward` | `shift(KeyCode::Char('t'))` | motion till backward (vim_operator focus) |
| `vim_operator.motion_jump_top` | `∅ (unbound)` | motion jump top (vim_operator focus) |
| `vim_operator.motion_jump_bottom` | `shift(KeyCode::Char('g')), plain(KeyCode::Char('G'))` | motion jump bottom (vim_operator focus) |
| `vim_operator.select_inner_text_object` | `plain(KeyCode::Char('i'))` | select inner text object (vim_operator focus) |
| `vim_operator.select_around_text_object` | `plain(KeyCode::Char('a'))` | select around text object (vim_operator focus) |
| `vim_operator.cancel` | `plain(KeyCode::Esc)` | cancel (vim_operator focus) |
| `vim_text_object.word` | `plain(KeyCode::Char('w'))` | word (vim_text_object focus) |
| `vim_text_object.big_word` | `shift(KeyCode::Char('w')), plain(KeyCode::Char('W'))` | big word (vim_text_object focus) |
| `vim_text_object.parentheses` | `plain(KeyCode::Char('(')), shift(KeyCode::Char('(')), plain(KeyCode::Char(')')), shift(KeyCode::Char(')')), plain(KeyCode::Char('b'))` | parentheses (vim_text_object focus) |
| `vim_text_object.brackets` | `plain(KeyCode::Char('[')), plain(KeyCode::Char(']'))` | brackets (vim_text_object focus) |
| `vim_text_object.braces` | `plain(KeyCode::Char('{')), shift(KeyCode::Char('{')), plain(KeyCode::Char('}')), shift(KeyCode::Char('}')), shift(KeyCode::Char('b')), plain(KeyCode::Char('B'))` | braces (vim_text_object focus) |
| `vim_text_object.double_quote` | `plain(KeyCode::Char('"')), shift(KeyCode::Char('"'))` | double quote (vim_text_object focus) |
| `vim_text_object.single_quote` | `plain(KeyCode::Char('\''))` | single quote (vim_text_object focus) |
| `vim_text_object.backtick` | `plain(KeyCode::Char('`'))` | backtick (vim_text_object focus) |
| `vim_text_object.cancel` | `plain(KeyCode::Esc)` | cancel (vim_text_object focus) |
| `pager.scroll_up` | `plain(KeyCode::Up), plain(KeyCode::Char('k'))` | scroll up (pager focus) |
| `pager.scroll_down` | `plain(KeyCode::Down), plain(KeyCode::Char('j'))` | scroll down (pager focus) |
| `pager.page_up` | `plain(KeyCode::PageUp), shift(KeyCode::Char(' ')), ctrl(KeyCode::Char('b'))` | page up (pager focus) |
| `pager.page_down` | `plain(KeyCode::PageDown), plain(KeyCode::Char(' ')), ctrl(KeyCode::Char('f'))` | page down (pager focus) |
| `pager.half_page_up` | `ctrl(KeyCode::Char('u'))` | half page up (pager focus) |
| `pager.half_page_down` | `ctrl(KeyCode::Char('d'))` | half page down (pager focus) |
| `pager.jump_top` | `plain(KeyCode::Home)` | jump top (pager focus) |
| `pager.jump_bottom` | `plain(KeyCode::End)` | jump bottom (pager focus) |
| `pager.close` | `plain(KeyCode::Char('q')), ctrl(KeyCode::Char('c'))` | close (pager focus) |
| `pager.close_transcript` | `ctrl(KeyCode::Char('t'))` | close transcript (pager focus) |
| `list.move_up` | `plain(KeyCode::Up), ctrl(KeyCode::Char('p')), ctrl(KeyCode::Char('k')), plain(KeyCode::Char('k'))` | move up (list focus) |
| `list.move_down` | `plain(KeyCode::Down), ctrl(KeyCode::Char('n')), ctrl(KeyCode::Char('j')), plain(KeyCode::Char('j'))` | move down (list focus) |
| `list.move_left` | `plain(KeyCode::Left), ctrl(KeyCode::Char('h'))` | move left (list focus) |
| `list.move_right` | `plain(KeyCode::Right), ctrl(KeyCode::Char('l'))` | move right (list focus) |
| `list.page_up` | `plain(KeyCode::PageUp), ctrl(KeyCode::Char('b'))` | page up (list focus) |
| `list.page_down` | `plain(KeyCode::PageDown), ctrl(KeyCode::Char('f'))` | page down (list focus) |
| `list.jump_top` | `plain(KeyCode::Home)` | jump top (list focus) |
| `list.jump_bottom` | `plain(KeyCode::End)` | jump bottom (list focus) |
| `list.accept` | `plain(KeyCode::Enter)` | accept (list focus) |
| `list.cancel` | `plain(KeyCode::Esc)` | cancel (list focus) |

Vim search defaults are declared separately: `/` forward, `?` backward, `n` next, `N` previous; configured Vim normal/operator/chord bindings can shadow defaults, explicit collisions error. [keymap/vim_search.rs:10–77](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap/vim_search.rs#L10-L77). Vim normal/operator/text-object table lists the actual supported action set, not full Vim compatibility: Insert/Normal/Replace states, counts, motions, operators, character-find, inner/around objects, undo/redo and dot replay are implemented; no claim of full visual/ex-command/macros compatibility. [bottom_pane/textarea/vim.rs:1–88](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/textarea/vim.rs#L1-L88). `/vim` toggles current composer state with an info notice: [chatwidget.rs:1749–1758](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget.rs#L1749-L1758).

### Routing, submission, fixed keys and recovery

| Action | Default behavior / gates / outcome | Source |
|---|---|---|
| Submit vs queue | Enter submits; Tab is queue outside popups, but completion/selection inside them. Slash preparation is separate from normal text; pending paste bursts can defer submission and absorb Enter as newline. | [bottom_pane/chat_composer.rs:1–111](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1-L111) [bottom_pane/chat_composer.rs:2320–2555](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2320-L2555) |
| Multiline editing | Ctrl+J/Ctrl+M, Alt+Enter and Shift+Enter insert newline in editor handling; plain Enter also belongs to editor defaults but the composer submit route takes precedence; terminal must actually distinguish reported key events. Grapheme-aware textarea and atomic element ranges prevent splitting structured placeholders. | [keymap.rs:1600–1645](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1600-L1645) [bottom_pane/textarea.rs:1–95](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/textarea.rs#L1-L95) |
| Ordinary kill/yank | Backspace/Ctrl+H, Delete/Ctrl+D, word-delete aliases, Ctrl+U/K kill line portions, Ctrl+Y yank; cursor motions listed exhaustively above. Ctrl+D is editor delete in text, not always quit. | [keymap.rs:1600–1645](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1600-L1645) |
| Quit / cancel | Ctrl+C routing is context-sensitive: cancel active view/turn or clear input; arm quit hint and repeat to exit where applicable. Ctrl+D likewise has empty-input quit path; `/quit` and `/exit` request without confirmation. Do not equate Escape, Ctrl+C, `/stop`, `/clear`, and process exit. | [chatwidget/interaction.rs:515–640](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L515-L640) [chatwidget/slash_dispatch.rs:423–428](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L423-L428) |
| Backtrack | Esc-Esc backtracking is enabled in regular composer with no running task/modal/popup; first Escape can be consumed by Vim insert escape or hints/popup dismissal. This is distinct from shell-like input history. | [chatwidget.rs:1759–1795](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget.rs#L1759-L1795) [app/input.rs:243–335](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/input.rs#L243-L335) |
| Global editor/pager keys | Ctrl+G external editor, Ctrl+T transcript, Ctrl+O copy picker, Ctrl+L terminal clear, Alt+R raw, Ctrl+/ side toggle. Ctrl+L is refused during a task. Remapped/chord input must respect active focus and reserved bindings. | [keymap.rs:1553–1600](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1553-L1600) [chatwidget/interaction.rs:277–300](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L277-L300) |
| Running-turn controls | Esc interrupt, reasoning-effort Alt+,/Alt+. or Shift+Down/Up; Alt+Up/Shift+Left edits queued message; Alt+Down/Shift+Right prompt-stack back; Ctrl+] skip question; Ctrl+X voice mute only applicable voice state. Actual effects depend on active task/question/audio state. | [keymap.rs:1567–1592](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1567-L1592) [chatwidget/interaction.rs:16–120](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L16-L120) |
| Restricted composer | Modal/plain-text configuration disables popups, slash and shell commands, image-path attachment behavior; not every text box is a full chat composer. | [bottom_pane/chat_composer.rs:475–524](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L475-L524) |

### Popups and focus actions

| Surface/action | Trigger, navigation, selection, cancellation and result | Source |
|---|---|---|
| Slash discovery | Leading slash first-line token opens command popup; fuzzy match favors direct prefix and preserves stable presentation rank. Busy commands can remain visible but disabled; side-gated commands are filtered. | [bottom_pane/command_popup.rs:208–340](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/command_popup.rs#L208-L340) [bottom_pane/chat_composer/slash_input.rs:132–235](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs#L132-L235) |
| Slash select | Up/Down and Ctrl+P/N move; Escape closes. Tab normally completes selected command and preserves inline tail/structured draft; **Skills dispatches immediately on Tab**. Enter dispatches bare selected command or prepares supported inline args; service-tier route separate. | [bottom_pane/chat_composer/slash_input.rs:239–432](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs#L239-L432) [bottom_pane/chat_composer/slash_input.rs:514–530](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs#L514-L530) |
| File mentions | `@` or `/mention`; asynchronous file-search results are query-scoped. Up/Ctrl+P and Down/Ctrl+N select. Enter/Tab accept file/folder path, replacing mention token; Escape remembers dismissed query so it does not immediately reopen. Absolute paths outside cwd preserved; directories with spaces quoted. | [bottom_pane/chat_composer.rs:1920–1950](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1920-L1950) [bottom_pane/chat_composer.rs:2164–2270](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2164-L2270) |
| Skill/app mention chooser | Skill popup takes precedence where applicable; Enter/Tab applies selected skill or app mention as a structured binding; Escape records dismissed token and closes. Exact available skills/apps are environment-dependent, not a fixed enum list. | [bottom_pane/chat_composer.rs:2270–2390](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2270-L2390) [chatwidget/connectors.rs:150–165](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/connectors.rs#L150-L165) |
| Generic selection/list | Configurable arrows/Ctrl+P/N/K/J, page keys/Ctrl+B/F, Home/End, Enter accept, Escape cancel; plain j/k can be interpreted by a searchable view as query input rather than navigation. Do not blindly project generic bindings into the composer-specific popup handlers above. | [keymap.rs:1792–1820](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1792-L1820) [bottom_pane/selection_popup_common.rs:1–100](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/selection_popup_common.rs#L1-L100) |
| Transcript pager | Ctrl+T opens; movement/page/half-page/top/bottom bindings above; q/Ctrl+C close, Ctrl+T close transcript; ordinary composer input is not focused behind pager. | [keymap.rs:1768–1791](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1768-L1791) |
| Shortcut hint | `?` toggles shortcut footer in the appropriate empty-input state; otherwise printable question mark is text. Hints derive from effective keymap, not universal terminal shortcuts. | [keymap.rs:1590–1600](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1590-L1600) [bottom_pane/chat_composer.rs:1–56](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1-L56) |

### History, paste and attachments

| Action | Default / gated behavior, data preservation and failure boundary | Source |
|---|---|---|
| Previous/next draft history | Up/Ctrl+P and Down/Ctrl+N navigate at appropriate cursor/history boundaries, not in the middle of ordinary multiline movement. Two stores: persisted text-only session history and local full-fidelity history. Draft snapshot restored on return. | [bottom_pane/chat_composer.rs:47–65](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L47-L65) [bottom_pane/chat_composer_history.rs:1–87](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer_history.rs#L1-L87) |
| History fidelity | Local history carries text elements, pending large pastes, local/remote images and mentions. Persistent recalled text does not magically restore attachments; image placeholders stripped. History can request asynchronous older entries and deduplicate. | [bottom_pane/chat_composer_history.rs:1–110](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer_history.rs#L1-L110) [bottom_pane/chat_composer.rs:1658–1692](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1658-L1692) |
| Reverse/forward search | Ctrl+R starts/steps backward; Ctrl+S forward; query updates asynchronously with searching/match/no-match state. Enter accepts matched draft **without submitting**; Escape restores original draft. Backspace/Ctrl+H edits query, Ctrl+U clears. | [bottom_pane/chat_composer/history_search.rs:1–250](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer/history_search.rs#L1-L250) |
| Explicit paste | CRLF/CR normalize to LF. More than 1000 characters stored behind `[Pasted Content …]` atomic placeholder; short text inserts directly unless detected image path. History navigation resets. | [bottom_pane/chat_composer.rs:369–370](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L369-L370) [bottom_pane/chat_composer.rs:1230–1282](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1230-L1282) |
| Unbracketed burst paste | Timing state buffers rapid ASCII, can retro-grab prior characters, holds Enter as newline during burst window, flushes on timeout/before modified input. Non-ASCII/IME path avoids first-character hold. Config can disable heuristic. This reduces accidental multiline submission but remains timing/terminal-dependent. | [bottom_pane/paste_burst.rs:1–148](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/paste_burst.rs#L1-L148) [bottom_pane/chat_composer.rs:65–99](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L65-L99) |
| Submit/expand pasted text | Expand pending payloads by element ranges, trim/rebase remaining spans, remove unused image attachments, preserve remote URLs even for image-only message. Deleting an atomic placeholder is not equivalent to submitting hidden stale data. | [bottom_pane/chat_composer.rs:99–111](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L99-L111) [bottom_pane/chat_composer.rs:2555–2685](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L2555-L2685) |
| Paste image path | If image paste enabled and path recognized/openable, normalize path, decode dimensions and attach; failed decode falls back to text insertion. Generic pasted paths are not arbitrary file uploads. | [bottom_pane/chat_composer.rs:1230–1282](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1230-L1282) [clipboard_paste.rs:1–130](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/clipboard_paste.rs#L1-L130) |
| Clipboard image shortcut | Ctrl+V/Control+Shift+V recognized with optional Alt; WSL footer recommends Ctrl+Alt+V. Capability check reports unsupported model before attachment. Clipboard image converted to temp PNG; success attaches and prints dimensions/MIME, error warns. Active overlay/image-input-enabled state gates handling; ordinary terminal text paste remains a separate path. | [chatwidget/interaction.rs:64–116](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L64-L116) [bottom_pane/footer.rs:1217–1238](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/footer.rs#L1217-L1238) [clipboard_paste.rs:1–130](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/clipboard_paste.rs#L1-L130) |
| Local images | Inline atomic `[Image #N]` placeholders tied to attachment state; pruning/relabeling follows edits. Image capability checked again at chat layer, not inferred from successful local decode. | [bottom_pane/chat_composer.rs:99–163](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L99-L163) [bottom_pane/chat_composer.rs:1871–1900](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1871-L1900) |
| Remote images | Noneditable rows above textarea, not URLs injected as editable text. Up at cursor 0 selects last row; Up/Down navigate and return to textarea; Delete/Backspace removes selected row. Remote prefix and local image placeholder numbering unified. | [bottom_pane/chat_composer.rs:148–164](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L148-L164) |
| External-edit attachment reconciliation | Expand pending pastes for editor seed, preserve only surviving known local-image placeholders with multiplicity, rebuild atomic spans, relabel images, reconcile bindings; arbitrary lookalike text is not a new attachment. | [bottom_pane/chat_composer.rs:1305–1420](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1305-L1420) |

### External editor lifecycle and platform boundary

- **Action:** Ctrl+G by default; only if not external-writer view, no active modal/composer popup and no expanded question. No blanket running-task prohibition in `can_launch_external_editor`. Requested/Open/Closed state and footer hint prevent presenting an editor request as completion. [chatwidget/interaction.rs:277–279](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L277-L279) [bottom_pane/mod.rs:1635–1648](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/mod.rs#L1635-L1648) [app/input.rs:147–216](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/input.rs#L147-L216)
- **Selection:** `VISUAL` if present, otherwise `EDITOR` when environment lookup fails; a present empty VISUAL does not fall back and yields EmptyCommand. Unix uses shlex, Windows winsplit; no implicit shell expansion. Missing/bad command returns error. No invented vi fallback. [external_editor.rs:1–58](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/external_editor.rs#L1-L58)
- **File boundary:** choose validated non-agent-writable editor directory under candidate Codex homes, with Unix/project and Windows-specific directory handling. Temporary `.md` seed file closes handle before spawn (Windows compatibility). Program plus arguments plus temp path, inherited stdin/stdout/stderr; Windows resolves program shim. This is local human editor execution, not a provider tool. [external_editor.rs:49–223](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/external_editor.rs#L49-L223)
- **TUI handoff:** clear prior hint, reject empty command, expand seed, temporarily restore terminal modes/run editor, then reset state. Success trims trailing newlines and applies draft only (does not submit); nonzero exit, spawn/read/directory errors become `Failed to open editor` history error. The preexisting draft is not replaced on error. [app/input.rs:147–200](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/input.rs#L147-L200)
- **Unknown:** actual editor binary, terminal restoration under crashes/signals, native macOS/Windows clipboard and temp-directory behavior were not executed. Source guards are evidence of design, not platform certification.

## UX lessons for Helm (recommendations, not Helm implementation claims)

1. **One discoverability/dispatch contract:** share gating rules, but retain typed-command recognition where a precise unavailable explanation is better than “unknown.” Codex's Usage/side exceptions show the distinction. Include effective gate reason in a future Helm matrix rather than treating hidden = absent.
2. **Track action stages:** “opens picker”, “emits request”, “queued”, “accepted”, and “finished” must be separate outcomes. The immediate “Memories dropped” notice is a caution: request submission alone does not establish backend completion.
3. **Separate cancellation scopes:** offer distinct labels and feedback for cancelling popup, clearing draft, interrupting turn, stopping terminals, archiving/deleting session and exiting client. Bindings are contextual; exposing a shortcut list without focus conditions is misleading.
4. **Keep high-risk actions recoverable:** archive/delete confirmation defaults to No; deletion spells out irreversibility and child-thread consequences. Permission selection still needs runtime policy checks. Preserve that boundary rather than mistaking UI choice for authority.
5. **Treat draft as structured data:** history/queued editing/paste/external editor must preserve attachments and mention identity, not only visible placeholder text. Explicitly document lossy persistent history versus rich in-session recall.
6. **Avoid completion surprises:** Codex Tab usually completes but dispatches Skills; record such exceptions visibly. Fuzzy discovery, disabled busy rows and aliases help only if keyboard consequences are predictable.
7. **External editor deserves an explicit handoff state:** display Requested/Open/Closed, restore terminal modes and draft on error, distinguish editing from sending, and keep local-human execution separate from remote/agent authority.
8. **Configurable shortcuts require effective diagnostics:** `/keymap debug`, shared action inventory and collision validation are useful models; account for terminal/WSL aliases without asserting all terminals transmit them identically.

## Reproducible scenario backlog — NOT EXECUTED

- Compare popup and typed behavior for every enum entry in release/debug, Android/Linux/macOS/Windows builds; test backend-auth absent, feature disabled, active task, side thread and review mode. No platform pass implied.
- Exercise all inline leaves and invalid args; unknown slash vs leading-space literal vs slash path; aliases and `/gooal`; confirm non-inline commands do not acquire invented arguments.
- Queue `/export`, `/cd`, `/new name`, `/plan prompt`, `/goal objective` across pre-session/start/busy transitions; verify Stop/Continue drain behavior and restored drafts.
- Test file/skill/app popup cancellation and stale asynchronous search result; Tab/Enter with inline tails and attached images.
- Paste CRLF, >1000-char text, rapid unbracketed multiline, IME and fake placeholders; delete/reorder attachments; reverse-search local and persisted history; confirm Enter accepts search but does not submit.
- External editor missing/bad/nonzero exit, temp-directory rejection, successful edits, deleted/duplicated image placeholders, concurrent running turn and abnormal termination; verify draft/terminal restoration independently on each native platform.
- Verify UI-request acknowledgments against actual async failures for compact, memories, diff, export and directory change. Provider-dependent scenarios require a separate approved environment/budget, not this audit.

## Source manifest

All citations are immutable GitHub blob URLs at the full pin. Manifest below records SHA-256 of each cited local source file so the reviewed inputs can be compared without relying on branch-head URLs. Only the compact audit fragment is written; upstream source is not modified. No link HTTP availability check was performed; paths and anchor ranges were validated locally.

| Pinned source file | SHA-256 of local reviewed input |
|---|---|
| `codex-rs/features/src/lib.rs` | `a21065af423dab633499802df24e7b4fd7330bc47ff7e43180eef7f70939a350` |
| `codex-rs/tui/src/app/input.rs` | `96c3e1535cb720488a3e74054178a191b9e12380bfb2164b1923f525f0542bc8` |
| `codex-rs/tui/src/bottom_pane/chat_composer.rs` | `5dec0ca733a74304a4b0025429655201778fdc03b9acca1198ac881274bf7602` |
| `codex-rs/tui/src/bottom_pane/chat_composer/history_search.rs` | `2b85458c46e8769b4ee998f974eaa6ef3dd84f7a34c61a82429cbac19c028641` |
| `codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs` | `b80bad370a5eedb258db7b8e63b6a248638cc150ecca77bacc4871cb1af82f49` |
| `codex-rs/tui/src/bottom_pane/chat_composer_history.rs` | `781e1409025bf0af4f75a011b3ff8cbff407b514e5579c074e9faaf07efe7e4f` |
| `codex-rs/tui/src/bottom_pane/command_popup.rs` | `b915bc0a5a982650ede4fce8aa857e22c2459cd6f978bb4d281bb1b70c8b570c` |
| `codex-rs/tui/src/bottom_pane/footer.rs` | `e1cabdb6bfb494738b11f1f4cc93dab014d342a384f5802a51b13a290447a466` |
| `codex-rs/tui/src/bottom_pane/mod.rs` | `598b2c6e293c557dfb1e20ef2ceb82ac5b94db81a51fc8c4f25325db54ec0c1c` |
| `codex-rs/tui/src/bottom_pane/paste_burst.rs` | `59d39542b7a8adb5dc69c4847975b1af0e590ab412ded9cfe7e944297fcad4cf` |
| `codex-rs/tui/src/bottom_pane/selection_popup_common.rs` | `15e0ad4bd77a742a472c252c267597b2eb86c7d95ac3356a306d1a17c7964ffc` |
| `codex-rs/tui/src/bottom_pane/slash_commands.rs` | `49f7dcd39054af929f6c36f8b2c4043fe1c5af7b562fe0385cfcbeccadee5797` |
| `codex-rs/tui/src/bottom_pane/textarea.rs` | `ed8f5f35ca8eef9b586faf617ca990565a39c6af4e6760358f503408387c5b6d` |
| `codex-rs/tui/src/bottom_pane/textarea/vim.rs` | `a7fbdf9166d84a0eab7f6caf354b43334d5391a1f2935905b3622d7d862fe6c4` |
| `codex-rs/tui/src/chatwidget.rs` | `28c0ff76f5dbab70932ebe700d6854a51071918b54af3787bed497d23cdd83fe` |
| `codex-rs/tui/src/chatwidget/connectors.rs` | `d75d69df22d14f33eeb4a05ab25f6b64e4fb16fa76144c4199ee740f4f47a8d3` |
| `codex-rs/tui/src/chatwidget/ide_context.rs` | `3a949a29e834d5eb21d71161d9a7bc802f9e5eaa57e525c538b296dfd3848959` |
| `codex-rs/tui/src/chatwidget/interaction.rs` | `c07d59a9dd80a55c6680d91dcadafebcde22f64e7d2f8f1735879f53e7c36fc4` |
| `codex-rs/tui/src/chatwidget/settings.rs` | `7ce94cd12777b48302074aaab7b2c2a88a0841ec5f280766023004daaa24bd3b` |
| `codex-rs/tui/src/chatwidget/slash_dispatch.rs` | `3100759256a1049326982d59a801d7c8d47fedeb7ab22d02faad61767c57e86a` |
| `codex-rs/tui/src/chatwidget/working_directory.rs` | `391117c29078bf2e431a7d12952defb483eb7d7542e76dbdea986634cc160d79` |
| `codex-rs/tui/src/chatwidget/worktree_picker.rs` | `f6fffe4292af9e61d28ecac18014403657eefc03572b566c72a78da303b50f3b` |
| `codex-rs/tui/src/clipboard_paste.rs` | `9f426c24d68a9fb1a638d1742ca73d00cfaef60e71b129c58fc8ad7498a1367b` |
| `codex-rs/tui/src/external_editor.rs` | `530d05c2c5ab0fe6bce502c4e1f5c4910db73114ba5ce1b2756bb1a8d9ecf915` |
| `codex-rs/tui/src/keymap.rs` | `67fb23246b89ff6a2a33c5c0b77676172bf8cb88805986b7ca934d69673a8ec7` |
| `codex-rs/tui/src/keymap/bindings.rs` | `fcda06192f2fa72c082bcb5b765ff5c8304743b44e03532c628753b5e8604368` |
| `codex-rs/tui/src/keymap/vim_search.rs` | `7649f552bd46f3c10baddf61d4fe3ddcdf7690fd84e054fe97aba7c0f2c7420f` |
| `codex-rs/tui/src/slash_command.rs` | `2141ca063111eaabb2851b41063e9de55938f7a832059e26470a1ae6c9133a16` |

## Verification and limits

Static verification: all enum variants have exactly one inventory row; busy/side columns derived from enum methods; default configurable bindings transcribed from source, with separate Vim search defaults. Every cited local path and line range is checked, and all blob URLs use the required pin. No focused test was executed, so source tests/comments are not reported as passing tests. Exhaustiveness is at the slash enum and editor action level; model-provided catalogs, every nested settings modal, provider outcomes and unrelated CLI/startup actions are outside this fragment.

---

## Session, resume, fork, review, diff and undo actions

All behavior below is **source-observed, not runtime-tested**. `app-server` source is used only where it implements a terminal action, not as an audit of the desktop application.

| Action / discovery | Focus and resulting state | Safety, failure and recovery | Evidence |
|---|---|---|---|
| `/new [name]`, `/clear` | Fresh conversation; clear also clears the terminal. With managed worktrees available, new asks current checkout versus new worktree. This is not deletion of the previous saved history. | Fresh startup loads config, starts the replacement, and retains a resume hint; initialization failure is surfaced. Do not equate terminal clearing with privacy erasure. | [dispatch](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L158-L240), [fresh lifecycle](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/session_lifecycle.rs#L914-L1035), [checkout choice](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/worktree_picker.rs#L12-L78) |
| `/resume`; CLI `resume` selector/ID/name/last variants | Saved-thread chooser rather than creating a historyless chat. In-app switching retains per-thread composer/input state and redraws on exit. | Picker startup/load failures add an error and permit queued input to continue; cancel preserves the existing in-app view, unlike startup's start-fresh meaning. | [picker transition](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/session_picker.rs#L1-L158), [CLI inventory below](#cli-config-source-audit) |
| Search and navigate saved sessions | Plain text edits search before plain-character navigation. Modified list keys navigate; Enter accepts. Tab/BackTab move toolbar focus, left/right cycle its value. | Esc first clears a nonempty query; empty-query cancel returns StartFresh; Ctrl+C exits picker. Actual enclosing context determines whether that exits the program. | [key router](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/resume_picker.rs#L1222-L1416) |
| Preview, expand, density; sorting/filter/status | Ctrl+T opens selected transcript; Ctrl+E toggles selected expansion; Ctrl+O changes density. Toolbar exposes sorting, cwd filtering and active/archived status. | Density persistence, pagination and loading are separate asynchronous states. Do not call the default cwd-filtered list a global exhaustive history list. | [keys](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/resume_picker.rs#L1248-L1380), [controls](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/resume_picker.rs#L1849-L1943), [query params](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/resume_picker.rs#L2052-L2081) |
| Resume into a different cwd | Config selects current or saved-session cwd, otherwise a difference opens a cwd prompt; explicit cwd override selects current. | If Session mode cannot determine saved cwd, it errors rather than guessing. Prompt outcome can exit and causes auth refresh handling. | [cwd resolution](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/session_resume.rs#L18-L112) |
| Resume thread with another active writer | Attempts normal resume; the active-writer error falls back to read-only thread viewing. | A successful read is **not** takeover. Read/resume failures retain the old view and show errors; input is blocked in the external-writer view. | [resume fallback](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/session_lifecycle.rs#L1225-L1280), [input guard](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget.rs#L1835-L1855) |
| `/fork [name]`; CLI `fork` | Branch saved conversation; optional checkout selection. Source session is preserved. | History branching alone is not filesystem isolation; choosing Current checkout retains the working directory. CLI selectors and flags are enumerated in the CLI section. | [checkout picker](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/worktree_picker.rs#L19-L78), [backtrack contract](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_backtrack.rs#L1-L22) |
| Edit an earlier prompt: Esc, Esc, selection, Enter | Empty-composer backtrack primes then opens transcript; confirmation forks **before** the selected turn and restores its prompt into the new composer. Ctrl+T also opens transcript including a live render-only tail. | Does not automatically resend the restored prompt; not file rollback. Rejects branching a steer independently or a still-running turn. Keeps images/text elements; canonical mention bindings are resolved against saved visible history. | [state machine](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_backtrack.rs#L1-L22), [selection](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_backtrack.rs#L397-L436), [rejections](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_backtrack.rs#L437-L500) |
| `/worktree`: Continue current / Start new / Browse | **Stable default-on `worktrees` feature**, additionally requires local Git and local operations. Continue forks history into an isolated managed checkout; browse supports resume owner or copy cwd. | Rejects explicitly untrusted source, remote environment, non-idle primary/queued input, duplicate creation, loading MCP inventory, active agents, unsaved nonempty history and background terminal blockers. Rechecks after allocation; retained checkout recovery is an explicit design contract. | [feature](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1256-L1261), [menu](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/worktree_picker.rs#L80-L155), [preflight](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/managed_worktree_creation.rs#L1-L150) |
| Worktree browse → Resume owner / Copy working directory / Delete worktree | Owner resume appears only for a resumable owner; invalid UTF-8 disables path copy. Delete has a second Cancel/Delete confirmation and preserves thread history. | Current checkout deletion is disabled; confirmation warns it may disrupt other sessions. Stale request checks fence actions. | [worktree leaves](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/worktree_picker.rs#L254-L394) |
| `/archive`; archive selected row | Archive a saved thread; picker Ctrl+A shortcut only in Resume/active status, not Fork/archived. Active current thread exits embedded TUI on success; remote target returns to overview. | Errors remain visible. Picker handles pending/result by thread identity; current side conversation cannot archive—return to main first. Archive is distinct from permanent delete. | [picker archive](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/resume_picker/archive.rs#L19-L112), [current archive](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/event_dispatch.rs#L3204-L3274) |
| `/delete` confirmation; CLI archive/unarchive/delete selectors | Delete presents a confirmation warning that it cannot be undone and includes subagent threads. CLI archive/unarchive resolve one ID/name; delete prompts unless explicitly forced. | CLI name lookup distinguishes active versus archived collections and errors for missing matches. Noninteractive delete confirmation refuses with guidance to use `--force` and a session UUID. Delete of current side thread is refused. This audit did not execute destructive commands. | [confirmation](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L199-L237), [CLI selectors](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/session_archive_commands.rs#L24-L158), [delete CLI](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/session_archive_commands.rs#L168-L235), [current delete](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/event_dispatch.rs#L3275-L3326) |
| `/review` → base branch | Searchable local branches, displays current → selected (detached fallback); sends `ReviewTarget::BaseBranch`. | A model review, not the `/diff` renderer and not automatic patch acceptance. Child acceptance dismisses parent; cancelling does not launch the target. | [review menu](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/review_popups.rs#L10-L97) |
| `/review` → uncommitted | Directly submits `ReviewTarget::UncommittedChanges`. | Can consume provider resources when executed; not run in this audit. | [target](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/review_popups.rs#L28-L35) |
| `/review` → commit | Search title/SHA over the 100 recent commits; chooses SHA and title. | A bounded local commit list, not every historical commit. | [commit picker](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/review_popups.rs#L99-L133) |
| `/review` → custom instructions; `/review <text>` | Text input popup or inline target; custom target sends exact instructions. | Request is a review turn, not a local linter. Starting review clears pending questions; review state preserves prior usage information. | [custom popup](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/review_popups.rs#L135-L154), [inline dispatch](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L995-L1005), [review state](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/review.rs#L1-L13) |
| `/diff` | Adds in-progress cell; runs Git via current workspace runner, asynchronously emits cwd-tagged result. Computes tracked **unstaged** `git diff` plus untracked `--no-index` diffs. | **Important scope limit:** command argv has neither `HEAD` nor `--cached`; do not advertise staged-only changes as covered. Nonrepo, missing runner and command errors have explicit messages. Per-command 30-second bounds, no aggregate deadline. | [dispatch](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/slash_dispatch.rs#L439-L463), [actual Git argv](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/get_git_diff.rs#L18-L120), [execution](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/get_git_diff.rs#L143-L161) |
| Safely inspect a diff | Disables executable diff helpers/textconv, sanitizes executable filter config, handles fsmonitor policy, suppresses hooks, ignores dirty submodule traversal. | These are source protections, not proof against every hostile repo/platform. Git diff status 1 is accepted as differences; unexpected exit status errors. | [Git safety implementation](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/get_git_diff.rs#L18-L28), [capture/filter handling](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/get_git_diff.rs#L143-L251) |
| Undo distinctions | **No `/undo` slash command in this pin's authoritative enum.** Prompt backtracking branches conversation; Vim editor undo affects composer editing. | Do not infer automatic workspace restoration from prompt edit, diff or review. Comprehensive hidden/internal rollback implementation is not audited; absence claim is limited to public slash inventory. | [complete slash enum](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/slash_command.rs#L12-L83), [prompt branch semantics](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_backtrack.rs#L1-L14), [editor action](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap_setup/actions.rs#L150-L165) |

### Session/review comparative lessons

1. **P0 — show the authority and filesystem boundary.** Helm should preserve its exclusive Voyage ownership and present read-only viewing separately from takeover; Codex's active-writer fallback is a concrete example, not evidence of process survival.
2. **P0 — name rollback precisely.** Conversation branch, composer undo, Git diff and file restoration are different effects. Acceptance for Helm: the pre-action UI names which state changes and which state does not.
3. **P1 — retain focus and draft across cancelled navigation.** Codex explicitly preserves per-thread input and redraws on picker exit. Exercise Helm switches with attachments, a queued prompt and failed resume rather than only a successful happy path.
4. **P1 — surface diff scope.** A label such as “working-tree diff” should say whether staged/untracked/binary/submodule changes are included. Codex's actual argv is more specific than generic “current changes”.
5. **P1 — preserve recovery handles for partial worktree creation.** Source-before-effects checks and post-allocation rechecks are useful; neither an async success banner nor a retained directory alone proves a session handoff succeeded.

---

<a id="runtime-interaction-source-audit"></a>

## Codex CLI runtime UX — source audit fragment for #271

## Scope and evidence contract

Source: supplied `openai-codex-36f0dbe` tree; pin **36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564**, upstream `openai/codex`. This fragment covers interactive CLI/TUI paths and their shared CLI backend handlers, not the desktop product, SDK, or a live session. All behavioral descriptions below mean **implemented source paths**, not observed execution. No builds, tests, provider calls, sandbox executions, downloads, or GitHub issue changes were made. Local `commit.json` identifies the requested pin; an independent remote verification of the archive's provenance was not performed. The source manifest below fingerprints the cited local files.

**Default** means the compiled default or explicit config fallback at this pin, not a guarantee for a managed installation. **Gated** means a feature, mode, policy, model, or account condition. **Platform** describes source conditionals, not native validation. **Absent** is used only for a narrow explicitly excluded path; **unverified** is not evidence of absence. Comparative lessons are design recommendations, not claims about Helm's current implementation.

## 1. Choose approval and sandbox behavior before work

1. **Select policy independently from containment.** CLI options expose `--sandbox` and [`--ask-for-approval`](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/cli.rs#L60-L67); `--approve-for-me` opts into automatic review with workspace-write, while the dangerous bypass disables both checks and sandboxing. Do not import older `--full-auto` descriptions into this pin. Do not equate “never ask” with “full access.” [CLI options](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/utils/cli/src/shared_options.rs#L38-L65). The protocol default approval is `OnRequest`; `Never` returns failures rather than escalating, and granular policy separates categories. At this pin `on-failure` is an alias of `OnRequest`, not evidence of a separate legacy retry mode. [Policy enum](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/protocol/src/protocol.rs#L985-L1025).
2. **Account for project trust and requirements.** Config selects on-request for trusted projects, internal untrusted policy for untrusted projects, otherwise the enum default; disallowed implicit defaults fall back to requirements. Explicit TOML untrusted policy is rejected in this path. [Config resolution](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/core/src/config/mod.rs#L3612-L3639). Default built-in permissions choose workspace for projects with an explicit trusted/untrusted classification, except Windows with sandbox disabled; otherwise read-only. This is not an unconditional workspace-write default. [Permission profile fallback](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/core/src/config/permissions.rs#L51-L61).
3. **Distinguish reviewer capability from reviewer selection.** `guardian_approval` is stable/default-on, but the config fallback reviewer is **User**, subject to requirements. Default-on Guardian support does not establish default automated approvals. [Feature](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1574-L1579), [reviewer resolution](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/core/src/config/mod.rs#L3640-L3653).
4. **Set up platform containment when required.** Windows prompt code checks requirements, distinguishes whether unelevated mode is allowed, and offers administrator-required default sandbox setup. This is platform-gated UI, not proof of successful provisioning or equivalent security across OSes. [Windows prompt](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/windows_sandbox_prompts.rs#L6-L75). The Linux bwrap feature entry and nearby Windows sandbox feature entries are marked removed/default-off, while legacy Landlock is deprecated/default-off; registry entries alone must not be mistaken for active setup controls. [Platform registry](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1186-L1214).

## 2. Approve, decline, cancel, or grant scope

1. **Read the pending operation in context.** Exec approval flushes the answer stream, emits a notification, and carries command, reason, request identity, available decisions, network context, and additional permissions into the bottom pane. Patch approval carries changed files and working directory. [Request handlers](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/tool_requests.rs#L283-L329).
2. **Choose only a server-offered command decision.** The menu is built from `available_decisions`, not a fixed universal set. It differentiates once, session, prefix-rule amendment, host policy amendment, “No, continue without running it,” and “No, and tell Codex what to do differently.” Session wording changes for host/permission requests; prefix amendments containing newlines are omitted. Thus decline-and-continue and cancel-for-redirection are distinct actions. [Exec menu handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L829-L914).
3. **Approve file changes at explicit scope.** Patch choices are proceed, accept-for-session (“these files”), or cancel-and-redirect. This menu is not evidence of a global permission grant. [Patch choices](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L1012-L1029). Explicit permission-tool grants are a separate surface: `request_permissions_tool` is under-development/default-off. [Feature gate](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1180-L1185).
4. **Treat a sandbox denial as a failure with policy-controlled escalation, not an automatic escape hatch.** The orchestrator distinguishes sandbox denial from other errors, preserves non-escalatable failures, and checks whether the tool wants no-sandbox approval. Its on-request network exception is conditional. The source explicitly documents no automatic unsandboxed retry under Never/OnRequest in the ordinary path. [Denial handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/core/src/tools/orchestrator.rs#L327-L388). Native enforcement, approval-cache persistence, and every escalation branch are unverified here.

## 3. Answer a structured question

1. **Enter an eligible mode.** Plan permits `request_user_input`; Default is added only with `default_mode_request_user_input` (under-development/default-off). The old `collaboration_modes` feature entry is removed/default-true, not a live opt-in switch for the basic mode distinction. [Mode predicate](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/protocol/src/config_types.rs#L696-L702), [tool availability](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tools/src/tool_config.rs#L17-L26), [gate](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1550-L1555), [removed entry](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L1652-L1657).
2. **Receive a tool question, not an approval.** Backend rejects non-root agents and unavailable modes, normalizes arguments, marks Plan questions blocking, and sets no auto-resolution deadline in this handler. Cancellation before response returns a model-facing error. Therefore general Default-mode questions and subagent-origin questions are absent from this specific default handler path, not absent from all possible conversational UI. [Backend handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/core/src/tools/handlers/request_user_input.rs#L69-L103). TUI flushes answer output, emits a question-count/summary notification, and opens the question surface. [TUI handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/tool_requests.rs#L449-L465).
3. **Select answers and add notes, then submit.** Submission converts selected options plus free text into answers keyed by question ID, sends the response, appends a result history cell, and advances the queue. [Answer submission](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L840-L901). The unanswered-confirmation handler supports navigation, Enter, and numbered selection; do not assume every Enter commits the entire form. [Confirmation handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L1113-L1153).
4. **Cancel with modal-aware semantics.** Ctrl-C and the interrupt key depend on notes focus, whether notes are empty, and unanswered confirmation; Esc can first act on the notes UI. This differs from the blanket claim “Esc always aborts.” [Question interrupt predicate](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/request_user_input/mod.rs#L1170-L1187). Timer behavior elsewhere in the question component is not established as a default for this tool.

## 4. Follow streaming answers, tools, and reasoning

1. **Watch incremental answer text.** Nonempty deltas initialize a width-aware stream controller with working-directory/render context. [Delta handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/streaming.rs#L526-L548). Lifecycle interruptions are queued while streaming, and once a queue exists further events remain queued to preserve begin/end ordering. Completing the stream flushes those events. This is a concrete implementation of ordered presentation, not a claim of measured smoothness or latency. [Ordering boundary](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/streaming.rs#L493-L524).
2. **Follow command output by call identity.** Command deltas update unified-exec tracking, then append to an active exec cell when a task is running; redraw is requested only if the cell changes. [Output delta](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/command_lifecycle.rs#L53-L71). Start and completion are distinct handlers; completion flushes the answer stream and queues or handles the end event. [Command lifecycle](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/command_lifecycle.rs#L19-L51), [completion](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/command_lifecycle.rs#L133-L162). Shell and unified exec/TTY are stable/default-on capabilities, but individual tool availability still depends on configuration and execution environment. [Feature defaults](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L922-L958).
3. **Read a reasoning summary without treating it as full internal reasoning.** Summary deltas feed a derived status header; finalization records summary parts in a history cell and retains the latest useful summary. [Reasoning handlers](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/streaming.rs#L274-L337). This establishes summary presentation, not provider-independent raw reasoning exposure. Actual supplied summaries and model support remain unverified. Default reasoning-effort shortcuts are Alt-comma/Alt-period with Shift-arrow alternatives; their existence is not a guarantee every model supports all efforts. [Default keys](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1570-L1578).

## 5. Steer now or queue the next turn

1. **Use Enter versus Tab intentionally.** Default bindings are Enter submit, Tab queue; queued-message editing uses Alt-Up or Shift-Left. These are configurable/context-sensitive bindings, not promises that Tab bypasses an open completion popup. [Key defaults](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1567-L1593).
2. **Submit while work is active.** The composer handler considers session readiness, plan streaming, suppression, recovery, and pending turn start. It submits immediately only when eligible; otherwise it queues. User-shell-only activity has its own queue condition. [Input routing](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/input_flow.rs#L18-L80). Backend routing attempts active-turn steering, including a pinned expected turn ID in the request. [Steer request](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app_server_session.rs#L1397-L1417).
3. **Handle explicit steering races rather than silently losing input.** If the server says the active turn is missing, routing switches to starting a turn. A changed active-turn ID is retried once with the reported ID; subsequent mismatch or other errors propagate. [Race handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/app/thread_routing.rs#L775-L821). `ActiveTurnNotSteerable` invokes rejected-steer queueing. [Rejected steer](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/turn_runtime.rs#L310-L318). These explicit rejections are not a license to replay transport-ambiguous effects.
4. **Drain deferred input when safe.** The queue drain refuses while unconfigured, policy-blocked, suppressed, recovering, recovered-from-disconnect, direct-input-blocked, or pending/running. Its stated unit is one input to start the next turn. [Drain guard](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/input_flow.rs#L199-L216). Keep “queued,” “pending steer,” and “accepted” distinct in UX: the local state separately models queue, pending steers, and rejected steers. [Input state](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/input_queue.rs#L14-L40).

## 6. Interrupt, diagnose failure, and recover

1. **Interrupt active work, or quit when idle.** Esc is the default interrupt binding. Ctrl-C first delegates to the bottom pane (which may clear/dismiss/handle a modal); with the double-press shortcut disabled it requests an interrupt for cancellable work, otherwise requests quit. [Default Esc](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/keymap.rs#L1567-L1571), [Ctrl-C handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/interaction.rs#L553-L590), [disabled double-press constant](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/bottom_pane/mod.rs#L201-L205). This is request-side evidence only: process termination and cleanup completion were not measured.
2. **Get actionable interruption text.** Ordinary interruption asks the user to tell the model what to do differently and points to `/feedback`; budget-limited stop instead says the goal budget was reached. [Interruption message](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/turn_runtime.rs#L553-L558).
3. **Separate retrying from terminal failure.** `will_retry` errors update stream retry status; non-retry errors are stored and routed to failure handling. [Notification dispatch](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/protocol.rs#L178-L207), [retry status](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/streaming.rs#L353-L363). Server overload clears pending steers, restores queued input to the composer, appends an error, and finalizes. Generic errors may restore the rejected initial prompt and then consider queued input. Do not generalize “every error stops all queued work.” [Failure handlers](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/turn_runtime.rs#L363-L399).
4. **Recover account-limited work without premature queue drain.** Rate-limit recovery sets a pending guard for ChatGPT accounts, distinguishes owner/member credit or usage limits, shows appropriate error/nudge, and requests a recovery rate-limit refresh. This is account-specific, not a generic API-key billing workflow. [Rate-limit handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/turn_runtime.rs#L420-L472). Retry schedules, provider reconnection guarantees, and successful remediation are unverified.
5. **Disconnect without automatically replaying uncertain submissions.** Pause disables question delivery and queue autosend, preserves editable initial input, replaces the interrupt hint with quit, and displays reconnecting status. Restore explicitly preserves a prompt whose acceptance is unknown for manual recovery rather than comparing partial history and resubmitting it. [Disconnect and restore](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/chatwidget/reconnect.rs#L1-L52). This narrowly establishes an intentional absence of automatic replay in this recovery path, not a global exactly-once guarantee.
6. **Resume saved context deliberately.** CLI resume setup distinguishes picker, last, and explicit session ID; fork is a separate command. [Resume argument handler](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L2870-L2895), [fork command](https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L222-L223). These establish saved-session entry points, not survival of a dead process or rollback of external effects. Persistence durability, crash recovery, and native OS recovery journeys were not tested.

## Comparative lessons for Helm/Voyage design

- **Separate policy, reviewer, and enforcement in labels.** Guardian capability being default-on while User remains the fallback reviewer is a useful warning against deriving UX promises from one feature bit (§1).
- **Keep scope and negative decisions explicit.** Once/session/prefix/host choices and decline-versus-cancel communicate materially different consequences. Do not collapse these into one “Yes/No” surface (§2).
- **Let questions remain questions.** Blocking mode, free text, unanswered confirmation, and modal-aware interrupts need a separate contract from security authorization (§3).
- **Preserve lifecycle ordering during streaming.** Queueing tool boundaries behind text avoids misleading end-before-start presentation. Summary status can stay useful without promising raw reasoning (§4).
- **Name input states and expose recovery.** Enter/Tab is learnable only if queued versus pending/accepted steering is legible. Explicit turn-ID races and unknown delivery need different handling (§5–6).
- **Do not turn a reassuring UI into a cleanup claim.** An interrupt request, retry banner, restored prompt, or resume picker is not evidence that a tool stopped, a request was never accepted, or external work was undone (§6).

## Verification and limits

Static inspection only (`rg`, numbered source excerpts, local file/hash checks). Pinned link targets and line bounds were checked against the local tree, not fetched over HTTP. No runtime assertions or test-pass claims are made. Source branches for Linux/Windows are evidence of implementation intent, not native sandbox assurance. Broader MCP elicitation, raw reasoning, all permission caching/escalation cases, crash durability, and platform parity remain **unverified**, not “missing.” Only this fragment was written; pre-existing tracked changes and deleted scripts were left untouched. Issue #271 is context supplied by the requester, not an issue audited or updated here.

## Source manifest

The following manifest covers every file cited above. Paths are relative to the supplied source root; SHA-256 hashes fingerprint local bytes, not remote attestation. All blob links use the same full commit pin.

| Source file | SHA-256 |
|---|---|
| `codex-rs/cli/src/main.rs` | `5cf524d836322dc6130cc7b3718271233db28bdbcfb0227decdfbb1b87c9888a` |
| `codex-rs/core/src/config/mod.rs` | `405fd7cac4aefbbf538e63bc6c3b2390f3c6e3c01c8e6d4a535b797a7afe3a91` |
| `codex-rs/core/src/config/permissions.rs` | `96ff0f5de4d2c8918b7b6657cae26dba73cb343f7953dcf4623117e885b67f93` |
| `codex-rs/core/src/tools/handlers/request_user_input.rs` | `f8cf561ba3e6c6527bdc6b7c66330b4a3e71ce978bde52077f4b4e5efbf3a016` |
| `codex-rs/core/src/tools/orchestrator.rs` | `824a1b318a718f69b5e0f3f0e3c7bc46bf66742c39f1a514e1ddb2cb2b64334f` |
| `codex-rs/features/src/lib.rs` | `a21065af423dab633499802df24e7b4fd7330bc47ff7e43180eef7f70939a350` |
| `codex-rs/protocol/src/config_types.rs` | `079f3650c59c002a5ea64feb01709c7aaf9cf904bce54aae42ae971fc068275e` |
| `codex-rs/protocol/src/protocol.rs` | `141ce4afc82e1f9dfb299f89500bf4ed4e1bd6909702824194b09c56f14343b1` |
| `codex-rs/tools/src/tool_config.rs` | `1cb8ecfd0d0721bb9b30e82492128e7fd1f2aa85faa12a607e01cfdd144c3d95` |
| `codex-rs/tui/src/app/thread_routing.rs` | `91c68ff1a32a522a0b0cff65966f58b2b1c7f9af8fb4d4ee52cf060c85aa980a` |
| `codex-rs/tui/src/app_server_session.rs` | `f3f5159c4ef84826efbe86a5bcc58e0c7e9ef7a5758a86cb787ae099b85ee8b1` |
| `codex-rs/tui/src/bottom_pane/approval_overlay.rs` | `d54cf42a84d523db57d9a0ad2bee4e361e31cec4e714b8de755d2085ae45e125` |
| `codex-rs/tui/src/bottom_pane/mod.rs` | `598b2c6e293c557dfb1e20ef2ceb82ac5b94db81a51fc8c4f25325db54ec0c1c` |
| `codex-rs/tui/src/bottom_pane/request_user_input/mod.rs` | `00e983e84972c9d84c7995631f73bef18217c7bfb3b07438c6fd9c16631b192e` |
| `codex-rs/tui/src/chatwidget/command_lifecycle.rs` | `5bca6a87d6fa2c31ee6d98483e4ec0a396e73a715bd4ec11f65918218ead6f84` |
| `codex-rs/tui/src/chatwidget/input_flow.rs` | `418647bfc20b6276b11fe08fe27f570af6af51382ba9f051cf142bff7d635bbb` |
| `codex-rs/tui/src/chatwidget/input_queue.rs` | `7b9f28ddfe99745206f03a73cd7589bc39b474bd5ec7f981d48ba6a879d06ce1` |
| `codex-rs/tui/src/chatwidget/interaction.rs` | `c07d59a9dd80a55c6680d91dcadafebcde22f64e7d2f8f1735879f53e7c36fc4` |
| `codex-rs/tui/src/chatwidget/protocol.rs` | `d80e013058f3ab8811f1030e698d13de106de29886c8dfded9b4838a38436f0e` |
| `codex-rs/tui/src/chatwidget/reconnect.rs` | `f57114dc8645f4e8559074858dd2edf58d451dad0506218f89be65d621deeaca` |
| `codex-rs/tui/src/chatwidget/streaming.rs` | `8e206e4eaf7dc7ad00f0acf102bf3a69bbb04db16a894f28575ec673086ba052` |
| `codex-rs/tui/src/chatwidget/tool_requests.rs` | `b40527c4014b8f7d35f7a6dd08339f089b66da01533c79de3229a86d590db129` |
| `codex-rs/tui/src/chatwidget/turn_runtime.rs` | `10b4cd49eeb7d6d8e4a3a6cd2434cff4d9c084f2bc29fab25f9839757521fe1d` |
| `codex-rs/tui/src/chatwidget/windows_sandbox_prompts.rs` | `0b52cd33df3e7507572b832a120c60f821a113c047b3872959a32ba897ecca47` |
| `codex-rs/tui/src/cli.rs` | `742c86c920c0df679c46542918c9661cff36d68d8fc78c5398263872e2c454f6` |
| `codex-rs/tui/src/keymap.rs` | `67fb23246b89ff6a2a33c5c0b77676172bf8cb88805986b7ca934d69673a8ec7` |
| `codex-rs/utils/cli/src/shared_options.rs` | `6eefb5fcf0545fca54ffa1f80a6d3630c8d9b61ec449466d5834ac9c489f76d8` |

---

<a id="cli-config-source-audit"></a>
## CLI/config source audit

**Issue context:** #271. **Pinned upstream:** `openai/codex` at `36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564`. Source: supplied extracted tree `target/ux-audit/codex/openai-codex-36f0dbe`; supplied `commit.json` identifies that SHA. Static source audit only: no Codex execution, provider/auth/network calls, downloads, or platform tests. Links below pin exact blobs, not current documentation. No tracked files or issues changed.

**Boundary:** exhaustive *`codex` terminal parser* command tree, including hidden leaves and CLI entry points into app/daemon services. Desktop windows, app-server RPC catalog, slash-command enum, editor mechanics, and detailed session/review lifecycle are outside this fragment. A CLI wrapper listed here is not an audit of the wrapped desktop surface. `[H]` means hidden from ordinary help; `[E]` means explicitly experimental in command help; `[P]` means platform-specific. Neither visibility nor a stable command name proves a feature is enabled. Rust `clap(skip)` fields are internal state, not flags. Standard Clap help (`-h/--help`, generated `help` traversal) and version (`-V/--version` where declared) are implicit rather than repeated per row. [root][tui][exec]

## Flag bundles and invocation rules

All leaf paths below are prefixed by `codex`. A dash denotes no additional declared leaf flags. `[]` means optional, `…` repeatable. Aliases are named explicitly, not separate capabilities.

| Bundle | Complete declared surface and default/constraint |
|---|---|
| **C: overrides** | `-c/--config key=value` repeatable; dotted keys; parse value as TOML, falling back to a string if TOML parsing fails. Missing `=` or empty key fails. Global in its Clap definition. [override] |
| **S: shared session flags** | `-i/--image FILE…` (comma-delimited); `-m/--model MODEL`; `--oss`; `--local-provider PROVIDER` (documented lmstudio/ollama; config default or interactive selection when omitted with OSS); `-p/--profile NAME` layers `$CODEX_HOME/<name>.config.toml`; `-s/--sandbox {read-only,workspace-write,danger-full-access}`; `--approve-for-me` (alias `--not-so-yolo`); `--dangerously-bypass-approvals-and-sandbox` (alias `--yolo`); `--dangerously-bypass-hook-trust`; `-C/--cd DIR`; `--worktree`; `--add-dir DIR` repeatable. Booleans default false, others unset. Auto-review conflicts with sandbox/bypass and writes `approvals_reviewer=auto_review`, `approval_policy=on-request`, `sandbox_mode=workspace-write`; hook trust bypass is separate from sandbox bypass. [shared][sandbox-values] |
| **I: interactive** | S + `[PROMPT]`, `--strict-config` false, `-a/--ask-for-approval {on-request,never}`, `--search`, `--no-alt-screen`; C is supplied by root. Never returns execution failures directly rather than prompting; this CLI enum does **not** offer older `untrusted`/`on-failure` values. `--search` enables live search. No-alt-screen preserves scrollback/inline rendering. [tui][approval] |
| **R: remote interactive endpoint** | `--remote ENDPOINT` (`ws://host:port`, `wss://host:port`, `unix://`, or `unix://PATH`); `--remote-auth-token-env ENV_VAR`. Runtime rejects token-env without remote or except `wss://`/loopback `ws://`; reads the named variable, not an inline token. [toggles][remote-token] |
| **F: feature overrides** | `--enable FEATURE` / `--disable FEATURE`, repeatable and global, translated into `features.KEY=true/false`. Unknown keys fail; enables are emitted before disables. These are run overrides, unlike persistent `features enable/disable`. [toggles] |
| **X: exec flags** | S + `--strict-config`, `--thread-source SOURCE`, `--skip-git-repo-check`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--output-schema FILE`, `--color {always,never,auto}` (auto), `--json` (alias `--experimental-json`), `-o/--output-last-message FILE`, `[PROMPT]`. Absent prompt or `-` reads stdin. Ignore-user-config still uses CODEX_HOME for auth. Explicit globals: strict/source/git-check/ephemeral/ignore flags/schema/color/json/output; S model, dangerous bypass, hook-trust bypass, worktree are made global. Do not assume all S flags are legal after a nested exec verb. [exec] |

Root with no subcommand opens interactive CLI with I+C+R+F; root interactive options are not all Clap-global. Session wrappers flatten the bundles shown; root-to-exec inheritance is explicitly implemented, not a universal option inheritance promise. [root][shared][exec]

## Terminal command tree: every declared leaf

| Action/path | Arguments / flags | Effect, gate, failure/recovery boundary |
|---|---|---|
| `agents` | I, R | Browse daemon-wide agent sessions, not a separate `list` subcommand. [root][mainargs] |
| `exec` (`e`) | X, C | New noninteractive run; Git check opt-out and machine-output flags explicit. [exec] |
| `exec resume` | `[SESSION_ID] [PROMPT]`, `--last`, `--all`, `-i/--image FILE` (comma-delimited, one CLI value per occurrence), exec-global flags | UUID/name; with last, sole positional is reinterpreted as prompt. [exec] |
| `exec fork` | `SESSION_ID [PROMPT]`, `-i/--image FILE`, exec-global flags | Requires source UUID/name; prompt `-` reads stdin. [exec] |
| `exec review` | review selectors below, exec-global flags | Noninteractive review, not an editor UI. [exec] |
| `review` | S, C, `--strict-config`, `[PROMPT]`, `--uncommitted`, `--base BRANCH`, `--commit SHA`, `--title TITLE` | Uncommitted/base/commit/custom prompt mutually exclusive; title requires commit. Same selectors on exec review. [mainargs][exec] |
| `resume` | `[SESSION_ID]`, `--last`, `--all`, `--include-non-interactive`, I, R | Picker by default; last bypasses picker; all removes cwd filtering. SessionTuiCli prompt conflicts with last, allowing last plus one prompt-like positional rather than ID+prompt. [mainargs] |
| `fork` | `[SESSION_ID]`, `--last`, `--all`, I, R | Picker/new fork entry point; lifecycle covered elsewhere. [mainargs] |
| `archive`, `unarchive` | exactly one `SESSION` UUID/exact name, S, C, R, `--strict-config` | Single-target lifecycle leaves, **no** `--all`, `--yes`, or multi-target vector. [mainargs] |
| `delete` | same single target/options; `--force` | Force skips prompt and requires UUID. No bulk delete or `--yes`. [mainargs] |
| `queue` | required `--thread THREAD`, `--message TEXT`; S, C, R, `--strict-config` | Queue nonempty text to session UUID/name; images explicitly rejected. [queue] |
| `apply` (`a`) | `TASK_ID` | Apply latest agent-produced diff locally via git apply; not a general patch-file positional. [root][mainargs] |
| `login` | `--with-api-key`, `--with-access-token`, `--device-auth`; [H] `--api-key [API_KEY]`, `--experimental_issuer URL`, `--experimental_client-id CLIENT_ID` | Browser sign-in by default; secret inputs from stdin. Deprecated direct key spelling exits with guidance, not an alternative valid login path. [mainargs][login] |
| `login status` | — (parent login options are parser parent options, not new status flags) | Reports auth status; absence/error exit 1; success 0. [login-status] |
| `logout` | — | Removes stored credentials; does not mean remote session cancellation. [root][login-status] |
| `mcp list`, `mcp get NAME` | C; each `--json` | Effective server inspection (see detailed MCP section). [mcp] |
| `mcp add NAME` | C; either `-- COMMAND…` plus `--env KEY=VALUE` repeatable, **or** `--url URL` plus `--bearer-token-env-var ENV_VAR`, `--oauth-client-id CLIENT_ID`, `--oauth-client-registration {auto,cimd,dcr}`, `--oauth-resource RESOURCE` | Required exclusive transport group; env is stdio-only; OAuth/bearer options HTTP-only. [mcp] |
| `mcp remove NAME` | C | Remove global launcher entry, not merely log out. [mcp][mcp-action] |
| `mcp login NAME` | C, `--scopes SCOPE,SCOPE`, `--oauth-client-registration {auto,cimd,dcr}` | Per-login registration strategy, not a saved default. [mcp] |
| `mcp logout NAME` | C | Removes OAuth credentials for named server. [mcp] |
| `plugin add PLUGIN` | C, `--marketplace MARKETPLACE`, `--json` | NAME@MARKETPLACE or bare NAME with marketplace; missing name/marketplace fails. [plugin] |
| `plugin list` | C, `--json` | Installed plus available marketplaces/plugins; auth/policy can constrain view. [plugin] |
| `plugin remove PLUGIN` | C, `--json` | Remove by NAME@MARKETPLACE. [plugin] |
| `plugin marketplace add SOURCE` | C, `--ref REF`, `--sparse PATH` repeatable, `--json` | Local path or owner/repo[@ref], HTTPS/SSH Git. This can fetch files; audit did not run it. [market] |
| `plugin marketplace list` | C, `--json` | Configured sources/roots. [market] |
| `plugin marketplace upgrade [MARKETPLACE_NAME]` | C, `--json` | Omission upgrades all Git marketplaces, not all plugins. [market] |
| `plugin marketplace remove MARKETPLACE_NAME` | C, `--json` | Remove configured source. [market] |
| `features list` | C, F at root | Stages and effective states, not just enabled names. [toggles] |
| `features enable FEATURE`, `features disable FEATURE` | C | Persistent config mutation; known feature key required. [toggles] |
| `completion [SHELL]` | default bash | Generate shell completion script; shell enum comes from clap_complete, not slash-command enum. [mainargs] |
| `doctor` | `--json`, `--summary`, `--all`, `--no-color`, `--ascii`; [H] `--feedback` | Full redacted diagnostics by default; summary only contracts display; feedback limits DB integrity scans. [doctor] |
| `update` | — | Stable package-manager update; debug builds refuse; unsupported install route errors with manual update guidance; successful update asks restart. [update] |
| `migrate-rollouts` | `--apply`, `--thread THREAD_ID` repeatable, `--max-mib-per-second MIB` (>=1), `--json`, `--verbose` | Default is inspection/dry run; apply publishes migration. [migrate] |
| `debug models` | `--bundled` | Raw catalog JSON; bundled avoids active catalog refresh. Default can use online-if-uncached, so **not** inherently offline. [mainargs][models-debug] |
| `debug prompt-input` | required `--output PATH`, I | Write model-visible prompt-input JSON, not editor/slash inventory. [mainargs] |
| `debug trace-reduce` [H] | `--input PATH`, `--output PATH` (required) | Replay trace bundle into reduced JSON. [mainargs] |
| `debug clear-memories` | — | Reset memory state and on-disk artifacts; destructive diagnostic action, not a status query. [mainargs] |
| `debug app-server send-message-v2` | `USER_MESSAGE` | Debug app-server client invokes message operation; listed CLI leaf only. [mainargs] |
| `execpolicy check` [H parent] | `-r/--rules PATH` required/repeatable, `--pretty`, `--resolve-host-executables`, `COMMAND…` required/trailing | Policy evaluation emits JSON with matched rules and strongest decision; no heuristic fallback. Read/parse errors carry path. Not command execution. [execpolicy][mainargs] |
| `cloud` (`cloud-tasks`) [E] | no extra root flags | Cloud task browser when no child. **Provider/network capable**, not run here. [root][cloud] |
| `cloud exec` | `[QUERY]`, required `--env ENV_ID`, `--attempts N` (1, range 1–4), `--branch BRANCH` | Submit task; branch defaults current branch. Invalid count rejected by parser. [cloud] |
| `cloud status TASK_ID` | — | Task status. [cloud] |
| `cloud list` | `--limit N` (20, range 1–20), `--env ENV_ID`, `--cursor CURSOR`, `--json` | Pagination explicit. [cloud] |
| `cloud apply TASK_ID`, `cloud diff TASK_ID` | each `--attempt INDEX` (1-based, parser range 1–4) | Apply vs show unified diff. [cloud] |

## Platform sandbox CLI leaves

`sandbox` (alias `sb`) itself runs trailing `COMMAND…` in the host backend; explicit children are `macos` (alias `seatbelt`), `linux` (alias `landlock`), `windows`. Parent host options resolve at compile time to the corresponding struct; this is not evidence that all backends work on every host. Other platforms have only profile and command in their unsupported-host shape. [root][mainargs]

* **All three backends:** `--sandbox-state-json JSON`; repeatable `--sandbox-state-readable-root PATH` and `--sandbox-state-disable-network` require state JSON. State JSON conflicts with named permissions profile, cwd and managed-config inclusion. `-P/--permission-profile NAME` (alias `--permissions-profile`), `-p/--profile NAME`, `-C/--cd DIR`, `--include-managed-config`, trailing `COMMAND…`. Cwd and managed-config inclusion require permission-profile. C overrides are routed internally. [sandbox]
* **macOS additionally:** `--allow-unix-socket PATH` repeatable, conflicts with state JSON; `--log-denials`. [sandbox]
* **Linux additionally:** `--use-legacy-landlock`. Name `linux` is documented as Landlock+seccomp; flag is explicit legacy selection. [sandbox]
* **Windows:** no extra leaf flags beyond shared sandbox shape. [sandbox]

Do not confuse JSON sandbox-state replay, configuration profile (`-p`), and permissions profile (`-P`), or recommend bypass as recovery from a policy denial. Native enforcement verification is outside this source audit.

## Service/desktop boundary: CLI entry points, not desktop feature inventory

| CLI path | Complete local flags/arguments | Scope/gates |
|---|---|---|
| `app` [P] | `[PATH]` (.), `--download-url URL` | Compiled only macOS/Windows; open/install desktop app. No desktop UI audit implied. [root][app] |
| `mcp-server` | no local fields; root config overrides | Run Codex as stdio MCP server; opposite direction from `mcp add`. [root] |
| `app-server` [E] | `--code-mode-host URL`, `--strict-config`, `--listen URL` (stdio://; supports unix://, unix://PATH, ws://IP:PORT, off), `--stdio` conflicts listen; [H] `--remote-control`, `--managed-daemon`; `--analytics-default-enabled`; WS auth flags below | Service process CLI, not RPC audit. Analytics default off unless flag/config opts in; config can still opt out. Code-mode host defaults local, supplied URL selects remote gRPC. [mainargs][code-host] |
| app-server WS auth bundle | `--session-id ID` (env `CODEX_APP_SERVER_SESSION_ID`), `--auth-ws-shared-secret-env ENV_VAR`; `--auth-ws-external-validator-url URL`, `--auth-ws-allowed-validator-host HOST` repeatable, `--auth-ws-user-key-cache-ttl-seconds SECONDS` (300) | Shared-secret/external-validator strategies conflict; auth requires session ID; validator whitelist and TTL require external-validator. CLI syntax is not proof of safe external exposure. [ws-auth] |
| `app-server generate-ts` | required `-o/--out DIR`, `-p/--prettier PRETTIER_BIN`, `--experimental` | Experimental methods/fields omitted by default. [mainargs] |
| `app-server generate-json-schema` | required `-o/--out DIR`, `--experimental` | Same experimental-output opt-in. [mainargs] |
| `app-server generate-internal-json-schema` [H] | required `-o/--out DIR` | Internal schema artifact generator. [mainargs] |
| `app-server daemon run` [H parent] | `--listen URL` (unix://), `--rotation-interval SECONDS` (86400), `--restart-on-version-change` | Background service manager entry point. [mainargs] |
| `app-server daemon start`, `status`, `restart`, `update`, `enable-remote-control`, `disable-remote-control`, `stop`, `version` [H parent] | no leaf fields | Distinct service lifecycle leaves, not terminal session lifecycle. [mainargs] |
| `app-server daemon pid-update-loop` [H] | — | Internal managed-daemon monitor. [mainargs] |
| `remote-control` (`rc`) [H] | `--server-name NAME`, `--keep-awake` | Headless remote-control app server; macOS-only sleep inhibitor; standalone path expects suitable ChatGPT auth. [root][remote] |
| `exec-server` [E] | `--strict-config`; `--concurrent-requests COUNT` (1); `--listen URL`; `--remote URL`; `--remote-transport {noise,direct}` (noise); `--environment-id ID`; `--name NAME`; `--use-agent-identity-auth`; `--aws-sigv4`; `--aws-profile PROFILE`; `--aws-region REGION`; `--aws-service SERVICE` (execute-api); `--exit-on-stdin-close` (also environment-backed) | listen conflicts remote; remote requires environment-id; direct requires SigV4 and vice versa; identity auth conflicts SigV4 and requires remote. AWS fields require SigV4; region falls back SDK chain. Remote options/strict are global to children; listen/count are not. [mainargs] |
| `exec-server forward` | required `--exec-server-url URL`, inherited exec-server globals | Requires remote; direct transport explicitly rejects forwarding. [mainargs] |
| `responses-api-proxy` [H] | `--port PORT` (ephemeral), `--server-info FILE`, `--http-shutdown`, `--upstream-url URL` (https://api.openai.com/v1/responses), `--dump-dir DIR` | Requests/response dump is sensitive; optional shutdown endpoint. No proxy launched in audit. [root][proxy] |
| `stdio-to-uds SOCKET_PATH` [H] | one socket path, resolved relative to cwd | Service transport shim, not general terminal command execution. [mainargs] |

## Action-by-action model, auth and configuration behavior

1. **Select a model/provider:** use S `--model`, `--oss`, `--local-provider` or config override. Model manager accepts an explicitly selected model without replacing it with a default; absent model chooses catalog default then first available. Model list refresh errors are logged and cached models remain available. Raw catalog (`debug models`) is distinct from user-visible/auth-filtered presets; `--bundled` reads embedded catalog. Do not hard-code a current model name as an invariant of this pin. [shared][models][models-refresh][models-debug]
2. **Inspect available configuration:** config is TOML, not a `codex config get/set` tree (no such root variant). Loader documents ascending precedence: package defaults → managed preferences → system config → enterprise cloud fragments → `$CODEX_HOME/config.toml` → selected `<name>.config.toml` → cwd config → ancestor `.codex/config.toml` → repo config → runtime overrides. Project layers can be loaded but disabled for untrusted directories. Admin *requirements* are a separate composed constraint surface (system requirements, cloud, Unix legacy managed_config, managed preferences), not a low-priority preference that CLI simply defeats. Windows system paths use `%ProgramData%\OpenAI\Codex`, Unix `/etc/codex`. [root][config]
3. **Override safely:** `-c` is ephemeral and TOML-aware; quote strings deliberately because invalid TOML falls back to string rather than necessarily rejecting. `--strict-config` changes unknown-field handling from permissive loading to errors; diagnostics derive unknown paths/source information. Fix spelling/version mismatches instead of silently assuming an option applied. File profile (`-p`) is not sandbox permission profile (`-P`). [override][strict][shared][sandbox]
4. **Turn features on/off:** F validates known keys then makes runtime overrides; persistent feature verbs edit config. Stage is separate from default. At this pin `plugins` is stable/default true; multi_agent is stable/true but multi_agent_v2 stable/false; MCP Apps (`enable_mcp_apps`), newer MCP protocol/refresh coordination gates are under-development/default false. Skill-related experimentation has its own feature entries, not a top-level `skills` CLI command. Feature registry, not old help prose, is source of defaults; platform-dependent defaults exist. [toggles][feature-spec][root]
5. **Sign in:** browser login enforces allowed ChatGPT login method, writes a login log, opens browser and waits; browser-server `AddrInUse`/`PermissionDenied` takes the device-code fallback path. `--device-auth` explicitly selects device flow. API-key and access-token modes read nonterminal stdin, trim it, reject empty input and exit 1 on read/auth failure. Forced auth methods can refuse the selected mode. Never advise putting a secret in deprecated `--api-key`, a URL, transcript, or config override. [login]
6. **Inspect/sign out/recover auth:** `login status` distinguishes active auth/missing/error with exit status; logout reports removal/error. Direct login logging warns rather than failing login if log setup fails; Unix creates login log with 0600 mode. Credentials and MCP OAuth are distinct: Codex logout is not `mcp logout NAME`. Runtime secrets/keyring behavior is not platform-verified by this static audit. [login][login-status][mcp]
7. **Recover CLI installation:** JS launcher maps Linux/Android, Darwin and Windows x64/arm64 to native packages, errors for unsupported combinations, and supports packaged-vendor lookup fallback. `update` is install-method dependent and disallowed in debug builds; update success still requires restart. This is source-supported packaging behavior, not a successful native installation test. [launcher][update]

## MCP and plugin actions: configuration is not connectivity

* **List/get first:** CLI `mcp list/get` loads effective config, while add/remove write global MCP configuration. This distinction matters for project/cloud requirements and selected profiles; a persisted entry is not proof that policy enables it. No-server output points to add examples. [mcp][mcp-action]
* **Add stdio:** NAME validation rejects empty/invalid characters; env strings need `KEY=VALUE` and nonempty key. Command is a trailing vector. **Add HTTP:** URL and optional environment-variable bearer token or OAuth metadata; exclusive transport prevents mixed launch/URL forms. New entries are enabled=true, required=false, parallel-tool support=false and leave timeouts/tool filters unset. OAuth client/resource configuration is not the credential itself. [mcp][mcp-action]
* **Persist, then authenticate:** add writes global config and prints success **before** automatic OAuth login. OAuth discovery/start can fail after a successful write, so inspect/get or run `mcp login NAME` rather than blindly repeating an uncertain add. Missing auth falls back to instructions when appropriate; explicit registration strategy is immediate-login-only. Credential removal and launcher removal have different effects. [mcp-action]
* **Policy/display:** disabled-reason types distinguish unknown from requirements with source; tool-level config supports approval mode and output token budgets, with stricter budget restriction choosing minimum. Do not portray an MCP tool appearing in configuration as approved to run. [mcp-types]
* **Skills/plugins:** plugin add/list/remove and marketplace verbs are real terminal leaves. Marketplace upgrade is snapshot refresh; install/auth policy checks exist in plugin handling, and bare names require marketplace selection. Plugin functionality is stable/default-on at this pin, but catalog availability can depend on account/config/policy. No `plugin update`, `plugin enable`, `skills install`, or `models list` root CLI leaves exist in this parser. [plugin][market][feature-spec][root]

## Skills without auditing slash/editor surfaces

1. **Discover:** no standalone `codex skills` root verb. Host root discovery considers project config-layer skills folders, deprecated user `$CODEX_HOME/skills`, user `$HOME/.agents/skills`, bundled/system cache, `/etc/codex/skills` on Unix, plus repo ancestor `.agents/skills` from working directory toward project root. Root enumeration walks all layers high-to-low; do not infer that project-config trust disabling itself suppresses skill discovery. Root discovery uses executor filesystem abstractions, so a local-path-only mental model is incomplete. [root][skill-roots]
2. **Bound loading:** discover `SKILL.md`; constants bound scan depth to 6, skill directories per root to 2000, concurrent root scans to 8, name to 64 and description to 1024. These are code limits, not claims every malformed skill yields the same user message. [skill-limits]
3. **Enable/disable and budget:** skills config has rules by optional path/name plus enabled state; bundled skills setting, include_instructions and nonzero max_context_tokens (documented default 2% of model context; explicit value capped at 10,000). Rules preserve config-layer context; inspect effective configuration before blaming missing files. [skills-config]
4. **Merge/recover:** merge collects skill errors and aggregates truncation rather than pretending a truncated scan is complete; duplicate file identities are skipped and surviving skills sorted. Missing/permission-failed root probes are handled differently (missing ignored, other errors warned). Repair file/root/config and reload; do not claim a skill is executable merely because a directory exists. [skill-merge][skill-roots]
5. **Separate gates:** skills loading/configuration, plugin installation, and MCP dependency-install/skill-search feature flags are distinct surfaces. This fragment deliberately excludes slash enum, mention picker, editor keybindings and detailed invocation UX assigned elsewhere. [skills-config][feature-spec]

## Lessons for Helm (inferences, not upstream or Helm implementation claims)

* Keep the parser inventory alongside action semantics: a hidden daemon command is not a supported desktop UX, and a stable feature may still default off. [root][feature-spec]
* Show effective config **and provenance/disabled reason**; expose requirements separately from preferences so CLI overrides never appear to broaden policy. [config][mcp-types]
* Separate read/plan/apply actions and report partial success: migration defaults to dry-run; MCP configuration persistence can precede failed OAuth. [migrate][mcp-action]
* Keep secret input out of argv; report named environment-variable requirements and bounded auth failures without leaking values. [login][remote-token]
* Keep one-target lifecycle syntax explicit; never invent bulk flags from picker conveniences. [mainargs]
* Distinguish cached catalogs, installed assets, configured tools and authenticated/reachable tools. None alone proves runtime readiness. [models][mcp-action][plugin]

## Source manifest and verification

Manifest below lists every cited file/range. All references use the same full commit SHA. Audit verified local path existence and line-anchor bounds, and reviewed parser definitions plus selected dispatch/failure paths; did not generate live help or execute any command described above. Scope is the `codex` multitool parser, not every independently built binary in the Cargo workspace. No runtime/provider/native-platform success is claimed.

- **root:** [`codex-rs/cli/src/main.rs` lines 117–245][root]
- **mainargs:** [`codex-rs/cli/src/main.rs` lines 245–878][mainargs]
- **toggles:** [`codex-rs/cli/src/main.rs` lines 1046–1128][toggles]
- **shared:** [`codex-rs/utils/cli/src/shared_options.rs` lines 8–105][shared]
- **tui:** [`codex-rs/tui/src/cli.rs` lines 9–143][tui]
- **exec:** [`codex-rs/exec/src/cli.rs` lines 10–318][exec]
- **override:** [`codex-rs/utils/cli/src/config_override.rs` lines 1–110][override]
- **approval:** [`codex-rs/utils/cli/src/approval_mode_cli_arg.rs` lines 1–24][approval]
- **sandbox-values:** [`codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs` lines 1–34][sandbox-values]
- **sandbox:** [`codex-rs/cli/src/lib.rs` lines 26–190][sandbox]
- **mcp:** [`codex-rs/cli/src/mcp_cmd.rs` lines 48–222][mcp]
- **mcp-action:** [`codex-rs/cli/src/mcp_cmd.rs` lines 223–550][mcp-action]
- **plugin:** [`codex-rs/cli/src/plugin_cmd.rs` lines 50–220][plugin]
- **market:** [`codex-rs/cli/src/marketplace_cmd.rs` lines 34–160][market]
- **queue:** [`codex-rs/cli/src/queue_cmd.rs` lines 13–63][queue]
- **migrate:** [`codex-rs/cli/src/migrate_rollouts.rs` lines 21–125][migrate]
- **doctor:** [`codex-rs/cli/src/doctor.rs` lines 154–182][doctor]
- **remote:** [`codex-rs/cli/src/remote_control_cmd.rs` lines 28–130][remote]
- **cloud:** [`codex-rs/cloud-tasks/src/cli.rs` lines 1–120][cloud]
- **proxy:** [`codex-rs/responses-api-proxy/src/lib.rs` lines 31–60][proxy]
- **execpolicy:** [`codex-rs/execpolicy/src/execpolicycheck.rs` lines 15–95][execpolicy]
- **app:** [`codex-rs/cli/src/app_cmd.rs` lines 1–25][app]
- **ws-auth:** [`codex-rs/app-server-transport/src/transport/auth.rs` lines 26–160][ws-auth]
- **code-host:** [`codex-rs/app-server/src/code_mode_host.rs` lines 8–38][code-host]
- **login:** [`codex-rs/cli/src/login.rs` lines 110–425][login]
- **login-status:** [`codex-rs/cli/src/login.rs` lines 425–580][login-status]
- **config:** [`codex-rs/config/src/loader/mod.rs` lines 99–150][config]
- **strict:** [`codex-rs/config/src/strict_config.rs` lines 25–176][strict]
- **skills-config:** [`codex-rs/config/src/skills_config.rs` lines 15–170][skills-config]
- **skill-roots:** [`codex-rs/ext/skills/src/host_roots.rs` lines 25–175][skill-roots]
- **skill-limits:** [`codex-rs/ext/skills/src/loader/mod.rs` lines 18–32][skill-limits]
- **skill-merge:** [`codex-rs/ext/skills/src/loader/host_merge.rs` lines 100–200][skill-merge]
- **models:** [`codex-rs/models-manager/src/manager.rs` lines 183–295][models]
- **models-refresh:** [`codex-rs/models-manager/src/manager.rs` lines 385–688][models-refresh]
- **models-debug:** [`codex-rs/cli/src/main.rs` lines 2397–2426][models-debug]
- **feature-spec:** [`codex-rs/features/src/lib.rs` lines 905–1540][feature-spec]
- **remote-token:** [`codex-rs/cli/src/main.rs` lines 2821–2865][remote-token]
- **launcher:** [`codex-cli/bin/codex.js` lines 16–110][launcher]
- **update:** [`codex-rs/cli/src/main.rs` lines 908–994][update]
- **mcp-types:** [`codex-rs/config/src/mcp_types.rs` lines 1–100][mcp-types]

[root]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L117-L245
[mainargs]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L245-L878
[toggles]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L1046-L1128
[shared]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/utils/cli/src/shared_options.rs#L8-L105
[tui]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/tui/src/cli.rs#L9-L143
[exec]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/exec/src/cli.rs#L10-L318
[override]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/utils/cli/src/config_override.rs#L1-L110
[approval]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/utils/cli/src/approval_mode_cli_arg.rs#L1-L24
[sandbox-values]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs#L1-L34
[sandbox]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/lib.rs#L26-L190
[mcp]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/mcp_cmd.rs#L48-L222
[mcp-action]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/mcp_cmd.rs#L223-L550
[plugin]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/plugin_cmd.rs#L50-L220
[market]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/marketplace_cmd.rs#L34-L160
[queue]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/queue_cmd.rs#L13-L63
[migrate]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/migrate_rollouts.rs#L21-L125
[doctor]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/doctor.rs#L154-L182
[remote]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/remote_control_cmd.rs#L28-L130
[cloud]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cloud-tasks/src/cli.rs#L1-L120
[proxy]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/responses-api-proxy/src/lib.rs#L31-L60
[execpolicy]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/execpolicy/src/execpolicycheck.rs#L15-L95
[app]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/app_cmd.rs#L1-L25
[ws-auth]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/app-server-transport/src/transport/auth.rs#L26-L160
[code-host]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/app-server/src/code_mode_host.rs#L8-L38
[login]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/login.rs#L110-L425
[login-status]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/login.rs#L425-L580
[config]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/config/src/loader/mod.rs#L99-L150
[strict]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/config/src/strict_config.rs#L25-L176
[skills-config]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/config/src/skills_config.rs#L15-L170
[skill-roots]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/ext/skills/src/host_roots.rs#L25-L175
[skill-limits]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/ext/skills/src/loader/mod.rs#L18-L32
[skill-merge]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/ext/skills/src/loader/host_merge.rs#L100-L200
[models]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/models-manager/src/manager.rs#L183-L295
[models-refresh]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/models-manager/src/manager.rs#L385-L688
[models-debug]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L2397-L2426
[feature-spec]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/features/src/lib.rs#L905-L1540
[remote-token]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L2821-L2865
[launcher]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-cli/bin/codex.js#L16-L110
[update]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/cli/src/main.rs#L908-L994
[mcp-types]: https://github.com/openai/codex/blob/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564/codex-rs/config/src/mcp_types.rs#L1-L100

---

## Comparative acceptance backlog (unexecuted)

These are proposed Helm comparison journeys for #271, not defects proven in Helm and not executed Codex tests. Helm context is the local `docs/architecture.md` and `docs/current-state.md` (checkout base `09fffcaca7fcc868b8f05eae509042f1a977e796`, with unrelated concurrent modifications). Preserve Helm → Vessel → independent Voyage; do not import a competing embedded executor or confuse a backend named app-server with a desktop UI.

| Priority | Scenario / action | Evidence to collect and acceptance condition |
|---|---|---|
| P0 | Submit, steer, queue, cancel while tool is waiting on permission | Distinct admission/outcome labels, preserved draft/attachments, no automatic approval or replay; show which message is queued versus admitted to current turn. |
| P0 | Approve once / scope / deny, then revoke or disconnect | Show exact executing machine, resource and lifetime. A pending/accepted decision must not be presented as observed effect completion. Test rejection and cancellation, not only approval. |
| P0 | Disconnect during side effect; resume with active writer or dead runtime | Existing owner identity remains canonical; read-only viewing must be labeled; no claim that a dead process survived or uncertain external action was replay-safe. |
| P0 | Prompt edit versus file undo versus fork to same/new checkout | Before confirmation, state whether history, files and process are changed. Preserve original history and make recovery destination discoverable. |
| P0 | Private terminal input versus model tool output | Ensure human secrets remain uncaptured; do not infer Codex's shell rendering establishes Helm's F3 private-terminal guarantee. This audit did not find/verify an equivalent private PTY consent workflow; not a broad absence claim. |
| P1 | Cancel picker after typing, pasting image, queuing and switching thread | Original draft, attachment provenance, focus and queue survive; cancellation returns to correct context, not accidental creation/exit. |
| P1 | Unknown command, hidden/gated command, invalid inline args, busy command | Provide actionable correction while preserving text; list gated state separately from typo/not installed. Establish menu/parser parity with an enum-based inventory. |
| P1 | Streaming text/reasoning/tool output while transcript overlay or modal is open | Final canonical text appears once and in order; reasoning/tool details remain discoverable; live overlay does not mutate history or steal keyboard focus. |
| P1 | Tool exit nonzero, output truncated, remote link reconnect, approval invalidated | Distinguish command failure, policy refusal, transport failure, retry delay and cleanup-pending. Preserve unresolved obligations. |
| P1 | Diff staged-only, unstaged-only, untracked, binary and submodule changes | UI scope accurately matches actual input; empty results do not imply clean repo across omitted categories. Never invoke configured Git helpers accidentally during preview. |
| P1 | Auth logout/expiry, unsupported model/effort, missing MCP and malformed config | Explain the setting source, effective value, safe recovery and credential location. No paid calls until provider/budget approved. |
| P1 | Export/copy/feedback with tool outputs and attachments | Explicitly describe destination, canonical-versus-rendered content, omitted media and redaction. Transmission consent and clipboard errors must be visible. |
| P2 | Narrow terminal, unsupported enhanced keys, paste burst, Vim/remapped keymap | Keep essential actions discoverable and keyboard hint text consistent with active mappings. Native Windows/macOS require native evidence. |
| P2 | Enable experimental feature and restart; managed policy denies change | Show whether change is draft/persisted/live, why unavailable and what restart is needed. Do not silently treat default-off as unsupported. |

### Coverage limits and unresolved verification

- Complete built-in slash inventory and CLI parser families are sourced below; dynamic plugin/MCP/skill catalogs cannot be exhaustively enumerated without an installation/account/environment and are treated as extensibility, not missing built-ins.
- Editor keys are source defaults and keymap actions, not a guarantee that every terminal delivers the same escape sequence. Native platform, clipboard, microphone, sandbox, accessibility and screen-reader behavior remain unverified.
- Provider-specific models, organization flags, quotas, remote account policies, live reconnect timing and actual tool effects remain unverified. No credential material was read or reproduced.
- Desktop/web/IDE destination interfaces, protocol-only backend endpoints without a terminal action, and a line-by-line security review of every backend subsystem are outside this terminal UX scope.
- There is no overall numerical UX score: source feature presence is not discoverability, correctness, low latency or safety certification. Comparative lessons propose acceptance criteria, not implementation obligations added to issues.

## Source manifest and verification

### Acquisition manifest

| Artifact | Identity / purpose |
|---|---|
| GitHub HEAD resolution | `gh api repos/openai/codex/commits/HEAD --jq '.sha'` → `36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` |
| Commit metadata | `codex/commit.json`, retrieved from `repos/openai/codex/commits/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` |
| Download | `codex/source.tar.gz`, public `https://api.github.com/repos/openai/codex/tarball/36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` |
| Download SHA-256 | `c63bfbf44d05b7e7b29c9e4963a7e49509bce9ea23c8889a055cafbab257486e` (this fetched archive; server archive byte identity may change independently of Git tree) |
| Extracted tree | `codex/openai-codex-36f0dbe/` |
| Cited-source manifest | `codex/source-manifest.tsv`: path, local SHA-256, total line count for every pinned upstream source linked by this report |
| Link/inventory check | `codex/report-validation.json`: local file/anchor bounds and built-in slash spelling coverage; not runtime execution evidence |

Paths in this manifest are relative to `target/ux-audit/`. All upstream implementation hyperlinks use the full immutable Git commit, not `main`. Subsection source manifests identify the primary files; the TSV includes all linked implementation files, including session/review sources.

**Local report checks:** 276 pinned citations resolve to 92 local source files with valid line bounds; all 59 slash enum variants have inventory rows. These are static document checks, not executed Codex behavior.

### Published compact cited-source manifest

```text
path	sha256	lines
codex-cli/bin/codex.js	61b0194f3bb6534439c8d26a3ed57d0805f84b884588b761795323eeb92fcf70	295
codex-rs/app-server-transport/src/transport/auth.rs	1af6224369deb23e7f894d9e4c42f897d274c9bee766382add5726a80655888d	751
codex-rs/app-server/src/code_mode_host.rs	39a0b682da90b7b567ef4dcbb4ba8b310401ce17a23345cb79084592b4daaf5d	83
codex-rs/cli/src/app_cmd.rs	bb3035022dfe3424598cd6906adda01a967d449f8987cd777d3de1053ec4ed2a	25
codex-rs/cli/src/doctor.rs	11dea3ed64871542a4e147b741a652ffdf201c4bf805d1dee71742b62bd2411f	4287
codex-rs/cli/src/lib.rs	cf24032f801314033b2ff21e6c5a85ad8c2898012e9fba279f3a5c91439c1ab6	190
codex-rs/cli/src/login.rs	34f8a089a6907d9791e4bb858f44fdee56350b9898daad45c2065325f9e6bef7	639
codex-rs/cli/src/main.rs	5cf524d836322dc6130cc7b3718271233db28bdbcfb0227decdfbb1b87c9888a	5167
codex-rs/cli/src/marketplace_cmd.rs	21688848f8d7946cdb69308c482f1473bb2efd8cee2743b1c4b0a1b105957c92	587
codex-rs/cli/src/mcp_cmd.rs	a5dd0a7a5e519daddd64e12fa876ece961db9fd8628e945bd49b8e407dc6a81f	1100
codex-rs/cli/src/migrate_rollouts.rs	3cffc54e387fddd1d35af76f37ee6d70af15dce728c78d7680c11469267a38c8	450
codex-rs/cli/src/plugin_cmd.rs	2300c5b2463da7466263b9d769e3ff591884cb7ec7a866ec2935469b8762ae7e	1092
codex-rs/cli/src/queue_cmd.rs	6dd11747249b2cb0a36f6dcd17756a4ebe14361067a1d3af5b37716e1b22dceb	63
codex-rs/cli/src/remote_control_cmd.rs	e579a28078e0dcee10a9a99ae2baf38e889add00a9d7ebb46eb02e2656d0a1c9	780
codex-rs/cloud-tasks/src/cli.rs	9ffbf4e43e12986a3ade28f05bc3d70e80d2860314f32ace7b3060148bc2fbaa	120
codex-rs/config/src/loader/mod.rs	a1d27cc2aef3cdf87cd22abeb9e6300fee2cec631b615e60395dcfa32e1ac6a5	1982
codex-rs/config/src/mcp_types.rs	0b1c05689e5f07603e6129b8deb9df61b3f3ea4542901984abe48d877dd54247	616
codex-rs/config/src/skills_config.rs	bc2eb9fcb92c2c99fedaa25d1ca4bbff6d69a009499ba34762249bdc3b05cc8c	215
codex-rs/config/src/strict_config.rs	bb1a8b9e64c5f62db3dfb3dd53ef564c756e249d4caa6f2c5fb1ca9f86336f68	312
codex-rs/core/src/config/mod.rs	405fd7cac4aefbbf538e63bc6c3b2390f3c6e3c01c8e6d4a535b797a7afe3a91	4781
codex-rs/core/src/config/permissions.rs	96ff0f5de4d2c8918b7b6657cae26dba73cb343f7953dcf4623117e885b67f93	893
codex-rs/core/src/tools/handlers/request_user_input.rs	f8cf561ba3e6c6527bdc6b7c66330b4a3e71ce978bde52077f4b4e5efbf3a016	175
codex-rs/core/src/tools/orchestrator.rs	824a1b318a718f69b5e0f3f0e3c7bc46bf66742c39f1a514e1ddb2cb2b64334f	556
codex-rs/exec/src/cli.rs	6abb86feb8fac2a192b34a617634527d0747b6d7fa09ee4827c0dc95abbe2f75	319
codex-rs/execpolicy/src/execpolicycheck.rs	ca0fd263be42e0ee7dab450912c409b8874dd2ec5f74e2650ba5de2deb8b1955	95
codex-rs/ext/skills/src/host_roots.rs	d4290ba4d783110e7533586ad19719536aa9b23f517b43e3147d3afb40c91d56	278
codex-rs/ext/skills/src/loader/host_merge.rs	7e2bce7a2bfb98401a3adf60fadf6a92ad8824b3dc9ad523fbc50997342e5532	273
codex-rs/ext/skills/src/loader/mod.rs	5b97cef387ccead62e3cdec2ebc0308747e4c9d5a5d6a6bc3ae341b61d6db35d	32
codex-rs/features/src/lib.rs	a21065af423dab633499802df24e7b4fd7330bc47ff7e43180eef7f70939a350	1842
codex-rs/models-manager/src/manager.rs	8ae229b8587990f9f9643d019a56703f48465172ddf75bc22a158c09f489ed90	751
codex-rs/protocol/src/config_types.rs	079f3650c59c002a5ea64feb01709c7aaf9cf904bce54aae42ae971fc068275e	980
codex-rs/protocol/src/protocol.rs	141ce4afc82e1f9dfb299f89500bf4ed4e1bd6909702824194b09c56f14343b1	6338
codex-rs/responses-api-proxy/src/lib.rs	092e92662b9a564f98bd5ee157c09348465288a1656adcd5eb34d555dc551c1f	275
codex-rs/tools/src/tool_config.rs	1cb8ecfd0d0721bb9b30e82492128e7fd1f2aa85faa12a607e01cfdd144c3d95	103
codex-rs/tui/src/app/event_dispatch.rs	095c05afb12bae14aeb43f52e6786ed14a1c286b7077c1e8a1a7c01a48fbbaf0	3326
codex-rs/tui/src/app/input.rs	96c3e1535cb720488a3e74054178a191b9e12380bfb2164b1923f525f0542bc8	629
codex-rs/tui/src/app/managed_worktree_creation.rs	dd3748337288a5460549adeb278cc8c6eee2a03be13e8944db85341a655ad8b5	288
codex-rs/tui/src/app/session_lifecycle.rs	cf77ee779a0a85be6ccbb1d9d3711c9c2e21f4c1bdca49e2c4aea97d9a3b7961	1418
codex-rs/tui/src/app/session_picker.rs	f1e2a0f3e0fa85477b1dc9d3ba2dbdb44f19194df8cbb33b19ad0251621781ad	159
codex-rs/tui/src/app/thread_routing.rs	91c68ff1a32a522a0b0cff65966f58b2b1c7f9af8fb4d4ee52cf060c85aa980a	2220
codex-rs/tui/src/app_backtrack.rs	e2674ae8e0663aabc3ae1f2f8c04c93e3db74a66b2a92637bba38dc4153a356a	983
codex-rs/tui/src/app_server_session.rs	f3f5159c4ef84826efbe86a5bcc58e0c7e9ef7a5758a86cb787ae099b85ee8b1	4217
codex-rs/tui/src/bottom_pane/approval_overlay.rs	d54cf42a84d523db57d9a0ad2bee4e361e31cec4e714b8de755d2085ae45e125	2510
codex-rs/tui/src/bottom_pane/chat_composer.rs	5dec0ca733a74304a4b0025429655201778fdc03b9acca1198ac881274bf7602	13048
codex-rs/tui/src/bottom_pane/chat_composer/history_search.rs	2b85458c46e8769b4ee998f974eaa6ef3dd84f7a34c61a82429cbac19c028641	1191
codex-rs/tui/src/bottom_pane/chat_composer/slash_input.rs	b80bad370a5eedb258db7b8e63b6a248638cc150ecca77bacc4871cb1af82f49	687
codex-rs/tui/src/bottom_pane/chat_composer_history.rs	781e1409025bf0af4f75a011b3ff8cbff407b514e5579c074e9faaf07efe7e4f	1628
codex-rs/tui/src/bottom_pane/command_popup.rs	b915bc0a5a982650ede4fce8aa857e22c2459cd6f978bb4d281bb1b70c8b570c	599
codex-rs/tui/src/bottom_pane/footer.rs	e1cabdb6bfb494738b11f1f4cc93dab014d342a384f5802a51b13a290447a466	2154
codex-rs/tui/src/bottom_pane/mod.rs	598b2c6e293c557dfb1e20ef2ceb82ac5b94db81a51fc8c4f25325db54ec0c1c	3651
codex-rs/tui/src/bottom_pane/paste_burst.rs	59d39542b7a8adb5dc69c4847975b1af0e590ab412ded9cfe7e944297fcad4cf	589
codex-rs/tui/src/bottom_pane/request_user_input/mod.rs	00e983e84972c9d84c7995631f73bef18217c7bfb3b07438c6fd9c16631b192e	3907
codex-rs/tui/src/bottom_pane/selection_popup_common.rs	15e0ad4bd77a742a472c252c267597b2eb86c7d95ac3356a306d1a17c7964ffc	969
codex-rs/tui/src/bottom_pane/slash_commands.rs	49f7dcd39054af929f6c36f8b2c4043fe1c5af7b562fe0385cfcbeccadee5797	372
codex-rs/tui/src/bottom_pane/textarea.rs	ed8f5f35ca8eef9b586faf617ca990565a39c6af4e6760358f503408387c5b6d	4619
codex-rs/tui/src/bottom_pane/textarea/vim.rs	a7fbdf9166d84a0eab7f6caf354b43334d5391a1f2935905b3622d7d862fe6c4	336
codex-rs/tui/src/chatwidget.rs	28c0ff76f5dbab70932ebe700d6854a51071918b54af3787bed497d23cdd83fe	2122
codex-rs/tui/src/chatwidget/command_lifecycle.rs	5bca6a87d6fa2c31ee6d98483e4ec0a396e73a715bd4ec11f65918218ead6f84	458
codex-rs/tui/src/chatwidget/connectors.rs	d75d69df22d14f33eeb4a05ab25f6b64e4fb16fa76144c4199ee740f4f47a8d3	522
codex-rs/tui/src/chatwidget/ide_context.rs	3a949a29e834d5eb21d71161d9a7bc802f9e5eaa57e525c538b296dfd3848959	132
codex-rs/tui/src/chatwidget/input_flow.rs	418647bfc20b6276b11fe08fe27f570af6af51382ba9f051cf142bff7d635bbb	386
codex-rs/tui/src/chatwidget/input_queue.rs	7b9f28ddfe99745206f03a73cd7589bc39b474bd5ec7f981d48ba6a879d06ce1	165
codex-rs/tui/src/chatwidget/interaction.rs	c07d59a9dd80a55c6680d91dcadafebcde22f64e7d2f8f1735879f53e7c36fc4	688
codex-rs/tui/src/chatwidget/protocol.rs	d80e013058f3ab8811f1030e698d13de106de29886c8dfded9b4838a38436f0e	616
codex-rs/tui/src/chatwidget/reconnect.rs	f57114dc8645f4e8559074858dd2edf58d451dad0506218f89be65d621deeaca	115
codex-rs/tui/src/chatwidget/review.rs	9469883b572f4e86613e5f0b49caedd31f38bd47bc1ff0fc4c86323be96a5366	13
codex-rs/tui/src/chatwidget/review_popups.rs	8d1612a1d2b6cba48a8c15a53de1144e565e4351383efe3a21113e9ca33e7cf4	186
codex-rs/tui/src/chatwidget/settings.rs	7ce94cd12777b48302074aaab7b2c2a88a0841ec5f280766023004daaa24bd3b	713
codex-rs/tui/src/chatwidget/slash_dispatch.rs	3100759256a1049326982d59a801d7c8d47fedeb7ab22d02faad61767c57e86a	1287
codex-rs/tui/src/chatwidget/streaming.rs	8e206e4eaf7dc7ad00f0acf102bf3a69bbb04db16a894f28575ec673086ba052	641
codex-rs/tui/src/chatwidget/tool_requests.rs	b40527c4014b8f7d35f7a6dd08339f089b66da01533c79de3229a86d590db129	485
codex-rs/tui/src/chatwidget/turn_runtime.rs	10b4cd49eeb7d6d8e4a3a6cd2434cff4d9c084f2bc29fab25f9839757521fe1d	560
codex-rs/tui/src/chatwidget/windows_sandbox_prompts.rs	0b52cd33df3e7507572b832a120c60f821a113c047b3872959a32ba897ecca47	328
codex-rs/tui/src/chatwidget/working_directory.rs	391117c29078bf2e431a7d12952defb483eb7d7542e76dbdea986634cc160d79	47
codex-rs/tui/src/chatwidget/worktree_picker.rs	f6fffe4292af9e61d28ecac18014403657eefc03572b566c72a78da303b50f3b	394
codex-rs/tui/src/cli.rs	742c86c920c0df679c46542918c9661cff36d68d8fc78c5398263872e2c454f6	144
codex-rs/tui/src/clipboard_paste.rs	9f426c24d68a9fb1a638d1742ca73d00cfaef60e71b129c58fc8ad7498a1367b	568
codex-rs/tui/src/external_editor.rs	530d05c2c5ab0fe6bce502c4e1f5c4910db73114ba5ce1b2756bb1a8d9ecf915	317
codex-rs/tui/src/get_git_diff.rs	ae5fbf00c2fd56dd5de228f650b87395d2a31088d18a5310755d15ce23c69ed1	941
codex-rs/tui/src/keymap.rs	67fb23246b89ff6a2a33c5c0b77676172bf8cb88805986b7ca934d69673a8ec7	3966
codex-rs/tui/src/keymap/bindings.rs	fcda06192f2fa72c082bcb5b765ff5c8304743b44e03532c628753b5e8604368	416
codex-rs/tui/src/keymap/vim_search.rs	7649f552bd46f3c10baddf61d4fe3ddcdf7690fd84e054fe97aba7c0f2c7420f	77
codex-rs/tui/src/keymap_setup/actions.rs	5091cdcb968f6091f8b6a630019d89f5ccb453813c5747959ecfaf34930664c9	531
codex-rs/tui/src/resume_picker.rs	100637bc19547214ee64c6fb31c21698dc66a609527bc0cb5f32e90362b48cc3	7238
codex-rs/tui/src/resume_picker/archive.rs	b39684b88e5c3c7b941c963dfeab259db93fdb45095af52e4b6424f31f0d6f7a	172
codex-rs/tui/src/session_archive_commands.rs	1864ef0ae9a6cdc5436e92265bfd3f3a0fe9248746ca8facee7eabfabcb14a0e	382
codex-rs/tui/src/session_resume.rs	7bb9035899d97fed7b62d48df7657f0ae66d39a325fc512448a34cece8e2bf16	217
codex-rs/tui/src/slash_command.rs	2141ca063111eaabb2851b41063e9de55938f7a832059e26470a1ae6c9133a16	328
codex-rs/utils/cli/src/approval_mode_cli_arg.rs	951511069a9778403f6a9a6e12c5d89778b70911728db4585f8d05ba9d2b36b3	25
codex-rs/utils/cli/src/config_override.rs	7edda8533dd3395b22214f140eb2c93a45b7dad2595635f57691099d408ce071	170
codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs	ffc5526811eeb12dea1fdafeea6971d53129bec017b272459d626fe5e394a49b	47
codex-rs/utils/cli/src/shared_options.rs	6eefb5fcf0545fca54ffa1f80a6d3630c8d9b61ec449466d5834ac9c489f76d8	220
```
