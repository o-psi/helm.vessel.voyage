# Validation

The automated tests and evaluation scenarios have been removed at the operator's
request and will be recreated later. **There is currently no automated regression
coverage.** Historical passing runs do not establish coverage for today's source.

## Available checks

`scripts/quality-gates.json` defines seven non-test gates:

1. Rust formatting.
2. Strict workspace Clippy across targets and features.
3. Locked optimized workspace build.
4. Full archive packaging.
5. Full archive checksum verification.
6. Standalone installer packaging.
7. Installer checksum verification.

Run them from a clean committed checkout:

```sh
./scripts/check-quality --plan
./scripts/check-quality
```

The Linux runner needs stable Rust, rustfmt, Clippy, Python 3.11+, Git, native build
utilities, archive tools and checksum utilities. It defaults to one compiler job;
`--jobs N` selects another bound. Do not run competing builds against the same
output directory.

A passing run establishes only these build, analysis and packaging results.
Runtime behavior, security boundaries, native macOS/Windows operation, deployed
services and live model quality need separate evidence. Live provider work requires
an approved provider and budget. No skipped or unavailable check is a pass.

## Isolation and evidence

The runner locks the repository's common Git directory, including linked worktrees.
It records the exact commit and tree, rejects dirty source, and rechecks source
identity between gates. Keep the checkout unchanged until the run finishes.
Manual Cargo commands do not acquire this repository-wide lock.

Output lives under `.quality-runs/quality-REVISION-UUID/`: `results.json` records
each gate's command, directory, timestamps, exit status and log path. Package labels
are unique. Preserve existing evidence and release artifacts. A failed gate stops
later gates; an unfinished `running` record is not passing evidence.

The runner uses native `target/release` binaries and rejects conflicting target or
Cargo output overrides. Per-gate timeouts default to 3,600 seconds and output is
bounded to 64 MiB. Cancellation/timeout stops the process group and checks for live
members. This process-group cleanup is not an OS sandbox.

## Publication

GitHub and Forgejo quality workflows are manual entrypoints to the same runner.
Keep validation local; do not dispatch hosted jobs merely to duplicate passing
local checks. Tag-driven archive builds are separate from quality validation and
do not establish native behavioral coverage.

For documentation-only edits, verify claims against source/help, relative links and
anchors, manifest membership, command examples and diffs. Verify archive layout when
packaged guide paths change. Follow [release procedures](releasing.md) and report
which checks actually ran, with their limits.

## Product interaction acceptance

Feature validation also needs the relevant journeys and adverse states in
[UX readiness](ux-readiness.md). That manual inventory records functional evidence
and product-owner review separately. A build/package pass is not a ship-ready UX
verdict. The full interface audit remains open on
[#14](https://github.com/o-psi/voyage/issues/14); a completed feature slice does not
close it. No automated test suite is introduced by this acceptance document.
