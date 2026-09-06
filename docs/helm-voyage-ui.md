# Voyages in Helm

Every session is a voyage, including local chat. Start a local voyage with
`helm chat` or **Ctrl+N** / `/new [TITLE]` in the full-screen interface. No Vessel
connection, configuration draft or machine-selection wizard is required. Current
UI labels use “session” and “conversation” for these voyages.

## Start and resume a voyage

Helm shows recent conversations beside the active conversation when the terminal
display is wider than it is tall and has room for both panes. It uses terminal
pixel dimensions when available, otherwise estimates character cells as twice as
tall as they are wide. Below 64 columns, recent conversations use a drawer.

Press **Ctrl+S** to focus recent conversations, use the arrow keys and Enter to
open one, or click a row. Esc returns to the composer. Portrait displays expose
the same list as a drawer. Switching saves your unsent composer text and acquires
the destination session's execution ownership. Helm retains workspace runtimes and
their terminals across navigation, building a runtime on its first visit. Failed
navigation keeps the current voyage available. Finish or cancel active work
before switching. Opening an existing session resumes the same voyage; **Ctrl+N**
starts a new one. **Ctrl+B** branches into a new voyage. **F1** shows shortcuts.

Branching with **Ctrl+B** saves the latest unsent text on the source voyage and
copies it to the new branch. Both drafts are available after reopening; neither
is submitted to the model. **`/branch [TITLE]`** consumes its command and starts
with an empty draft in both voyages, so an older saved draft does not reappear.
If saving or creating the branch fails, Helm keeps the source open and retains
the input (including a failed `/branch` command) for retry. It does not switch or
retry automatically; if storage reports an uncertain result or failed rollback,
inspect the recent voyages before retrying. Branches receive a new identity;
resuming either voyage retains its existing identity.

## Optional Helm configuration drafts

Press **Ctrl+V**, or use **`/voyages`**, to open voyage setup. This feature saves
configuration drafts, separate from sessions. Saving a draft does not create a
local or remote voyage, change the active voyage's scope, start coordination, or
execute work. Drafts are not required for local chat. Connecting these choices to
the ordinary voyage workflow and executing across Helms remain planned.

1. Press **N** for a new draft. Enter a name and an optional purpose.
2. Select permitted Helms with **Space**. Type to search by label or full machine
   ID; use arrows to move. **F2** opens details for the focused Helm.
3. Choose a coordinating Helm from your selection with **Space**. Your interface
   Helm is not automatically a participant or coordinator.
4. Review the name, purpose, selected machines, coordinator and availability.
   Press **Enter** on the review screen to save the draft.

Enter or Tab advances through setup; Shift+Tab goes back. Esc returns to the
library with unfinished setup retained in the current window; **C** continues it.
Unfinished setup is not saved across process exit. Select a saved draft and press
Enter to edit it. Saved machine identities remain visible even when discovery is
unavailable. A previously observed Helm is not proof of current availability or
execution authority.

Local-only drafts can select **This Helm**, which is a stable local setup identity.
It is separate from Vessel enrollment. Runtime integration must explicitly resolve
that identity before execution; a draft grants no authority.

## Select Helms through Vessel

From the library, press **V** to connect. Enter the Vessel HTTPS origin and its
operator token, then press Enter. HTTP is accepted only for literal loopback
addresses, such as `http://127.0.0.1:9480`. URLs containing credentials, queries,
paths or fragments are rejected; redirects are not followed.

The token field is masked. Tokens stay in memory, outside composer input, model
context, conversation history and saved drafts. **D** disconnects and releases
the connection credentials; exiting Helm also releases them. **R** refreshes the
inventory. Network and authentication failures keep your draft intact.

Discovery reads the authenticated `/v1/diagnostics` inventory. Vessel currently
returns connected machine IDs, without friendly machine names or a complete
offline inventory. The picker therefore displays full IDs with observed connection
state. Presence alone does not establish voyage-runtime support. No discovery
operation submits work or imports a private conversation.

Drafts retain their original Vessel origin. Connecting to another Vessel does not
move a saved or unfinished draft there. Start a new draft for a different Vessel.
An originless saved draft stays local-only.

## Storage and recovery

A voyage uses its existing session store and identity. Resuming it preserves that
session; the terminology does not create a second storage object. Unsent composer
text belongs to that session and is distinct from the configuration drafts below.

Drafts are private local files under `voyage-drafts/` in Helm's data directory,
separate from conversation history. They contain the name, purpose, selected IDs,
coordinator, labels and optional Vessel origin. They contain no credentials or
execution grants. Names and purposes can still be sensitive; they are not sent to
Vessel by this setup flow.

Saves validate the complete selection and use atomic publication with revision
checks. Concurrent edits cannot silently overwrite each other. A failed save keeps
the form and its identity for retry. After a conflict, Ctrl+S saves retained edits
as a separate copy; the newer original remains intact. Starting another draft
asks before discarding unfinished setup. Malformed records and unsafe filesystem links produce an
error instead of being replaced. Linux storage and terminal tests cover this
workflow; native macOS and Windows validation is separate.

See the [voyage model](voyages.md) for the local workflow and planned cross-Helm
behavior, and
[dedicated remote sessions](remote-sessions.md) for the currently available remote
execution APIs.
