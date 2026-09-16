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
