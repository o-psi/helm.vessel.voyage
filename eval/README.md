# Helm evaluations

The suite exercises coding, system-administration diagnosis, local research,
professional writing, data analysis, interruption-safe checkpointing, and parallel
delegation. Scenarios run in disposable workspaces and score both reported answers
and durable artifacts. `validate` checks definitions only. Current `live` execution
and no-follow evidence reads require POSIX; routine coverage runs on Linux, not
Windows or macOS.

```sh
cargo build --release
python3 eval/run.py validate
python3 eval/run.py live --scenario completion-evidence-verification
```

`--scenario ID` selects a named case; repeat it for distinct cases. Omitting it
runs the complete suite. Unknown and duplicate IDs fail before execution.

`live` uses the normal Helm provider configuration and writes structured evidence to
`eval/evidence/latest.json`. Evidence is intentionally ignored by Git: attach it to
the evaluation record instead. Complete the manual interruption drills in the
[operator runbook](../docs/cutover.md) alongside automated evaluations. These
evaluations exercise local Helm workflows. They do not establish readiness for
the planned [multi-Helm voyage model](../docs/voyages.md): remote coordination,
user-scoped machine selection, interface disconnect/reconnect, and authorized
steering across Helms need additional end-to-end and behavioral evidence. Local
subagent delegation is not evidence of delegation to another machine.

Parallel scenarios require Helm to delegate independent work to multiple children,
wait for their terminal states, and synthesize attributed results. Runtime tests
separately exercise cancellation, timeouts, policy denial, restart recovery, and
edit-conflict behavior without spending provider capacity.

The runner uses the documented `--access unrestricted` CLI mode, closes stdin, and
sets each scenario's working directory to its disposable workspace. This permits
required file/todo writes and completion accounting without waiting for approval.
Use `--access` for integrations. Normal configured filesystem roots and command denials
remain active. Additional roots or external tools in the operator configuration
remain configured; a disposable directory is not an OS sandbox. Use a provider and
configuration approved for these tasks and review capacity/cost before live runs.

Each scenario defaults to a 300-second wall timeout; override it with a positive,
finite `--timeout-seconds`. Collection retains at most 1 MiB per stdout/stderr stream;
exceeding either limit fails the scenario. Timeout and output overflow send SIGKILL
to this invocation's POSIX process group, wait up to two seconds for the direct Helm
process, close captured pipes and record the observed result. The evidence records
`timed_out`, `output_limited` and `direct_process_reaped` separately. An escaped
process can outlive that group: the runner does not prove all descendant effects
stopped or graceful runtime cancellation. Interrupted canonical files retain their
actual state and are never rewritten into a convenient outcome.

Timeout, output overflow, startup failure, unobserved direct-process reaping and
nonzero exit remain failed evaluations even when every answer marker matches.
Only stdout can satisfy answer markers; stderr is diagnostic evidence. At most the
last 4,000 characters of each bounded capture are included in the report. Evidence
is replaced atomically after each finished scenario, so later scenario failures do
not erase earlier results. Wall/output limits are not provider request or cost
budgets; approve and enforce provider capacity separately before live execution.

## Counted completion evidence

`completion-evidence-verification` uses the `completion-count-v1` oracle. It supplies
raw CSV records, a claim, and a fixed counter script. Helm must read the sources,
invoke exactly `python3 count_records.py` through the normal shell policy, write
`verification.json` containing the counter result and `verification.md` explaining
the incorrect claim with source attribution, then record that exact result JSON as
a string in its one verification todo's evidence. It completes and reads/accounts
the todo using the actual Completion tool's fresh revision/fingerprint. The prompt
states this constrained task and the permitted tool/write paths explicitly.

The evaluator independently validates and counts the CSV rows, checks unique record
IDs and unchanged source/counter bytes, and compares values and source SHA-256s with
the report. It never runs a model-authored artifact to score the answer. It also
requires successful canonical source reads, the seeded counter call and its exact
zero-exit output, matching successful report writes, and measured evidence recorded
after counting. Saying or writing `42` cannot satisfy this oracle.

This case saves an ordinary session in a separate disposable `XDG_DATA_HOME`, keeping
normal provider configuration and application policy. It does not use `--no-save`.
The oracle reads the exact workspace/session/run references, accepted canonical
message fingerprints, one owned completed todo with structured evidence, its current
reviewed fingerprint and the immutable sealed Completed ledger. It independently
reconstructs the pre-seal readiness fingerprint. Missing/open/stale/wrong-run records
and incomplete outcomes fail. Other scenarios retain `--no-save` and their documented
marker checks; this stronger oracle does not silently upgrade their evidence.

Evidence contains the oracle version, observed IDs, counts and hashes of the report,
sources, canonical session, todo store and ledger. Disposable workspaces and raw stores
are removed after scoring; those hashes and checks are bounded provenance, not an
archive of all raw artifacts. Reads reject traversal, symlinks (including ancestors),
nonregular files, oversized files, malformed/duplicate-key JSON and invalid identities.
Limits are 4 MiB per ordinary evidence file, 64 KiB per report and 16 MiB per reader;
directory enumeration is bounded too. These checks protect evidence ingestion. They
are not attestation against malicious same-user code rewriting all local records.

This version changes the task from comparing supplied `99`/`42` statements to counting
actual records with an independently checked oracle. Earlier failed live results are
unchanged and are not identical-task comparisons or successful accounting evidence.
A passing offline scripted provider proves the acceptance boundary, not that a live
model can accomplish the task. No successful new live evaluation is implied.

`tests/system/eval_oracle.py` checks independent synthetic records and adversarial
changes, unsafe inputs, numeric booleans/overflow, bounded output and false positives.
`tests/system/eval_counting.py` runs the real CLI with all three native protocols:
successful targeted reconciliation, report-only claims, wrong reports, false/missing
evidence, denied counting and stale accounting. Several dishonest cases reach an
ordinary runtime Completed decision but still fail this independent task oracle.

`tests/system/eval_runner.py` tests the actual Helm CLI against a loopback native
provider: explicit access overrides read-only configuration for a real fixture file
write; a streaming Unicode timeout preserves failure evidence; stderr provider errors do
not satisfy answer markers; later scenarios still run. Both CI workflows
run it without spending provider capacity. It also checks that tool names referenced
by the scenarios (`todo`, `subagent`, `completion`, `questions`, `read_file`, and
`write_file`) are present in the actual provider request's live registry. This proves
the runner's mechanics, not that a live model follows the prompts or produces
correct work. Review and redact captured provider output before sharing evidence.
