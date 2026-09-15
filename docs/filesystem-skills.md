# Filesystem skills (`SKILL.md`)

Helm can discover and import ordinary filesystem skills into Voyage's existing
format-1 declarative packages. You do not need to write a manifest or file hashes.
Discovery and import **do not activate instructions**. Activation is a separate
review of exact installed bytes and is applied on the next run.

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

Discovery searches **only immediate child directories** of these roots, in order:

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
is implied. This feature does not implement selective invocation/progressive
loading (#232) or a separate runtime executor.
