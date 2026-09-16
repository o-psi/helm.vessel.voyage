# Web voyage actions

Each voyage has a Flux context menu, reachable by right-click, the visible actions
button, or the keyboard context-menu key / Shift+F10. Actions target that voyage,
not whichever conversation happens to be selected in the main pane.

The menu follows Helm TUI's sidebar actions: details, access mode, rename,
archive/restore, branch, cancel current run, clear conversation, compact older
messages and permanent deletion. Context controls are actual Flux menu items;
review and confirmation use a Flux modal. Unavailable actions explain their
reason rather than dispatching speculative commands.

Opening an action reads current Vessel capabilities, process identity and where
available the voyage snapshot. Dispatch rechecks identity and revision. The owner
still enforces rights, lifecycle, cleanup and execution policy; browser visibility
is not authorization. Access changes and branching require full owner connection
authority. Other lifecycle operations retain the existing scoped-grant rights.

Destructive actions require explicit review and confirmation. Compact preserves
canonical history and affects the working context. Clear removes current messages
and continuation, not the voyage identity or prior admission evidence. Permanent
delete is not a forensic-erasure guarantee. Cancellation requested is not observed
cleanup. Branching copies canonical history into a new independent owner without
starting inference; subsequent use can incur provider charges.

Before dispatch, the client records operation identity and target metadata in its
connection-scoped intent journal. Lost replies are reconciled by receipts and
observed owner state, never automatic mutation replay. Restore may require
restarting a positively stopped archived owner before clearing its archive flag;
an unavailable owner is not assumed stopped. A branch snapshot receipt alone does
not prove the new child was started. Pending outcomes remain explicit.

This UI does not add terminal execution, arbitrary tools, provider credential
access or a general browser executor. The direct-browser allowlist exposes only
the existing typed operations. Server-side grant checks remain authoritative.

## Verification

Focused rendered Flux DOM checks cover all actions, clicked-voyage identity,
revision/selection races, typed confirmations, stopped/live archived restore,
branch boundaries and lost-reply reconciliation without replay. Integrated Web
checks: 49 passed; gateway compatibility: 23 passed. Focused backend sidebar tests:
four passed. Cargo check, Clippy (warnings denied), formatting and Vite build pass.
Workspace coverage after the final Rust edits: 1,677 passed, zero failed, one
ignored; lines 77,098/101,783 (75.7474%), up from 75.4670%. This measurement also
completes verification of the pending #313 draft-persistence removal; scope was
not narrowed to improve coverage. Current 16 executable objects include all
workspace packages, with source identity checked. See `coverage/latest.json`.
Browser screenshot/mobile interaction and installed-backend rollout need separate
evidence; fixture tests do not establish deployed authorization behavior.
