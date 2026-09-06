# Explore historical local inference usage

Use historical usage to find which voyages, models or agents consumed local
inference permits and which token reports are available. It reads the same private
ledger as [inference allowances](inference-budgets.md), without calling a provider,
changing a limit or consuming a permit. The result is local accounting, not billing.

## Select a range and compare groups

```sh
helm --workspace /path/to/project inference history \
  --from 2026-01-01T00:00:00Z --until 2026-01-08T00:00:00Z \
  --group-by model
```

Both boundaries must be explicit UTC RFC3339 timestamps ending in `Z` or `+00:00`.
The range includes `from` and excludes `until`. An attempt belongs to its admission
time, even when its final report arrives later. The query observes its current
retained report, not a reconstruction of what was known at an earlier date.
Admission and finish times are separate wall-clock observations. A clock correction
can reverse their apparent order; both are preserved without inferring a duration.

Choose `--group-by session`, `model`, `agent`, `purpose` or `day`. A model group
includes both provider and model identity. An agent group with `id: null` is root
work; child groups retain their actual agent IDs. Purpose distinguishes conversation
from automatic title inference. Daily trends use UTC calendar days. Days with no
retained records have no group; omitted detail can prevent concluding that a
missing day had no activity.

Add `--session SESSION_UUID` to restrict the query to a voyage already bound to the
selected project. A UUID from another project is refused. Project identity and
child worktree attribution follow the existing immutable accounting binding.
Saved conversations without a ledger binding have no attributed history to query.
Project queries can be inspected before the first inference attempt.

Each JSON result has `schema_version: 1`, the normalized query, scope, snapshot
identity, totals, groups or attempts, coverage information and a nullable `next`
cursor. The schema uses explicit fields rather than formatted totals:

- `totals.retained_attempts` counts matching retained permits. `completed`, `failed`
  and `unknown` classify the locally observed attempt outcome. A completed provider
  attempt does not establish successful voyage completion or provider billing.
- Input and output have independent `reported_sum`, `reported_attempts` and
  `missing_attempts`. A reported zero is `0`. If no matching attempt supplied that
  side, its sum is `null`; missing reports are never added as zero.
- `scope_lifetime_permits` and `scope_lifetime_omitted_details` describe the entire
  selected scope, including activity outside the time range.
- `range_permits` is exact only when the scope has no omitted details. Otherwise it
  is `null`: omitted records cannot be assigned to a time, model, child or outcome.
  The retained totals remain useful but are incomplete for that question.

Legacy saved token sums are not retroactively attributed to attempts. Unknown-send
permits remain consumed; historical inspection never refunds them. Prices, billed
cost, machine resource use and cross-Helm totals are not inferred.

## Page and drill down

Groups have a typed `key` and their own totals. Groups sort by their typed key;
attempt detail sorts by increasing durable sequence. Pages default to 50 entries
and accept `--limit 1` through `--limit 100`.

For another page, retain the original range, grouping and optional group filter,
then pass the exact `next.snapshot` and `next.offset`:

```sh
helm --workspace /path/to/project inference history \
  --from 2026-01-01T00:00:00Z --until 2026-01-08T00:00:00Z \
  --group-by model --limit 50 --snapshot SNAPSHOT_SHA --offset 50
```

`--offset` without a snapshot is refused. Each query reads counters and records
from one SQLite transaction. If an admission, report, outcome or relevant scope
configuration changes the snapshot, a continuation fails. Refresh from offset zero
without `--snapshot`; do not combine pages from the previous observation.

To inspect the retained attempts behind a group, copy its exact JSON key into
`--group`. Start the drill-down as a fresh query:

```sh
helm --workspace /path/to/project inference history \
  --from 2026-01-01T00:00:00Z --until 2026-01-08T00:00:00Z \
  --group-by model --group '{"kind":"model","provider":"openai-responses","model":"MODEL_ID"}'
```

Use the actual provider/model labels from the group result. A session key has
`{"kind":"session","id":"SESSION_UUID"}`; the other key forms are returned by
their respective group queries. Drill-down pages contain attempt, project, voyage,
run and agent identities, timestamps, provider/model, purpose, outcome and both
optional token reports. They contain no prompt or terminal text.

## Explore in the TUI

`/inference history` opens the preceding seven days, ending at the instant the
panel opened. To choose an exact range, use the same flags as the CLI:

```text
/inference history --from 2026-01-01T00:00:00Z --until 2026-01-08T00:00:00Z --group-by day
```

The panel starts with the project. Add `--session` with the active voyage UUID to
start there instead. Another voyage's UUID belongs in the standalone CLI.

Use these keys inside the panel:

- `1` session, `2` provider/model, `3` agent, `4` purpose, `5` daily trend.
- `s` switches between the project and current voyage.
- Up/Down selects a group; Enter opens its retained attempts. Backspace returns to
  groups. Page Up/Page Down, Home and End scroll longer output.
- `n`/`p` fetch the next/previous page of the same snapshot. `r` refreshes the
  current range; it does not move the range's end to the current time.
- Esc or Ctrl+C closes the panel and cancels its wait. Reopen it to choose a new
  range or return to the default preceding seven days.

Grouping, scope changes and drill-down start fresh observations. The existing
composer stays intact, paste cannot submit work from the panel, and results remain
outside canonical conversation. Close the panel before starting a model run or
switching voyages. A late result from a closed panel or previous voyage cannot
replace the current observation. Ordinary `/inference` still shows current
allowance status.

## Availability and privacy

Queries use bounded blocking workers and a ten-second caller wait. Cancellation
stops waiting promptly and is checked during scanning. It cannot undo an identity
binding already committed before inspection. No inference admission occurs.

Malformed records, inconsistent identity/counts, unsupported schema, overflowing
token sums and unavailable storage fail explicitly; partial totals are not
presented as complete. Busy state can be retried after other work settles. Existing
ledger detail and physical storage limits remain unchanged; queries do not prune,
rewrite or repair evidence.

Provider/model labels and identifiers may be sensitive. If configured secrets
appear in the projection, Helm refuses to display it rather than silently changing
an immutable group key. Keep exported JSON private. This is local operator data,
not a new model tool or a notification/remote-sharing capability.

Automated coverage uses synthetic ledger records and local CLI/TUI components.
Linux checks do not establish native macOS/Windows behavior or live provider billing.
