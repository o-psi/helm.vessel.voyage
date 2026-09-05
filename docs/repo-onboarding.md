# Repository onboarding

`helm onboard` generates reviewable project guidance from local files. It does not
call a provider, execute setup/build/test commands, install dependencies or modify
runtime policy. Existing project instructions take precedence.

```sh
helm --workspace /absolute/project onboard inspect --json
helm --workspace /absolute/project onboard preview
helm --workspace /absolute/project onboard preview --output guidance-draft.md --confirm
```

The report recognizes Rust, JavaScript/TypeScript, Python and Go manifests. It
includes source paths and SHA-256 digests, ecosystem and directory information,
confidence descriptions and fixed command argument lists. Every generated command
is **unverified**. A declared package script is evidence that a command exists,
not evidence that it is safe or successful. Read its implementation before running
it. `packageManager` can select npm, pnpm, yarn or bun; absent supported metadata,
JavaScript commands use npm as a candidate rather than assuming a verified setup.
Python test/lint commands are proposed only when their tool configuration exists.
No dependencies or setup commands are executed to improve confidence.

Discovery reads at most 512 directory entries through two nested directory levels,
with at most 64 project manifests and 256 KiB per input. It skips generated,
dependency and hidden directories, and does not follow directory or file symlinks.
Malformed, oversized or unsupported manifests produce review warnings without
copying their content. An overlarge workspace must be narrowed explicitly. Deeper
projects may exist: absence from this report is not proof that the repository has
no other projects.

Existing root/nested README, CONTRIBUTING and agent-guidance files are identified
with hashes. Their prose and arbitrary package script bodies are deliberately not
copied into generated instructions. Read the originals for project-specific rules.
The normal runtime's existing root `AGENTS.md`/`agents.md` precedence is unchanged.

Direct file creation and acceptance currently require Linux with anonymous-file
publication support. Inspection, stdout preview and diffs use portable directory
operations; macOS/Windows publication fails before writes until equivalent
descriptor-bound publication is implemented. Shell redirection remains an ordinary
operator action with the shell's own overwrite behavior.

Edit the generated draft using your editor. Review its exact contents and compute
its digest, then explicitly accept it into a **new** destination:

```sh
sha256sum guidance-draft.md
helm --workspace /absolute/project onboard accept \
  --draft guidance-draft.md --sha256 REVIEWED_SHA256 --output AGENTS.generated.md
```

An existing destination, including a dangling symlink, is never replaced. When no
`AGENTS.md` exists, you can explicitly choose that destination; otherwise keep a
sidecar and manually merge the reviewed guidance using your usual editing process.
A sidecar is not automatically loaded into model instructions. Edited content must
remain bounded UTF-8 without terminal controls. A digest mismatch rejects the
operation; it does not accept newly changed content. The digest confirms selected
bytes, not authorship, command verification or permission to broaden policy.
Read-only mode rejects both draft creation and acceptance.

Rerun discovery and inspect differences from a previous draft:

```sh
helm --workspace /absolute/project onboard preview --against guidance-draft.md
```

Preview does not change the earlier draft. To discard, do not accept it; remove only
your draft with your usual file-management tools. No project instructions are
modified merely by inspecting or previewing.

Storage uses pinned, directory-relative operations through the maintained
[cap-std filesystem API](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html)
and [no-follow extensions](https://docs.rs/cap-fs-ext/4.0.3/cap_fs_ext/trait.DirExt.html).
Paths must stay within the selected workspace, with existing parent directories.
Linux publication writes and syncs an anonymous mode-0600 staging file, then links
that open descriptor into the pinned destination directory with create-only
semantics and syncs the directory. No repository staging pathname can be replaced
or edited between digest confirmation and publication. Filesystems without
anonymous-file or descriptor-link support fail instead of falling back to an
in-place write. A crash before publication discards anonymous staging; after
publication the complete destination may exist despite a lost acknowledgement.
Inspect its digest before retrying; retries never replace it. No native power-loss
claim follows from Linux tests. These operations do not contain a hostile process
with the same OS authority accessing process descriptors or memory, and are not an
OS sandbox for agent subprocesses.

The Linux fixture covers actual inspect/preview/edit/accept/diff flows and existing
instruction preservation. Unit and failure tests cover multiple ecosystems,
malformed/untrusted input, bounds, links/FIFOs, read-only denial, exact digests,
concurrent create-only publication and process exit before/after publication.
Native macOS/Windows runs are not claimed or required development gates.
