# Filesystem skills (`SKILL.md`)

## Automatic executing-host discovery

At the beginning of each run, **Voyage** discovers ordinary filesystem skills on
its executing host. This works independently of Helm's extension CLI and applies
to Vessel-supervised local and remote voyages. Vessel supervises the runtime;
it does not load skill instructions into its own process or copy client files.

Discovery scans immediate child directories of these roots, in order:

1. The Voyage workspace's `.agents/skills`.
2. The executing user's `~/.agents/skills`.
3. Nested projects' `.agents/skills`, in bounded breadth-first, lexical order.

Each catalog entry carries a `scope`: the owning project directory, or `null`
for executing-user skills. Nested skills are candidates only for work in their
project subtree, not global instructions. Root skills remain workspace-wide;
nested scopes do not silently override parent skills or resolve duplicate names.
This is model-facing selection guidance, not an additional filesystem permission
boundary. The runtime advertises scoped metadata at run start so work can cross
project boundaries within a run; bodies are still loaded on demand. Newly created
scopes are discovered on the next run, not dynamically after each tool call.

Project traversal visits at most 4,096 directory entries and eight directory
levels below the workspace. Hidden directories and `vendor`, `node_modules`,
`target`, `dist`, `build`, `coverage`, and `__pycache__` are skipped. It does not
follow project-directory symlinks or search for arbitrary `SKILL.md` files.
Laravel `storage` is also skipped when its parent contains `artisan`.
Bounds and traversal failures report incomplete discovery rather than claiming
all scopes were found. Skill-root and entrypoint links retain the checks below.

Canonical roots and identical `SKILL.md` targets are deduplicated. Directory and
entrypoint symlinks are supported **only when their resolved targets are within
current policy read roots**. Discovery never grants additional roots; an Omarchy
link into `/usr/share/omarchy` requires that target to be readable under the
Voyage's policy. Missing default roots are normal; denied, broken, malformed,
missing-entrypoint and oversized candidates produce model-visible diagnostics.
No ancestor traversal, legacy `.codex` scan, recursive skill grouping-folder search,
remote URL download, hooks, or script execution is implicit.

The provider gets an ephemeral, JSON-escaped catalog of names, descriptions and
canonical paths, not full instruction bodies. When relevant, it uses the ordinary
`read_file` tool to load `SKILL.md` and referenced resources. Each tool read is
independently subject to current runtime policy. Catalogs are not injected into
saved conversation history; requested file reads remain normal tool evidence.
Registries without `read_file` do not receive a catalog. Resource references are
relative to the canonical entrypoint's directory. Files are live, not pinned:
changes are observed by later reads and the catalog refreshes on the next run.

Metadata uses the supported frontmatter subset below. Discovery is bounded to
128 child entries across roots, 64 KiB per entrypoint, 256 KiB aggregate source
bytes and 32 KiB of catalog JSON. Entry overflow refuses that root rather than
selecting a nondeterministic prefix. Byte/output exhaustion is explicitly marked;
it is not evidence of a complete scan. Different files declaring the same name
are both listed with a duplicate diagnostic, without an automatic winner.
Skill metadata and loaded instructions cannot override runtime tool inventory,
permissions, or other higher-priority instructions.

## Optional reviewed, pinned imports

Helm can also import ordinary filesystem skills into Voyage's existing format-1
declarative packages. This remains separate from automatic filesystem discovery.
You do not need to write a manifest or file hashes. Discovery and import through
the CLI **do not activate package instructions**. Package activation is a separate
review of exact installed bytes and is applied on the next run. Unlike live
filesystem discovery, the immutable import adapter rejects symlinks.

## Use

Run these commands on the executing host, from the intended workspace:

```sh
helm extension discover-skills
helm extension discover-skills --root /path/to/team/skills --legacy-codex
helm extension inspect-skill /path/to/team/skills/example
helm extension --scope project import-skill /path/to/team/skills/example --expected REVIEWED_DIGEST
helm extension --scope project inspect example
helm extension --scope project enable example --expected REVIEWED_DIGEST
helm extension --scope project resource example references/guide.md
```

`inspect-skill` prints JSON containing the full instruction/resource snapshot,
source path, metadata, warnings and deterministic archive digest. Review that
content before passing its digest to `import-skill`. Output strings are JSON
escaped, not raw terminal instructions. The name in frontmatter is the package ID.
Scope defaults to `user`; use `--scope project` consistently when wanted.

CLI discovery searches **only immediate child directories** of these roots, in order:

1. Workspace `.agents/skills` (the workspace selected with Helm's global workspace
   option, otherwise the current directory).
