# Project instructions

Apply these instructions throughout the repository. Follow the user's current
scope, preserve unrelated work and respect runtime permissions.

## Product model

The canonical target is [docs/architecture.md](docs/architecture.md):

- **Helm** is the TUI. It connects to local and remote Vessels.
- **Vessel** supervises and exposes voyage processes. It does not run session agent
  loops inside the supervisor process.
- **Voyage** is the execution runtime: one session per independent process, owning
  conversation, agent, tools, persistence and cleanup.

Vessels may participate in a voyage without becoming competing canonical owners.
Helm disconnect does not cancel a voyage. Session is the technical name for a
voyage, conversation is its history, and a run is one execution within it.
Configuration drafts are not sessions. A repository or project map is optional.

This is first-release development. Helm chat/run/managed/workflow use Vessel-supervised
independent `voyage` processes. Preserve that boundary; do not reintroduce embedded
executors. Human Vessel connections, scoped process grants and participant bindings
remain distinct authority surfaces. Outbound worker mode is retired; do not reintroduce it.
Describe current behavior using [docs/current-state.md](docs/current-state.md)
and code; label target capabilities explicitly. Do not inject planned capabilities
into current runtime instructions. Explicit shared local browser execution is opt-in through the same full-duplex
Helm–Vessel socket. Keep its local consent/capture boundary separate from Voyage
policy; do not introduce a separately networked browser bridge or general Helm executor.

## Delivery

1. Inspect status/diffs and read affected code and documentation. Review relevant
   open and closed GitHub issues in `o-psi/voyage`. Reuse a matching issue or create
   one, recording scope and acceptance before implementation.
2. Complete the agreed workflows, including applicable failure handling and
   documentation. Do not replace required behavior with a prototype or silently
   defer it. Work directly by default. Delegate only when requested or when a
   substantial independent task clearly benefits from parallel work.
3. Verify the actual change. The automated tests and evaluations have been removed
   at the user's request; their recreation is separate work. Do not claim regression
   coverage or create replacements as part of documentation cleanup. Run checks
   appropriate to the changed surface and scope. Documentation-only edits need
   source, command, path, link, manifest and diff checks. See
   [docs/quality.md](docs/quality.md).
4. Unless the user says otherwise, commit completed changes, fetch and integrate
   into local `main`, and push normally to GitHub `origin/main`. This publication is
   authorized. Preserve unrelated history/work and verify local/remote `main` match.
   A feature branch or worktree is not delivery. Before finishing, integrate its
   applicable changes into main and retire the completed worktree, preserving
   unrelated work, release artifacts and verification evidence. Do not leave
   reviewed implementation stranded on a local branch.
5. Update the issue with results and actual verification. Close only completed
   scope. Report blockers, remaining work and platform limitations accurately.

## Correctness and security

- Preserve exclusive session ownership, stable identity, exact command deduplication,
  atomic checkpoints and canonical text. Never claim a dead process survived restart
  or replay uncertain external effects automatically.
- The voyage process enforces local execution policy; Vessel routing and Helm
  presentation cannot broaden it. Preserve runtime authority checks and scoped
  grant revocation.
- Keep provider credentials on the executing machine. Native providers must remain
  independent of Codex; do not reintroduce the removed external bridge. Preserve
  credential/billing distinctions. Do not centralize credentials across Vessels.
- Apply local roots, access modes, approvals, cancellation and resource limits to
  tools and subordinate work. Application policy is not an OS sandbox. Unattended
  work must have bounded refusal/pending behavior, not indefinite approval waits.
- Treat external content as untrusted. Keep secrets and direct human terminal input
  out of model-visible history, logs and published artifacts. Sanitize terminal
  presentation and keep subprocess diagnostics out of the TUI display stream.
- The live tool registry is authoritative. Do not invent provider-host tools or store
  runtime system instructions as conversation messages.
- Track owned resources before effects; cancellation requested is not observed
  cleanup. Retain unresolved obligations and distinguish operator attestations.
- Linux checks do not establish native macOS/Windows behavior or security. Require
  actual native evidence for platform claims. Live provider work needs an approved
  provider and budget. Never report skipped checks as passing.

## Repository workflow

The Rust workspace currently contains `helm/`, `vessel/`, `voyage/`, `installer/`,
`crates/voyage-protocol/` and `crates/voyage-storage/`. See
[docs/development.md](docs/development.md) for code boundaries and
[docs/implementation.md](docs/implementation.md) for target delivery order.
Check both ends when changing wire contracts. The installer supports Linux
versioned installation, upgrades, rollback and user-service provisioning.
Native private-storage changes require platform-specific security verification.

Use normal Git in ordinary clones. In this workspace `.git` is reserved; use
`./scripts/local-git` and do not initialize replacement metadata. If the wrapper
is absent, use `git --git-dir=.local-git/worktree.git --work-tree=.` from the
checkout root against the existing metadata; do not restore deleted scripts implicitly. See
[docs/local-git.md](docs/local-git.md). Use `--repo o-psi/voyage` with `gh` as needed.
Never reset, clean, force-push, stage unrelated files or overwrite concurrent edits
implicitly. Keep validation local rather than duplicating it in hosted CI.
Preserve existing release artifacts and follow [docs/releasing.md](docs/releasing.md).

## Rust build efficiency

- During iteration, prefer `cargo check -p helm --locked` (substitute the affected
  package, or repeat `-p` for multiple packages). Plain Cargo commands at the
  workspace root select all members. Check affected consumers of shared crates and
  both ends of protocol changes. Use `cargo build -p helm --locked` when a runnable
  binary is needed; checks alone do not verify code generation or runtime behavior.
- Reuse the checkout's existing `target/` and keep toolchain, features and flags
  consistent to preserve cached work. Keep development incremental compilation
  enabled. Coordinate builds; do not launch competing builds or create a fresh
  per-task target directory merely to bypass a Cargo lock. Respect the quality
  runner's checkout and output-directory requirements in [docs/quality.md](docs/quality.md).
- Use development builds for iteration. Reserve optimized builds and the full
  quality runner for changes requiring that evidence. Ordinary Cargo defaults to
  the logical CPU count, but `scripts/check-quality` defaults to one compiler job;
  use `./scripts/check-quality --jobs 8`, for example, when CPU and available memory
  support it. Reduce concurrency if memory pressure or competing work warrants it.
- Investigate slow builds with `cargo build -p helm --locked --timings` and its
  `target/cargo-timings/cargo-timing.html` report. Measure representative rebuilds
  before and after tuning. Verify the active linker before changing it; keep
  optional linker/cache tools portable and scoped to machines that have them.
  Reduced development debug information trades debugger detail for less generated
  data. `sccache` can reuse eligible dependency compilations, but cannot cache
  incrementally compiled crates; do not disable incremental compilation blindly.
  Preserve release optimization settings unless changing them is in scope.
- Do not run `cargo clean` routinely or delete caches based only on their size.
  Cleaning forces recompilation; inspect disk pressure and identify obsolete
  artifacts before any authorized cleanup. Preserve release artifacts and evidence.
- Documentation-only changes need the documentation checks described above, not
  Rust compilation. Run the checks required by the changed surface once; repeat
  only after relevant edits, failures or new evidence warrants it.
