# September 2026 checkout consolidation

Historical audit of the named consolidation below. Capability limits and checkout
counts describe that checkpoint, not the current product or workspace. See the
[current architecture](architecture.md), [voyage design](voyages.md) and subsequent
[worktree audit](worktree-cleanup.md).

The primary working tree had remained at `486c453` while published improvements
were built in isolated checkouts. Rebuilding that tree reintroduced the old tool
renderer. This consolidation merges published `origin/main` through `5c168e3`
with accumulated local work and recovers the remaining unique source.

Tracking: [worktree integration #46](https://github.com/o-psi/voyage/issues/46).

## Inventory and disposition

The inventory contained 22 existing checkouts and two stale registrations.
All committed references and non-generated source snapshots were backed up locally
before integration. Backups include dirty and untracked source; private local
runtime state, backups and build products are excluded from the Git publication.

| Checkouts | Disposition |
| --- | --- |
| Primary `voyage` | Preserved accumulated runtime, steering, attachment and documentation work. |
| `voyage-tool-output` | Merged published `5c168e3`, including multiline previews, Ctrl+O details and regression fixtures. |
| `local-session-ownership-*` | Integrated `5704902`, with session-path and portability fixes. |
| `attachment-enrollment-client-*` | Imported client and tests, wired its protocol dependency and module. |
| `agents-loader-*` | Already incorporated; primary includes later hardening. |
| `enrollment-proof-*`, `owner-sharing-policy-*` | Earlier source incorporated; primary retains later validation and lifecycle improvements. |
| `inline-question-docs-*`, `inline-questions-*`, `questions-80`, `inline-questions-80` | Represented by published inline questions plus primary steering changes. |
| `helm-runtime`, `helm-ui`, `helm-serve`, `vessel`, `quality-release`, `security-observability` | Previously merged or equivalent commits; no old implementation reapplied. |
| `attachment-contract-*`, `attachment-service-lifecycle-*`, `vessel-outbound-broker-*`, `workspace-loader-*` | Clean checkouts at the original primary HEAD. |
| `voyage-resource-fixture` | Its resource fixture is identical to the primary copy. |
| Missing `voyage-all-features`, `voyage-archive-push` | Stale registrations; commits are already represented. |

## Preserved limits

This integration does not declare the unfinished attachment or completion epics
complete. Enrollment remains a library foundation, and remote execution is not
exposed. The JSON session store now guards revision writes with execution leases;
CLI/TUI execution does not yet hold a lease across the whole model/tool run.
Foreign-store JSON paths are rejected before execution; resume by UUID, title or
a UUID-named path within the configured session store. No implicit import occurs.
Unix directory entries are synced after session replacement; non-Unix platforms
sync file contents without promising directory-entry crash durability.

## Validation

The consolidated Linux tree passed:

- `cargo fmt --all -- --check` and strict workspace/all-target/all-feature Clippy.
- `cargo test --workspace --all-features`: 341 tests, no failures.
- `cargo build --workspace --release --locked`.
- All seven offline system fixtures: native provider independence, unlimited turns,
  questions, tool output, subagent archive, subagent resources and Vessel lifecycle.
- `python3 eval/run.py validate`: 11 scenario definitions across eight categories.
- Release packaging and SHA-256 verification.

Initial enrollment fixtures used directories without the required private Unix
permissions; the fixtures now provision mode 0700. Production validation remains
strict. Earlier sandboxed socket failures were rerun with local network access.
The tool-output fixture exercises multiline previews, failed exits, Ctrl+O
expansion/collapse, resize, canonical persistence and terminal restoration.

Twenty-one redundant checkouts were removed after comparing their source with
verified backups; two stale registrations were pruned. The primary is the only
remaining registered checkout. Original branches remain recoverable.

Linux results do not establish Windows/macOS or live-provider validation. Eval
validation checks definitions, not live model behavior.
