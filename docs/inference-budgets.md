# Local inference allowances and usage

Helm can limit local inference attempts for a voyage or project. An attempt is a
permit committed before Helm calls its model transport. It is **not** a token,
dollar amount, tool turn, or proof that a provider received a request. Limits are
optional and default to unlimited. They do not reinstate arbitrary turn, child
count, or duration limits.

Use this workflow when you want a predictable bound on how often Helm may start
inference, including retries and delegated work. Token and monetary hard limits,
price estimates, machine resource telemetry, distributed budgets and historical
trend dashboards remain planned.

## Inspect, preview and configure

Select the project with `--workspace`. The first inspection or execution creates
a persistent project identity bound to that canonical workspace. Symlink aliases
resolve to the same identity. A session is bound to its project before its first
accounted run; that binding is immutable. Child agents inherit the initiating
session/project, including children using isolated Git worktrees. Starting a
separate local voyage in another workspace selects another project; this is not
a machine-wide or cross-Helm budget.

```sh
helm --workspace /path/to/project inference inspect
helm --workspace /path/to/project inference inspect --session SESSION_UUID
helm --workspace /path/to/project inference audit --after 0 --limit 100
```

All three commands emit structured JSON with `schema_version: 1`. Attempt and audit
pages use an exclusive `--after` sequence cursor and a maximum page size of 100.
Read subsequent pages using the last returned sequence. Inspect includes exact
local permit totals and independently available input/output token reports.
`null` means unavailable; reported zero is `0`. Reports are provider-reported,
not independently measured billing data. Failed or interrupted work can have
known input and unknown output. Never treat a missing report as zero spending.

The limit is a **cumulative ceiling since scope binding**, not additional credit.
For example, if inspection reports 6 consumed attempts, a limit of 10 permits
at most 4 more attempts. Both the project and session must have remaining credit.

Use a new UUID as `OPERATION_UUID` and the current inspected revision:

```sh
helm --workspace /path/to/project inference configure \
  --operation OPERATION_UUID --expected-revision 0 \
  --limit 10 --warning 8 --reason 'Allow ten attempts for this investigation'
```

This previews the change. Repeat the exact arguments with `--confirm` to commit.
For a narrower session ceiling, include `--session SESSION_UUID`. A saved session
which has not executed can be configured after its workspace is verified.

Inspection shows warning and hard thresholds. Helm records a warning once per
scope/revision when an admitted attempt reaches its warning threshold. The plain
interface and TUI display warnings; child warnings use the existing child progress
channel. `/inference` displays current local counts in either interactive interface.
Use the CLI to inspect full pages or preview configuration edits.

## Exhaustion and deliberate continuation

When another attempt cannot be admitted, Helm stops provider dispatch and reports
the local allowance failure. A run stopped this way is not successful completion.
Owned child shutdown and tool cleanup retain their ordinary evidence requirements.
You can inspect the saved conversation, run summary, attempt records and audit,
then deliberately approve more work:

```sh
helm --workspace /path/to/project inference configure \
  --operation NEW_OPERATION_UUID --expected-revision CURRENT_REVISION \
  --limit HIGHER_CUMULATIVE_CEILING \
  --reason 'Approve four more attempts after reviewing the saved result' --confirm
```

Removing the ceiling also requires an explicit reason and current revision:

```sh
helm --workspace /path/to/project inference configure \
  --operation NEW_OPERATION_UUID --expected-revision CURRENT_REVISION \
  --unlimited --reason 'Resume unlimited local work after operator review' --confirm
```

An exact retry with the same operation UUID and payload returns its immutable
receipt. Changing the payload under that UUID fails. If the revision is stale,
inspect current state before preparing a new operation. A busy store does not
implicitly retry mutations or authorize work: retry the identical operation after
other work settles. Do not erase the ledger to manufacture additional credit.
Keep credentials and sensitive task text out of operator reasons.

Configuration belongs to the local operator account. No budget mutation tool is
exposed to the model. Application policy is not an OS sandbox: an unrestricted
same-account shell can run programs or modify that account's files, including
administrative commands. These allowances do not claim to defend against such
private-state tampering or constrain other software.

## Dispatch, cancellation and recovery

The same admission boundary applies to root requests, nested children, follow-ups,
provider retries and automatic title inference. Model discovery is not inference
and does not consume a permit. The optional external compatibility bridge cannot
establish native request accounting; configured warning/hard allowances reject
its dispatch. Unconfigured bridge dispatch permits describe local adapter calls,
not the bridge's internal model requests.

Admission is transactional. A concurrent edit takes effect for attempts admitted
after its commit; it cannot recall an already admitted request. Cancellation is
checked before dispatch and authority is checked again after admission. A crash
or cancellation after permit commit but before sending can consume a permit
without sending anything. This conservative unknown-send record is not refunded.
A provider timeout, disconnect or error does not establish zero cost or server-side
cleanup. Retries require new permits. Resuming a session does not replay uncertain
tool effects or recover old credit.

Attempt outcome describes the locally observed provider interaction. A completed
provider response can belong to an interrupted or rejected run; it is not proof of
successful voyage completion. Unknown attempts stay unknown when execution was
interrupted without observing or durably recording their terminal provider event. Already reported
usage remains separately available even when later output is unknown.

Inference records live in the private local Helm data directory and contain
identifiers, provider/model labels, timestamps, purpose and accounting—not prompts,
provider credentials or PTY streams. `run --no-save` omits canonical conversation
saving; local attempt accounting still applies. Existing saved token sums have
unknown completeness and are not retroactively attributed to new attempt records.
No price, billing source or live spending estimate is inferred from those sums.

## Storage and availability

Records and database size are bounded. Existing immutable receipts and retained
attempt evidence are not silently pruned. When detailed capacity is reached,
scopes without a warning or hard limit can continue using exact durable counters;
inspection explicitly counts omitted attempt detail, whose token usage is
unavailable. A later ceiling applies to those same cumulative permit totals.
Configured scopes require durable detail and fail closed at that capacity.

Audit capacity reserves room for lowering limits or explicitly removing obsolete
configuration. If even that reserved space or the filesystem is unavailable,
mutation fails closed; a new authorization cannot be claimed without an audit.
Private storage corruption, unavailable current authority, or failure to persist
an admission stops dispatch. SQLite work runs outside the async reactor with a
bounded worker queue. Cancelling a waiting caller does not roll back work already
committed by its blocking worker.

The automated native-provider matrix uses local HTTP fixtures and persisted
accounting and conversation evidence. It does not demonstrate live model
performance, exact provider billing or cross-platform runtime validation. Native
Windows and macOS results must be reported separately from Linux checks.
