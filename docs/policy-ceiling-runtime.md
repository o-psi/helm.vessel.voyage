# Runtime administrator ceiling

On Linux, ordinary Helm CLI, TUI, resumed sessions and child builders read the fixed
protected `/etc/helm/policy-ceiling.toml` source described in
[policy-profiles.md](policy-profiles.md). There is no user-configurable path or
skip flag. An absent source under trusted ancestry preserves the existing Config
rules. An invalid or untrusted present source prevents startup before provider or
MCP construction. The workspace must fit both administrator root sets because
Policy adds implicit workspace read and write roots.

Builders consume one resolution for Policy, tool environment, access mode,
unattended approval and child authority. A runtime-only Config clone contains the
restricted settings. The original selected Config remains the persistence and TUI
handoff source. Existing Config environment names and redundant roots retain their
old validation contract; imported profile and ceiling documents remain strict.
Provenance contains rule names and paths, never environment values.

Inherited environment names are intersected with the ceiling. Explicit Config.env
and individual MCP server environment values keep their existing precedence unless
the administrator excludes that name. This uses the actual administrator name cap,
not the final inherited-name list: an explicitly allowed value need not also be
inherited. When a ceiling exists, internal managed-worktree Git commands also use
the filtered environment. Without a ceiling, their existing ambient environment
behavior is preserved.

Before another turn, a reused Agent resolves the ceiling and workspace identity
again, before canonical input or provider work. A changed snapshot refuses the turn
and requires restart or rebuild; it does not silently promote or relabel a live
runtime. New children intersect their captured parent delegation with the fresh
ceiling. New subagent operations check the parent snapshot again.

Worktree creation requires a single-component name and a prospective destination
inside both parent read and write roots before creating directories or running Git.
It also follows command policy and approval. This deliberately tightens existing
no-ceiling behavior: the default external worktree directory is not automatically
granted. Operators must explicitly delegate a suitable root. Subagents cannot
restore parent-excluded environment names, denied commands or access modes.

These are application-policy checks, not an OS sandbox. Already-running effects
are not instantaneously revoked by administrator edits, and filesystem paths can
change after a check. Providers and local trusted library callers are not granted
new execution authority by metadata. Profile CRUD, import/export commands and
interactive switching are separate unfinished #70 work.

The new protected-ceiling feature is Linux-only. Existing ordinary non-Linux
startup retains its previous Config behavior through an internal compile-time
adapter; it does not claim administrator-ceiling enforcement. Explicit public
profile resolution still reports unsupported enforcement there. Linux builds have
no corresponding bypass. Native tests are not claimed.

The `tests/system/policy_ceiling.py` fixture runs the real binary, local HTTP
provider, MCP stdio process and TUI PTY in a disposable user/mount namespace and
chroot. It uses the production fixed path and ownership checks, including unsafe
files, workspace exclusion, environment precedence, resumed startup, and a changed
ceiling between turns. It verifies that rejected second-turn input is absent from
durable canonical history. Namespace creation may be prohibited on some Linux
runners; those report an explicit skip rather than passing this system evidence.
The ordinary deterministic runtime and loader tests still run. No host `/etc`
state is modified and no external provider is contacted.

Every internal managed-worktree Git subprocess checks the captured current policy
and its actual argument deny entries. This includes inspection, commit/add,
integration/merge, removal and failure cleanup. The denial-only internal check
is not an access grant: mutation still follows the existing operation approval and
access-mode flow. It also does not classify harmless internal Git inspection as
arbitrary shell execution in read-only mode. A newly denied cleanup can leave the
worktree for explicit operator handling; denial does not imply rollback.

Standalone, plain-chat and TUI model discovery also checks current policy before
provider construction or refresh, including the optional compatibility subprocess.
Configured values removed from child environments retain their original redaction
coverage in the runtime-only clone, without entering rule provenance.
