# Voyage titles

Voyage owns automatic naming; Vessel forwards canonical names and Helm displays
those names from refreshed snapshots. Automatic titles refresh at **Fibonacci
user-message counts: 1, 2, 3, 5, 8, 13, 21, …**. Every newly accepted user input
advances the count: initial submissions, follow-ups, workflow/operator submissions
and active steering. At a checkpoint, the refresh starts on acceptance, not run
completion. Exact command retries, model/tool iterations and rejected admissions
do not advance the count or request another title.

New voyages start with `session-` plus eight UUID hex digits. Until a title is
available, live Helm views show the first seven words of the first line of the
first available user message. Existing automatically named voyages keep their
current name until a user-message checkpoint. The count is durable and independent
of conversation compaction and run success/failure. Legacy states without a
user-message counter start at zero for subsequently accepted input; old completed-
turn counts are not converted or guessed from potentially compacted history.

## Intent and timing

The utility prompt asks for **3–6 words describing what the user wants to
accomplish**, not assistant actions, progress or outcomes. It uses recent user
messages only, with the latest input refining the overall goal. Responses have a
hard limit of **48 Unicode characters**; the word count is prompt guidance rather
than a validator requirement. Assistant/tool exchanges are excluded.

After agent construction and run-scope preparation, a borrowed, run-owned worker
runs title inference alongside execution, not after successful completion. Queued
steering at a Fibonacci checkpoint is included in the title-only projection upon
acceptance, even while the main provider or a tool is busy. This does not mark steering as
applied or append it to canonical history. Operator-tool execution uses the same
worker. If construction fails or cancellation wins before inference, naming is
best-effort and the old name remains.

Only one title request runs at a time. New input supersedes older requests: an
obsolete result cannot overwrite newer intent. Bursts coalesce to the latest
eligible input rather than queueing unbounded utility work. A non-checkpoint input
invalidates an older in-flight result but does not start a replacement request.
After execution ends,
the worker finishes any in-flight request and can drain the newest input once;
it does not keep a finished run alive indefinitely. Cancellation stops utility
work. There are no detached title tasks or automatic restart retries.

The build-time model is selected in [`utility-models.json`](../voyage/utility-models.json).
It must be available through the executing voyage's provider. Requests use that
provider's credentials, policy and inference accounting, not a Vessel-owned
provider. The helper redacts before bounding recent user text to 6,000 characters
(at most 1,500 per message), and rejects empty, oversized or control-containing
responses. The request has a maximum ten-second timeout, also bounded by the
configured tool-context timeout, and responds to cancellation.

Missing models, provider failures, invalid responses and timeouts leave the
existing name intact. The next Fibonacci user-message checkpoint is the next
attempt; a failed request never shifts the schedule to a non-checkpoint. Utility
usage remains separate from conversation usage. Title metadata is committed
independently of the conversation checkpoint and terminal result; a later failed
or cancelled run does not undo an already generated name. Failed metadata writes
and terminal-write retries never repeat provider inference.

Manual rename disables automatic updates, including an in-flight generated title.
Clearing conversation resets the user-message count and pending title identity,
and restores the UUID fallback
only for automatic names. Unnamed branches initialize automatic naming against
their final branch identity; explicitly named branches stay manual. Moving the
same voyage to another Vessel preserves its title state. Archived Helm views
prefer final archive metadata over cached live snapshots. The 48-character bound
is for newly generated titles, not a restriction on existing or manual names.

## Verification and limits

Focused offline Rust tests cover title updates during an actual run with a blocked
main provider, Fibonacci counting across submissions and queued steering,
non-checkpoint suppression, exact retry deduplication, newer-input protection,
failure and cancellation, user-only redacted prompts, response bounds, manual
names, counter persistence through history replacement/reload/reset, and legacy
state loading. No live-provider title-quality evaluation or native
macOS/Windows validation is implied by those checks.

Vessel's catalogue name remains a last-observed cache: a canonical snapshot repairs
it, but catalogue-only consumers need not immediately see a newly generated title.
The live Helm path already fetches canonical snapshots after observations. Running
processes retain their loaded executable; source changes alone do not upgrade an
already running voyage or Helm instance.
