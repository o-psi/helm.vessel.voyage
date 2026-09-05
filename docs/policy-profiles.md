# Policy profile resolution contract

The `policy_profile` library provides typed profile documents, preset expansion,
effective-rule resolution and a protected Linux system-ceiling reader. Current
CLI, TUI and execution builders do **not** consume these results yet. Creating a
ceiling file therefore does not currently protect ordinary Helm runs. Profile
management and runtime integration remain required work under issue #70.

## Typed rules and presets

Schema 1 profiles contain a name, positive revision and complete rules. Restricted,
Balanced and Autonomous expand respectively to read-only, approval and unrestricted
access. All three preserve workspace read/write roots, deny `shutdown`, `reboot`
and `mkfs`, inherit only the names `PATH`, `LANG`, `LC_ALL`, `TERM`, and deny
unattended automatic approval. Read-only access still disables mutations; a listed
write root does not override that mode.

Rules support access, unattended approval, read/write directories, denied command
basenames and inherited environment **names**. Roots are absolute existing directory
paths or the literal `$workspace`; they are canonicalized before comparison.
Relative paths and parent traversal are rejected. The resolver preserves current
Policy's implicit workspace roots and records when that supplies an omitted root.
It cannot express excluding the workspace while otherwise granting arbitrary tool
access.

Unknown fields and schema versions fail closed. Profile JSON is bounded to 64 KiB,
each collection to 128 entries, names to 128 bytes, and root spellings to 4,096
bytes. Export serializes the typed document only: no Config environment values,
API keys, MCP credentials, provider configuration or redaction values are read or
copied. This is not a generic secret detector for text an operator deliberately
puts in a path/name field.

There is no network policy, OS sandbox, new top-level tool grant, secret store or
restored subagent lifetime/child/history cap. Unsupported declarations are rejected.
These rules are application policy, not process containment.

## Resolution and provenance

`resolve_current(workspace, base, layers)` is the only public constructor of an
`EffectivePolicy`. It always checks `/etc/helm/policy-ceiling.toml` on Linux; there
is no public alternate path, environment override or skip flag. Effective fields
and workspace identity are private and exposed through read-only accessors.
They cannot be deserialized from untrusted input as a resolved policy.

After the caller translates existing access settings into base rules, layers must
be ordered global, project, session, explicit, with at most one of each. An override
replaces its whole field rather than implicitly appending grants. `Layer::from_profile`
binds provenance to the validated profile name, revision and document digest.
Explicit field overrides have their own identity and digest. The pure resolver
operates on prepared canonical inputs; filesystem reads occur before it.

Each field retains contribution history, including explicit assignments that happen
to keep the same value and the final system-ceiling restriction. Roots and sets are
normalized deterministically. The effective digest binds final rules, source history,
ceiling digest and canonical workspace device/inode identity. Profile revisions
with identical rules still have distinct provenance. A digest is binding evidence,
not authentication or proof of operator interaction.

Repository files and session transcripts are not automatically loaded as layers.
Future profile selection must derive those inputs from explicit local operator
choices and retain pinned revisions; imports must remain inert until selection.

## Protected Linux ceiling

The protected file is strict TOML with `schema = 1` and a `[rules]` table containing
the same complete rule fields. An absent source leaves base/layer rules intact.
An existing unreadable, malformed, oversized or untrusted source fails closed.
The loader pins `/`, `etc`, `helm` and the file through directory-relative no-follow
opens. Every component must be root-owned and not group/other writable. The file
must be a regular single-link file; symlinks, hardlinks and FIFOs are rejected.
Pinned identities and metadata are checked again after the bounded read. There is
no automatic creation, repair or rewriting of administrator state.

The ceiling intersects read/write roots and inherited names, unions command denies,
and selects the more restrictive access and unattended-approval modes. The workspace
must itself fit **both** ceiling root sets, even in read-only mode, because existing
Policy always includes it. Empty ceiling roots therefore refuse the workspace;
they do not trigger an unrestricted fallback. Canonical root intersections may
narrow an extra root to an allowed descendant, never widen it.

Root-owned administrators can intentionally change or remove the ceiling. This
reader is not protection against root, hostile same-UID process manipulation,
filesystem rollback, or stale runtime state after resolution. Every future launch
and runtime rebuild must call the checked entrypoint again. Non-Linux resolution
currently refuses rather than silently omitting a ceiling; schema operations remain
portable. Native platform validation is not claimed.

## Transition confirmation and integration requirements

`transition(previous, proposed)` requires matching canonical workspace filesystem
identity. Increasing access, enabling unattended approval, adding root/environment
grants, or removing a command denial requires confirmation. A mixed restrictive and
permissive change counts as escalation. Narrower roots and stricter rules do not.

The transition digest binds both effective digests and workspace identity. Comparing
an exact digest confirms only that metadata. A cached transition or successful
`confirm` call is **not** an execution permit. The caller must freshly resolve the
current ceiling and pinned profile revisions before committing a transition.

Existing Policy, tool environment, child policy and terminal managers must all
consume the same final rules; applying only some fields would allow bypass. Explicit
Config environment overrides must not re-add names excluded by a future environment
ceiling. Profile switching must stop and observe prior owned effects before rebuilding
and showing a new effective-policy label. A missing approver must never cause an
unattended wait or authority promotion. CRUD, these frontend paths and actual ceiling
enforcement remain unfinished; the library alone does not complete #70.

Tests exercise strict schemas, presets, source history, root/set intersections,
escalation and stale confirmations, canonical aliases/recreated workspaces, and
protected-source ownership, permissions, links, malformed data and replacement.
Filesystem tests use private internal injection; they do not modify operator `/etc`.
