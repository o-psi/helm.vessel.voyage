# Helm evaluations

The suite exercises coding, system-administration diagnosis, local research,
professional writing, data analysis, interruption-safe checkpointing, and parallel
delegation. Scenarios run in disposable workspaces and score both reported answers
and durable artifacts.

```sh
cargo build --release
python3 eval/run.py validate
python3 eval/run.py live
```

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
finite `--timeout-seconds`. Timeout kills the Helm subprocess, records any captured
partial stdout/stderr with `timed_out: true`, `exit_code: null` and a failure reason,
then continues to the next scenario. It is a failed evaluation even if partial output
matches every expected substring. A timeout is not proof of graceful cancellation
or descendant-process cleanup; the runner does not claim either. Process startup
errors and nonzero exits also remain failures. Evidence is replaced atomically after
each finished scenario, so later errors do not erase previously recorded results.

`tests/system/eval_runner.py` tests the actual Helm CLI against a loopback native
provider: explicit access overrides read-only configuration for a real fixture file
write; a streaming Unicode timeout preserves failure evidence; provider errors do
not become passing substring matches; later scenarios still run. Both CI workflows
run it without spending provider capacity. It also checks that tool names referenced
by the scenarios (`todo`, `subagent`, `completion`, `questions`, `read_file`, and
`write_file`) are present in the actual provider request's live registry. This proves
the runner's mechanics, not that a live model follows the prompts or produces
correct work. Review and redact captured provider output before sharing evidence.
