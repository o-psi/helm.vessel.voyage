# Runtime guidance

Helm treats model instructions as runtime configuration, not conversation data. The configured
base prompt describes durable operating behavior, while every execution generates a capability
contract from the live tool registry. Tool names, descriptions, and schemas therefore have one
source of truth.

The generated contract identifies the process as Helm, lists only currently registered calls, and
explicitly rejects capabilities inherited from a provider host. Provider adapters may reinforce
that boundary, but they must not invent a second tool catalog. `/tools` reads the registry directly
and is the deterministic operator-facing view; it does not ask the model to remember its tools.

System messages are deliberately excluded from saved sessions. On resume, Helm preserves user,
assistant, and tool history, then reconstructs current guidance. Loading also removes system
messages from older files. This prevents stale policy, disabled extensions, renamed calls, or a
provider's surrounding harness from becoming durable authority.

Tests should exercise both levels:

- structural tests prove generated guidance and persisted sessions reflect the registry;
- a live provider smoke test asks for exact call names and rejects foreign capabilities;
- behavioral evaluations require the model to select and successfully use relevant calls.

This follows the pattern used by agent runtimes that pass tools as explicit name/description/schema
objects and generate active extension context at runtime. It also accommodates Codex app-server's
current constraint that dynamic tools are supplied when a thread starts: a Helm agent's registry is
fixed for that provider thread, and a changed registry requires a new thread.

The planned [voyage model](voyages.md) does not add tools to today's registry.
Remote delegation, coordinator handoff and interface attachment must be exposed
through implemented contracts before models may use them. Each executing Helm
continues to construct guidance from its own registered capabilities and policy;
a coordinating role is not permission to inherit another Helm's authority.

## Automatic project instructions

Before each agent execution, Helm reads `AGENTS.md` in the active workspace root.
If that entry is absent, it tries `agents.md`; it never combines both. An empty
preferred file is still authoritative for selection. No parent-directory or nested
instruction discovery is performed. Select the intended root with `--workspace`.

The UTF-8 contents are included automatically in the model's system context, before
Helm's authoritative runtime contract. No `read_file` call or prompt from the user
is needed. Project instructions cannot grant tools, broaden filesystem roots,
disable hard denies, or override approval requirements. Configured secret-value
redaction is applied before sending them. This sends project guidance to the selected
provider: do not put secrets in these files; redaction is not a general secret detector.

Loading follows the workspace read policy, including canonical symlink targets and
explicit extra read roots. Files must be regular UTF-8 files of at most 65,536 bytes;
invalid, unreadable, dangling/denied symlink, non-regular, and oversized entries fail
the execution locally rather than silently omitting or truncating instructions. A
missing file is harmless. These checks use Helm's application policy, not an OS sandbox.

Each execution uses one snapshot across its model/tool loop. Edits or deletion take
effect on the next user turn/run, including resumed sessions. The shared agent entry
point applies this behavior to plain chat, TUI, one-shot runs, and
subagents. Children use their own workspace root; isolated Git worktrees see their
checked-out files, not uncommitted/untracked instructions in the parent's tree.
The optional Codex compatibility bridge starts a new thread when instructions
change between completed turns and replays canonical conversation history.

Injected system context is not returned as conversation history or written to session
files. Resume reconstructs it from the current file, and removes any saved system
messages. Explicit tool reads or assistant quotations of a file remain ordinary
conversation content; this feature does not retroactively erase those records.

Coverage includes bounded loader and capturing-provider tests, native Responses and
OAuth wire fixtures with tool follow-up/resume, and a compatibility-thread refresh
fixture. The `automatic-project-guidance` evaluation checks whether a real model can
report a seeded phrase without requesting a tool read; `eval/run.py validate` only
validates the scenario, while `live` requires a configured provider and budget.
