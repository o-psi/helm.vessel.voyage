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

This is first-release development. Helm chat/run/managed/workflow and outbound
worker adapters use Vessel-supervised independent `voyage` processes. Preserve
that boundary; do not reintroduce embedded executors. Vessel's enrollment relay
and scoped process gateway remain distinct authority surfaces.
Describe current behavior using [docs/current-state.md](docs/current-state.md)
and code; label target capabilities explicitly. Do not inject planned capabilities
into current runtime instructions. Browser console work remains deferred.

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
5. Update the issue with results and actual verification. Close only completed
   scope. Report blockers, remaining work and platform limitations accurately.

## Correctness and security

- Preserve exclusive session ownership, stable identity, exact command deduplication,
  atomic checkpoints and canonical text. Never claim a dead process survived restart
  or replay uncertain external effects automatically.
- The voyage process enforces local execution policy; Vessel routing and Helm
  presentation cannot broaden it. Preserve runtime authority checks and the
  outbound relay's transport-lease contract.
- Keep provider credentials on the executing machine. Native providers must remain
  independent of Codex; the optional compatibility bridge is distinct. Preserve
  credential/billing distinctions. Do not centralize credentials across Vessels.
- Apply local roots, command denials, approvals, cancellation and resource limits to
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
`./scripts/local-git` and do not initialize replacement metadata. See
[docs/local-git.md](docs/local-git.md). Use `--repo o-psi/voyage` with `gh` as needed.
Never reset, clean, force-push, stage unrelated files or overwrite concurrent edits
implicitly. Keep validation local rather than duplicating it in hosted CI.
Preserve existing release artifacts and follow [docs/releasing.md](docs/releasing.md).
