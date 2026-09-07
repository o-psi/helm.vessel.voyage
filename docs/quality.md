# Validation

The previous automated suites and evaluation scenarios were removed at the
operator's request. Targeted Linux regression coverage now checks concurrent
voyages in one workspace. Broader suite recreation remains separate; historical
passing runs do not establish coverage for today's source.

## Available checks

`scripts/quality-gates.json` defines eight gates:

1. Rust formatting.
2. Strict workspace Clippy across targets and features.
3. Locked optimized workspace build.
4. Concurrent voyage process regression.
5. Full archive packaging.
6. Full archive checksum verification.
7. Standalone installer packaging.
8. Installer checksum verification.

Run them from a clean committed checkout:

```sh
./scripts/check-quality --plan
./scripts/check-quality
```

The Linux runner needs stable Rust, rustfmt, Clippy, Python 3.11+, Git, native build
utilities, archive tools and checksum utilities. It defaults to one compiler job;
`--jobs N` selects another bound. Do not run competing builds against the same
output directory.

A passing run establishes these build, analysis and packaging results plus the
specific Linux behaviors below. Other runtime behavior, security boundaries,
native macOS/Windows operation, deployed services and live model quality need
separate evidence. Live provider work requires an approved provider and budget.
No skipped or unavailable check is a pass.

## Concurrent voyage regression

From a source checkout, after building `vessel` and `voyage`, run:

```sh
python3 tests/concurrent_voyages.py --bin-dir target/release
```

The fifteen cases start actual Vessel-supervised voyage processes with one shared
workspace and host data root. A local HTTP fixture holds provider responses until
both voyages reach inference, so sequential execution cannot pass. Coverage includes
separate live PIDs and canonical histories; simultaneous first-run admission;
duplicate command and competing session-owner exclusion; cancellation and a new
turn while the peer keeps running; simultaneous delegated agents using file tools
with isolated task/completion state; and brief versus persistent contention in the
shared inference database without replay or lost attribution. Additional cases verify
completed turn suspension, history/receipt observations without waking the executor, fresh
next-turn incarnations with canonical history and deduplication, and refusal to
automatically replay work while fenced recovery respawns an owner that lacks clean
suspension evidence. That recovery also preserves canonical history and the
supervisor-catalogued voyage name. Default-start configuration is retained even
when the original configuration file disappears between turns.
A supervisor restart selects its updated executable path for the next turn, and
simultaneous submissions against one suspended incarnation admit only one owner.
The local process endpoint is authenticated loopback HTTP rather than a public or
Unix-socket Helm route: checks cover private discovery credentials, rejected bearer
and browser-Origin requests, durable SSE invalidation/reconnect, and an actual Helm
run following SSE through terminal suspension without replay. A scoped loopback
gateway fixture exercises the same route used behind HTTPS, including grant-bound
catalogue/SSE access, revocation and stream termination. This does not establish a
deployed TLS proxy or public-network result.

The fixture uses only Python's standard library, synthetic credentials, isolated
HOME/XDG directories and a loopback provider. It retains evidence under its printed
`/tmp/vct-*` path and observes cleanup of its own runtime processes. These are
automated offline Linux checks, not live-provider or native macOS/Windows evidence.

## Conversation and account-limit checks

The explicitly enabled live test uses the executing user's native ChatGPT OAuth
login and an explicitly selected model. It sends at most three requests without
tools or automatic retries, requires three completed replies remembering a random
marker, and serializes/reloads canonical history and provider continuation between
turns. It tests the provider adapter; it does not establish TUI or supervised-process
acceptance. Run only with account and usage approval:

```sh
VOYAGE_TEST_MODEL=YOUR_MODEL cargo test -p voyage --locked --test chatgpt_conversation -- --ignored --nocapture
```

An exhausted account fails this check; a quota error is never counted as a successful
conversation. The ignored test does not contact a provider during ordinary tests.

The offline failure regression starts an isolated Vessel and independent voyage
processes against a loopback ChatGPT quota response, with synthetic credentials.
It checks one inference request per explicit submission, safe durable failure
summaries, failed transcript status, history retention, and observed cleanup across
suspension and the next turn. It retains evidence under its printed `/tmp/vql-*` path.

```sh
cargo build -p helm -p vessel -p voyage --locked
python3 voyage/tests/account_limit.py --bin-dir target/debug
cargo test -p voyage --locked --lib provider::failure_tests
```

These focused checks do not restore the removed general test suite. No live success
is implied by the offline failure regression.

## Delivery recovery checks

```sh
cargo build -p helm -p vessel -p voyage --locked
python3 voyage/tests/delivery_recovery.py --bin-dir target/debug
```

This focused Linux process check uses isolated HOME/XDG directories and a local
synthetic provider. It verifies event polling overlapping idle suspension and
submission, a lost acceptance response, exact duplicate admission, durable
non-admission and late-request refusal across suspension and Vessel restart,
identity/payload conflicts, and distinct approval expiry/refusal/cancellation.
Evidence is retained under the printed `/tmp/vdr-*` directory. Cleanup succeeds only
after fixture-owned processes disappear and matching durable cleanup evidence is
observed. This is not a live-provider, native macOS/Windows, or full TUI check.

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
