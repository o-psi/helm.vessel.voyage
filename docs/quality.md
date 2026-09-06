# Linux quality validation

Run the complete suite from a clean, committed source checkout before publishing:

```sh
./scripts/check-quality --plan
./scripts/check-quality
```

Install stable Rust with rustfmt and Clippy, Python 3.11 or newer, Git, and the native build,
packaging utilities used by the repository scripts. The runner selects the
stable toolchain and defaults to one Cargo build job. `--jobs N` changes compiler
parallelism; coordinate this with other work on the machine. Gates execute in
sequence.

The current automated tests and evaluation scenarios have been removed at the
operator's request and will be recreated later. There is currently no automated
regression coverage. Historical test results do not establish coverage for this
source tree.

`scripts/quality-gates.json` retains seven non-test gates: formatting, strict
Clippy, a locked optimized workspace build, and full plus installer packaging
with checksum verification. A passing run establishes only those checks.
Native platform and behavioral validation remain separate requirements.

The runner uses this checkout's `target` directory and absolute optimized Helm,
Vessel and installer paths. Custom `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET` and
`TARGET` selections are rejected when they conflict with native packaging.
Cargo `build.target` settings in checkout, ancestor or Cargo home configuration
are also rejected. Use native Cargo configuration for this suite.

Only one full suite may own a repository, including its linked worktrees. A lock
in Git's common directory rejects a second runner before it starts gates. Coordinate
manual Cargo commands too: they do not acquire this lock. Use isolated worktrees
and separate target directories for independent targeted builds. Keep the tested
checkout unchanged until validation finishes; dirty or changed source invalidates
the run. The runner checks committed revision and tree before each gate and after
the last gate.

Every attempt writes `.quality-runs/quality-REVISION-UUID/results.json` and one log
per executed gate. The record includes exact revision/tree, commands, directories,
start/end times, exit results and log paths. Existing evidence and packages remain
intact. Package labels are unique per attempt. A failure stops later gates; their
absence is not success. Default per-gate timeout is 3,600 seconds, configurable via
`--timeout-seconds` up to 86,400. Output is bounded to 64 MiB per running gate.
Interrupt, termination, hangup and timeout stop the command's process group, with
signal escalation when needed. A command leaving live group members fails the
gate. This is process-group cleanup, not an OS sandbox; a process that deliberately
creates another session is outside that group. Forced runner death can leave an
unfinished record: a `running` result is never passing evidence.

Both GitHub and Forgejo quality workflows are manual `workflow_dispatch` entry
points for the same runner. They retain results and logs as workflow artifacts,
including on failure. Local successful evidence satisfies the quality gate; do not
dispatch duplicate hosted runs merely to obtain another green result. Record the
local evidence with the issue or delivery record, reviewing logs before sharing.
Only published workflow definitions change remote triggers. Report actual branch
protection or access blockers without bypassing them.

Tag-only platform release workflows remain separate. Ordinary validation creates
no release tags and does not establish native macOS or Windows test coverage.
