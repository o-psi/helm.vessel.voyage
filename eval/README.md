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
evaluations exercise local Helm workflows.

Parallel scenarios require Helm to delegate independent work to multiple children,
wait for their terminal states, and synthesize attributed results. Runtime tests
separately exercise cancellation, timeouts, policy denial, restart recovery, and
edit-conflict behavior without spending provider capacity.
