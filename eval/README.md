# Helm replacement-readiness evaluation

The suite exercises coding, system-administration diagnosis, local research,
professional writing, data analysis, and interruption-safe checkpointing. Scenarios
run in disposable workspaces and score both reported answers and durable artifacts.

```sh
cargo build --release
python3 eval/run.py validate
python3 eval/run.py live
```

`live` uses the normal Helm provider configuration and writes structured evidence to
`eval/evidence/latest.json`. Evidence is intentionally ignored by Git: attach it to
the release/cutover record instead. A passing run is necessary but not sufficient for
cutover; complete the manual Vessel and interruption drills in the operator runbook.
