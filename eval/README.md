# Helm replacement-readiness evaluation

Voyage is developing its first version and has no product releases. This suite
collects evidence toward a release-quality product that delivers tested user value.
The `--release` flag below selects Cargo's optimized build profile.

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
the first-release readiness/cutover record instead. A passing run is necessary but not sufficient for
cutover; complete the manual interruption drills in the operator runbook. Remote
Vessel cutover is blocked during the [connectivity transition](../docs/vessel-connectivity-retirement.md),
not established by these local evaluations.

Parallel scenarios require Helm to delegate independent work to multiple children,
wait for their terminal states, and synthesize attributed results. Runtime tests
separately exercise cancellation, timeouts, policy denial, restart recovery, and
edit-conflict behavior without spending provider capacity.
