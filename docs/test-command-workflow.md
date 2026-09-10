# Local command workflow check

Issue #249: a voyage executes a useful local command and reports its result.
This focused offline Linux happy path uses a real Vessel-supervised independent
voyage and the native `shell` tool, with a scripted loopback OpenAI Chat provider.
It does not use a real provider account or external network service.

From the repository root, with existing compatible debug binaries:

```sh
python3 voyage/tests/command_workflow.py --bin-dir target/debug
```

The binary directory may be absolute when running from an isolated worktree.
If binaries need rebuilding, coordinate use of the shared target directory before
running `cargo build -p vessel -p voyage --locked -j 8`. This fixture does not build
binaries. Python 3 with standard-library Linux pidfd support, `/proc`, `sh`, and
GNU `timeout` on PATH are required. Do not use Python's `-O` option: assertions
are part of verification.

## Acceptance and observations

1. The provider receives the exact user prompt and the native `shell` definition.
   It requests one foreground command to aggregate synthetic CSV sales by item
   and produce a JSON summary. The command has a five-second `timeout`; the
   voyage configuration also bounds shell execution to ten seconds.
2. The actual output file must exactly match independently specified expected
   bytes: apples 500 cents, oats 480 cents, and total 980 cents. The input CSV
   must be unchanged. Expected values are not computed by the command under test.
3. The provider's second request must contain the shell result with the correct
   tool-call identity, exact stdout, zero exit status, and empty stderr. Only
   after checking that result and the output file does the provider return the
   final assistant reply. Exactly two provider requests are accepted.
4. After completion and normal voyage suspension, a fresh retained snapshot must
   contain the exact prompt, shell result, final assistant reply, one completed
   turn, and a successful, complete structured command outcome with exit code 0.
5. Before reporting success, the fixture observes that its command process has
   exited (PID plus Linux start-time identity), no fixture-owned voyage remains,
   the supervisor has been reaped, and the loopback provider thread has stopped.
   Cleanup runs on failure too; unresolved cleanup fails rather than passing.

The fixture creates a private temporary evidence directory, isolated HOME/XDG
paths and synthetic workspace. The Vessel receives a minimal environment without
inherited provider credentials. HTTP requests and polling have finite bounds.
The script prints its evidence directory and leaves it for diagnosis, including
`workspace/sales.csv`, `workspace/summary.json`, `workspace/command-process.json`,
`snapshot.json`, `provider-requests.json`, `provider-errors.json`, `vessel.log`,
and, on success, `cleanup.json`. Do not publish the whole temporary directory:
it also contains private local runtime discovery state.

## Limits

This proves application plumbing for one normal local command, not whether a real
model selects a useful command or interprets arbitrary results. It is not Helm TUI,
live-provider, external-network, crash recovery, hostile subprocess containment,
or native macOS/Windows evidence. The process identity observation covers the
foreground Python transformation; the command intentionally creates no background
jobs. Normal idle suspension is not a crash/restart test.

This Python process check is separate from workspace Rust coverage. Existing
binaries may differ from checkout source; record the binary provenance alongside
execution evidence and rerun against delivery binaries when needed. Follow the
coordinated [validation workflow](quality.md) for coverage/publication and other
focused checks. This adds no broad suite or edge-case campaign.
