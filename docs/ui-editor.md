# Helm composer editing

New and existing voyage composers edit Unicode grapheme clusters, including combining
marks and emoji joined with ZWJ, as one unit. Owned image markers remain atomic UUID
references; typing a literal `[Image 1]` still produces ordinary text.

Shift with arrow, Home or End extends a visible selection. Typing and terminal paste
replace it. Ctrl+A selects the draft. Ctrl/Alt+Left and Right move across
whitespace-delimited words; Ctrl/Alt+Backspace and Delete remove words. Up/Down move
between wrapped display rows in multiline or wrapped drafts. Alt+Up/Down recall
prompt history in existing voyages; unmodified Up/Down retain recall in a one-row
prompt. New drafts have no submitted-prompt history. Existing global shortcuts,
Tab completion/view switching and Enter send versus Alt/Shift+Enter newline remain.
Ctrl+Home/End remain transcript navigation in existing voyages; in a new draft they
move to the beginning/end of its text.

Ctrl+X cuts a selection into a private, in-memory text buffer. Alt+Y inserts that
text; this does not write or read the operating-system clipboard. Owned images are
excluded from the cut buffer. Ctrl+V and Shift+Insert retain explicit clipboard
acquisition, including image paste. Plain bracketed text paste uses the terminal
payload. Acquisition remembers the selected range and rebases it while typing;
failed or cancelled acquisition preserves the draft. Undo during acquisition
invalidates its destination so the result is rejected, asking the operator to
paste again. Completed selection replacement synchronizes image ownership before
persisting the draft; save failure preserves the prior paste draft.

Ctrl+Z undoes and Ctrl+Y or Ctrl+Shift+Z redoes text edits. History is local to the
current draft state, bounded to 64 snapshots. Undo capture keeps at most 1 MiB of text; moving through
undo/redo can additionally retain one current draft (up to 64 KiB). History is not persisted.
Replacing/restoring a draft, sending, and inserting or deleting an owned image
clear editing history. This attachment retention policy releases deleted image
bytes immediately and prevents undo from resurrecting labels without their UUID
attachments. Text edits around unchanged images remain undoable. Selection, word
editing and paste cannot split an owned marker or turn a literal label into an
attachment. Draft recovery and submitted-prompt history remain separate mechanisms.

## Dependency decision

The published [ratatui-textarea 0.9.2 source](https://docs.rs/crate/ratatui-textarea/0.9.2/source/)
was downloaded and an attachment-free executable exercised insertion, undo and
redo successfully. The same executable showed Backspace converting `e` plus a
combining accent to bare `e`, and a woman/ZWJ/laptop sequence to woman plus a
trailing ZWJ. Its cursor/edit API counts Unicode scalar values. Using it for Helm's
composer would require overriding its fundamental editing behavior while also
maintaining UUID spans and asynchronous anchors. Helm retains its coordinated
editor and existing unicode-segmentation dependency for this surface.

Published metadata declares MIT, Rust 1.86, default Crossterm integration,
ratatui-core/ratatui-crossterm 0.1.1 and ratatui-widgets 0.3.1 compatible requirements.
The temporary attachment-free probe disabled default features and resolved
ratatui-core 0.1.2 / widgets 0.3.2 locally. No editor dependency, extra terminal
backend, native library or clipboard integration was added to Helm.

## Verification and limits

A temporary executable compiled the actual composer source against cached checkout
dependencies and exercised combining/ZWJ/wide characters, marker boundaries next
to combining text, literal labels, selection replacement, cut/yank, word deletion,
text undo, image history barriers, delayed-paste rebasing and invalidation, prompt
history and wide-character wrapped motion. It passed on Linux. This is targeted
executable evidence, not a recreated regression framework or native macOS/Windows
certification. The existing scalar-editing unit check was updated for the new
grapheme behavior.

On this Linux machine, an unoptimized executable using an approximately 64-KiB draft
performed 200 insert/delete operations at the end of the draft in 0.917 ms and
200 wrapped vertical movements in 4.399 seconds (about 22 ms each). Vertical motion scans the bounded draft;
this measurement excludes terminal rendering and provider work. Existing composer
presentation hard-wraps styled graphemes before drawing, preserving selection
styles. Terminal-specific key reporting and emoji cell widths can differ.
