# Worktree consolidation, September 2026

Issue [#181](https://github.com/o-psi/voyage/issues/181) tracks review and retirement
of the historical secondary checkouts. Main remains the delivery branch. A branch
whose patch IDs differ from main is not necessarily missing functionality: older
integration commits often edited or combined the original patches.

## Review dispositions

| Work | Disposition |
| --- | --- |
| Inference controls, provider overrides and composer selectors (#177) | Delivered to main in `e8c32c0`, including the three feature worktrees and pending UI refinements. |
| Linux sandbox recovery (#62) | Ported the adapter and launch hooks from old Helm paths into Voyage in `6a37f50`, preserving explicit off/required modes. Broader issue acceptance remains open. |
| Voyage multiplexing draft | Superseded by `crates/voyage-protocol/src/process`, the public Vessel service contract, and Voyage command binding, receipts, decisions and lifecycle persistence. The draft launched a hidden Helm executor and must not replace the independent Voyage process. |
| Plain terminal branches (#32) | Production changes already integrated: `42ee87e` → `2dc183e`, `e2f60d2` → `e54a9fe`, `695dce0` → `3e5129e`. Current terminal helpers remain in `helm/src/plain_terminal.rs`. |
| GitHub branches (#68) | Production changes already integrated: `5cde3d9` → `5b3dcdd`, `2b996f4` → `52df147`, `349b069` → `ddcecf1`, `9b38f82` → `82771d3`. Current implementation lives in `voyage/src/github/`. |
| Dirty GitHub integration branches | Formatting, equivalent syntax changes and historical fixture maintenance. No missing production behavior identified. |
| MCP and inventory branches (#7) | Hardened transport and inventory changes already integrated, including `9bf926e` → `02ba0b9` and `322c75c` → `dd4df10`; current code lives in `voyage/src/tools/mcp.rs` and `voyage/src/agent.rs`. |
| Usage/history, title policy, recovery, CI, private-input and token branches | Clean ancestors of main or exact patch equivalents already in main. |
| Old test-only commits | Preserve history without restoring the general suites intentionally removed in `c3ccbff`. Test recreation remains separate work. |
| Detached quality checkouts | Preserve release packages and evidence, then retire their checkout registrations. |

The production mappings above are source/history review, not claims of exact patch
identity or fresh runtime coverage. Retiring a historical branch does not close
the broader issue it originally addressed.

## Preservation and concurrency

Before retirement, the local evidence directory
`.local-git/evidence/worktree-consolidation-181/` captures the inventory, binary
diffs, untracked source archives and a Git history bundle. Archive refs retain the
original heads. Ignored build caches, release artifacts and quality evidence are
retained separately from removed worktree source directories. No cache cleaning or
release-artifact deletion is part of this consolidation.

Changes from simultaneous work on main are preserved. Newly created active
worktrees are not stale merely because they appear during this inventory; their
owners must deliver through main before those checkouts can be retired.

The concurrent multimodal content contract and temporary installer checkout were
also incorporated in main and clean before retirement. The final inventory leaves
only the main checkout: 59 secondary checkouts were retired during the operation,
and 26 registrations for missing directories were pruned. Pre-existing deletions
of scripts/tests and the unrelated inspiration repositories in the main working
directory were preserved rather than silently staged or restored.

## Verification

The combined Helm, Vessel and Voyage source passed locked development check and
build, formatting, and a fresh inference smoke run against the final binaries.
The offline process checks verified exact retries, frozen active-turn settings, resumed
next-turn settings, invalid/stale refusals, clearing defaults and supervisor restart
persistence. No paid inference was used. Full interactive selector acceptance was
not exercised.

The sandbox port passed locked Voyage/Helm check and development build, local
`helm doctor`, and native Linux x86_64 bubblewrap probes. Probes covered pinned-root
rename/symlink replacement, filesystem/environment/descriptor restrictions, denied
socket families, read-only mounts, private temporary storage and selected inherited
limits, repeated launch and setup refusal, plus actual shell, managed-shell, search,
PTY and MCP launch paths. A setsid descendant stopped writing after its bwrap
parent was killed; this is bounded effect observation, not proof of observed
descendant disappearance. Full subagent/bridge/secret-workflow integration and the
broader cancellation/parent-death matrix remain unverified. No macOS, Windows,
other Linux architecture or complete sandbox security assurance is claimed.

The local evidence directory retains the detailed review reports and native probe
results separately from tracked source. These one-shot checks do not recreate the
removed general automated suites or close the remaining product acceptance scope.
