# Voyage titles

Voyage owns automatic naming; Vessel forwards canonical names and Helm displays
those names from refreshed snapshots. Title generation runs after a successful
ordinary agent turn, before its terminal journal commit. Operator-tool actions,
failed, cancelled, interrupted and incomplete turns do not advance the schedule.

New voyages start with `session-` plus eight UUID hex digits. Until a title is
available, live Helm views show the first seven words of the first line of the
first available user message. Successful turns 1, 2, 3, 5, 8, 13, … request a
replacement title. Turn counts persist independently of conversation compaction.
Existing voyages with untouched automatic naming state start advancing that
stored count when run by the updated runtime; older history is not back-counted.

The build-time model is selected in [`utility-models.json`](../voyage/utility-models.json).
It must be available through the executing voyage's provider. Requests use that
provider's credentials, policy and inference accounting, not a Vessel-owned
provider. The helper redacts and bounds recent user/assistant text to 6,000
characters (at most 1,500 per message), excludes tool exchanges, and accepts only
a single title of at most 80 characters without terminal controls. The request
has a maximum ten-second timeout, also bounded by the configured tool-context
timeout, and responds to cancellation.

Missing models, provider failures, invalid responses and timeouts leave the
existing name intact. Successful conversation turns still advance the schedule;
the next Fibonacci checkpoint is the next attempt. Utility usage remains separate
from conversation usage. Title generation is not repeated during terminal-write
retries. Terminal status, title and completed-turn count are committed atomically;
a cancellation or incomplete-cleanup decision in that transaction prevents a
successful title update. A failed transaction does not advance the count.

Manual rename disables automatic updates, including an in-flight generated title.
Unnamed branches initialize a new schedule against their final branch identity;
explicitly named branches stay manual. Moving the same voyage to another Vessel
preserves its title state. Archived Helm views prefer final archive metadata over
cached live snapshots. Vessel accepts the runtime's 512-byte name bound, including
80-character Unicode titles.

## Verification and limits

Local offline journal probes checked Fibonacci scheduling, title persistence after
reopen, immutable terminal replay, manual override and cancellation. These were
temporary verification probes, not a reinstated automated suite. Compilation and
build checks cover Voyage, Vessel and Helm. No live-provider request or native
macOS/Windows validation was performed for this wiring change.

Vessel's catalogue name remains a last-observed cache: a canonical snapshot repairs
it, but catalogue-only consumers need not immediately see a newly generated title.
The live Helm path already fetches canonical snapshots after observations. Running
processes retain their loaded executable; source changes alone do not upgrade an
already running voyage or Helm instance.
