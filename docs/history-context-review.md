# History and context review (#272 U03, U07, U11)

## Find and read older messages

Ctrl+F searches **loaded rendered display lines**, not the entire saved
conversation. Hidden tool details and unloaded messages are excluded; expand
activity or load older messages to include them. Matching is case-insensitive;
a phrase split across rendered lines is not a single match. Enter finds the next
match, Shift+Enter the previous, wrapping within the loaded display. Escape closes
Find. Ctrl+Home loads an earlier page without closing Find or intentionally
moving the current reading anchor. Press Enter again to search newly loaded text.
The existing history loader remains revision-bound, limited to 64 MiB and timed
out after 30 seconds; retries do not imply a full-history search succeeded.

Outside Find, Ctrl+Up/Down jumps to the previous/next loaded user-message heading.
At the earliest loaded user message, Ctrl+Up requests older history; use it again
after loading to navigate those messages. Ctrl+End returns to latest output.
Navigation keys do not modify canonical history, select branch cutoffs or edit
files. Content-key/offset anchors are retained across older-page loading and
reflow; new live output does not intentionally snap a historical reader to latest.

## Keep N review

Keep N excludes the newest N non-system messages from **this reduction pass**.
It does not mean only N messages remain, nor does increasing N undo a prior
reduction. Older user text, including task and steering, remains in provider
context. Eligible older assistant/tool text can become extractive excerpts and
canonical references; complete tool groups stay valid. This is not generated
summarization. The canonical history remains saved and readable, and files are
not changed. Reduced working context persists across restart and affects
subsequent provider requests. No exact reduction count is available before the
backend operation; the receipt reports the actual result, including zero.

## Selected historical branch review

`/branch [name]` and sidebar Branch open a read-only preview. Ctrl+Up/Down
chooses a loaded saved user message or the explicit full-history option. A user
message at the transcript read anchor is initially selected; otherwise full
history is shown. Load older pages before opening the review to reach earlier
points. The preview names source/destination host, source and new voyage UUIDs,
reviewed revision, inclusive boundary, retained/omitted counts and bounded selected
text. Unsent composer text is not saved history. PageUp/Down scrolls the review. Enter confirms; Esc cancels.

Both Vessel and owner runtime Branch commands accept optional `through_message`:
an inclusive, zero-based **canonical saved message index**, required to name a
user message. Omission retains the prior full-history contract and serialization.
A historical request is never downgraded to full history for an older peer.
The owner validates every retained tool group: assistant-only calls, unique nonempty
IDs, exactly one matching tool result per call, no orphan results and no interleaved
or incomplete groups. It rejects an invalid point, stale revision, active run or
pending cleanup. Existing authenticated authority, incarnation, expiry and exact
command-ID payload fencing still apply. The UI uses current canonical snapshot message indices or hydrated pages matching
the snapshot revision, never stale cached pages, and rechecks the frozen review
before sending.

The immutable owner snapshot contains only the selected canonical prefix, not a
truncated display over full provider context. Historical branches reset working
reductions and provider continuation; full-history branches retain compatible
working reductions. The destination imports a new session identity with source
provenance and no live run state. Branch creation itself does not run a provider
or replay tools. Source canonical messages are unchanged (the lifecycle receipt
advances source revision).

The new voyage uses the same host/workspace: files are not rolled back, copied
or isolated, so later edits can affect shared files. Existing attachment-blob import
is unchanged; this is not repository/workspace copying or restoration.

## Keyboard disclosure inspection

Ctrl+Shift+Up/Down moves the read anchor between activity/tool disclosure headers.
Ctrl+Space toggles the anchored tool, reasoning disclosure or activity group.
Reasoning uses the same `Key::Tool` identity as tool disclosure presentation;
ordinary Enter remains the composer/send key. No prompt-history Alt shortcuts are
reassigned. Mouse double-click and Ctrl+T group controls remain available.

## Verification boundary

Focused tests cover canonical cutoff snapshot/import, exact command retry/conflict,
stale revision rejection, malformed tool groups, optional wire compatibility,
keyboard disclosures, historical point selection and review disclosures, search direction/wrap/Unicode,
canonical user navigation, coordinated-user headings, anchor rewrap/prepend and
older-load retry behavior. These are not native terminal or live-provider journey
evidence. Parent integration owns shared review/help/command wiring, serialized
Cargo checks, workspace coverage and publication.