2. User-home `.agents/skills`.
3. Explicit repeated `--root` directories, in argument order (maximum 16).
4. Only with `--legacy-codex`: workspace and user-home `.codex/skills`.

There is no ancestor/repository traversal or implicit scan of unrelated folders.
Identical roots are deduplicated; child names are sorted. Duplicate skill names
are marked on every candidate: **no automatic winner or activation**. Select the
source directory explicitly. Missing default roots are skipped; inaccessible
roots, malformed candidates and unsupported content receive diagnostics. Discovery
is limited to 128 child entries in total and refuses overflow rather than silently
omitting later skills. Project and user package scopes keep their existing catalog
and activation rules; discovery order does not alter those rules.

## Supported frontmatter

```markdown
---
name: example
description: >
  Brief description of when this skill is useful.
---
Follow these instructions. See references/guide.md for supporting text.
```

The first line must be `---`, followed by a string mapping and closing `---`.
`name` must satisfy existing portable package-identifier rules; `description` must
be nonempty and at most 2,048 UTF-8 bytes. The instruction body must be nonempty.
The supported YAML subset is plain scalars, single-quoted strings (doubled quote
escaping), JSON-compatible double-quoted strings, and indented literal/folded
blocks (`|`, `>`, with optional `-`/`+`). Block lines are trimmed and joined for
metadata display; the **original file text remains byte-for-byte in the package**.
Blank/comment lines are ignored for metadata display. Quote scalars containing
colon-space or inline comment syntax. Duplicate keys, nested mappings/sequences,
anchors/tags and unsupported scalar syntax are refused with diagnostics rather
than silently misparsed. This is not a general YAML parser or full Codex parity.

Other scalar metadata is retained as text with warnings, not interpreted as
permissions, tool dependencies or configuration. Optional `agents/openai.yaml` is
preserved as an inert resource; its contents are not applied. Foreign permissions
never broaden Voyage policy or the live tool registry.

## Snapshot and resources

The adapter renames the entrypoint `SKILL.md` to `skill.md` to satisfy existing
format-1 validation, without weakening it. Other relative paths are preserved.
Self-references to the original uppercase filename are not rewritten. A generated
`voyage-import.json` resource records the absolute source directory and original
per-file hashes; those paths become part of the reviewed package, so do not import
source paths you do not want in package context.

The full source tree is bounded to 128 entries, eight directory levels, 31 files
plus provenance, and 64 KiB of source bytes. The final JSON archive must also fit
the existing 64 KiB limit (escaping/provenance can make it larger than the source).
Resources must be UTF-8 regular files with existing lowercase portable paths.
Binary assets, uppercase/nonportable resource names, reserved `skill.md` and
`voyage-import.json` paths, symlinks (including source/root path components), special
files and parent traversal are refused explicitly. Hidden folders are not silently
ignored: remove unrelated files from the selected skill directory before importing.

Scripts are preserved only as inert UTF-8 resources. Import runs no hooks, marks
nothing executable and does not materialize files onto the workspace. Relative
references remain names within the package; retrieve resources through
`helm extension resource`. Any later workspace copying or script execution must
be an explicit separate action using existing tools and executing-host policy.

## Updates, moves and remote use

Installed skills are immutable snapshots, not a live source-directory watch.
Editing source files does not modify already installed instructions. A stale
inspection digest refuses import. Inspect again after any edit or move; moving
the directory changes provenance and therefore the review digest.

To replace an installed snapshot, supply **both** the new reviewed digest and its
current installed digest from `helm extension inspect`:

```sh
helm extension --scope project import-skill /path/to/skill --expected NEW_DIGEST --replace OLD_DIGEST
helm extension --scope project enable example --expected NEW_DIGEST
```

Replacement uses existing atomic catalog mutation and clears activation. Existing
package identity collisions cannot be overwritten without the exact old digest.
A failed or interrupted mutation must be inspected before retrying; do not assume
it left the old activation intact. Removing source files does not delete an
installed package; use the existing extension disable/remove commands.

The CLI operates on the machine where it runs. Connecting Helm to a remote Vessel
does not make these commands import local files into that remote host. Run them
on the executing host explicitly. No filesystem replication or credential transfer
is implied. The import adapter does not provide selective invocation or a separate runtime
executor. Automatic runtime discovery above provides on-demand reading of live
filesystem skills; it does not change installed package activation semantics.

Boost frontmatter supports the standard one-level string-valued `metadata` map
(e.g. `metadata: author: laravel`, expressed on indented YAML lines). These fields
are informational only; they neither authorize tools nor replace top-level name
and description. Other nested YAML remains unsupported and diagnostic.
