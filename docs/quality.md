# Validation

The previous automated suites and evaluation scenarios were removed at the
operator's request. Targeted Linux regression coverage now checks concurrent
voyages in one workspace. Broader suite recreation remains separate; historical
passing runs do not establish coverage for today's source.

## Linux binary release checks

Use `umask 077` for local test processes: the private-storage fixtures require
owner-only temporary files and directories. This tightens fixture creation and
does not disable the runtime's storage checks.

Run from clean committed source, retaining logs under ignored `target/`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets --all-features -j 8 -- -D warnings
cargo build --workspace --release --locked -j 8
python3 -m unittest discover -s packaging -p test_package_linux.py -v
python3 voyage/tests/conversation_files.py --bin-dir target/release
python3 packaging/package_linux.py --version v1.0.2 --bin-dir target/release --output dist/linux-release
(cd dist/linux-release && sha256sum -c *.sha256)
```

Also run the workspace coverage workflow in `AGENTS.md` after final Rust/test
edits, and commit `coverage/latest.json`. When reusing the ordinary `target/` with
cargo-llvm-cov's wrapper and `--no-clean`, passing tests alone do not prove that
all workspace artifacts were instrumented. Inspect executed artifact timestamps,
LLVM mappings and the source inventory against the previous full report. Ordinary
cached workspace dependencies can lose cross-crate counters even when every source
file appears in the union. If confirmed, refresh the source-root timestamps for
all workspace targets returned by `cargo metadata --no-deps --format-version 1`
without changing bytes, verify clean Git content, clear raw profiles only, and
repeat the full measurement. Preserve rejected reports and dependency caches;
never publish the reduced inventory or hide missing source (#251). Verify the extracted release with
`packaging/verify_linux_install.py`; see the
[release guide](releases-v1.0.2.md#maintainer-install-check).
Its `--hosted` mode simulates the bootstrap's systemd user-manager reachability
check inside Bubblewrap before testing anonymous pinned and latest downloads;
it does not establish real user-manager or service activation.
Use `python3 voyage/tests/two_voyages.py --bin-dir target/release` for the
current concurrent file-work check. Its `--legacy-ledger corrupt`,
`--legacy-ledger locked` and `--legacy-ledger exhausted` variants verify that
retired global allowance records cannot block execution or be mutated; the normal
variant verifies that no new global ledger is created. The legacy `tests/concurrent_voyages.py`
fixture below calls a retired endpoint and is not a current release gate. Missing/deleted historical
scripts are not a passing gate and must not be restored implicitly. This release
workflow does not depend on `scripts/check-quality` or the old packagers.

Linux checks need Rust, rustfmt, Clippy, matching LLVM coverage tools, Python
3.11+, Git, native build tools, tar/checksum utilities and Bubblewrap for isolated
installer checks. Do not run competing builds against one target directory.

A passing run establishes only the checks actually recorded. Other runtime
behavior, security boundaries, native-platform operation, real service activation,
reboot persistence and live model quality need separate evidence. Live provider
work requires an approved provider and budget. No skipped or unavailable check is
a pass.

## Privileged control storage fixtures

The staged Linux control and runtime-directory primitives have focused
ordinary-UID checks. The runtime fixture verifies a trusted execute-only parent,
private session ownership, invalid names, symlinks and changed permissions:

```sh
cargo test -p voyage-storage --locked protected_linux -j 8
```

Its native-root test is ignored in the default workspace coverage run. It must
run only in an explicitly disposable Linux root fixture, with an ordinary account
named `voyageordinary` and a root-owned non-writable source/test working directory:

```sh
VOYAGE_DISPOSABLE_ROOT_FIXTURE=1 cargo test -p voyage-storage --locked --lib -- --ignored --exact protected_linux::tests::native_root_records_exclude_ordinary_identity
```

Do not create that account or run this command as root on a development or
production host. The test changes ownership only on its temporary fixture record,
launches an ordinary UID/GID child with cleared groups, and checks denial of control
record reads/writes. Native evidence is separate from default coverage and does
not establish a working privileged Vessel or installer. The Linux fixture used for
[#346](https://github.com/o-psi/helm.vessel.voyage/issues/346) is a disposable KVM VM;
matching development loader/libraries were used for its copied test executable.

The staged cross-identity launcher has a separate ignored native-root test in
that same kind of disposable fixture, with `voyageordinary` present:

```sh
VOYAGE_DISPOSABLE_ROOT_FIXTURE=1 cargo test -p vessel --locked --lib -- --ignored --exact process::launch::tests::native_root_launch_drops_to_ordinary_user_without_regain
```

It observes actual UID/GID/group/capability state and failed root regain in a
child process. It does not start a Voyage or establish system-service readiness.

The bound catalogue integration has an additional ignored native test. In that
same disposable fixture, provide a root-controlled `VOYAGE_TEST_CATALOGUE_EXECUTABLE`
pointing to the built Voyage program (or a root-controlled matching-loader wrapper):

```sh
VOYAGE_DISPOSABLE_ROOT_FIXTURE=1 VOYAGE_TEST_CATALOGUE_EXECUTABLE=/opt/voyage-test/voyage cargo test -p vessel --locked --lib -- --ignored --exact process::catalogue_observer::tests::native_bound_catalogue_uses_ordinary_helper_and_protected_layout
```

Run from a root-owned, non-writable directory outside `/tmp`. The fixture creates
and removes only its unique child directory. It uses the real journal reader under
`voyageordinary`, verifies UID/GID/groups/capabilities/environment and control-file
denial, then exercises wrong-session/oversized/stalled replies, unsafe executable
links/permissions, runtime permission changes, a hostile journal symlink, stale
metadata retention and changed identity refusal. It does not start a Voyage owner,
install a system service, use a provider or establish full #344 acceptance.

## Historical concurrent voyage regression (not a current gate)

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

The offline [conversation and file-editing check](test-conversation-files.md)
verifies a normal task through Vessel-supervised voyage processes: read and patch
a file, save the reply, then continue with retained context and read the edited
file. It checks actual file contents and successful tool outcomes using a scripted
loopback provider, without credentials or paid requests.

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 voyage/tests/conversation_files.py --bin-dir target/debug
```

This Linux process check is separate from the workspace Rust coverage percentage.

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

## Ordered event replay and reconnect

`python3 voyage/tests/live_events_integration.py --bin-dir target/debug` uses an
explicit synthetic named account and real supervised processes. It checks bounded
public-v2 pagination, duplicate reads, session isolation, public-v1 compatibility,
unsupported-projection refusal, catalogue checkpoint/pagination/bounds and owner
changes across restart, retained replay after Vessel restart, and append-only
history across a fresh Voyage incarnation. Delayed one-event pages verify replay
continuity; they do not simulate browser-socket backpressure or full client rendering.

The reconnect and delivery-recovery journeys above/below also configure explicit
synthetic accounts. They do not rely on the removed implicit provider-account
selection. The recovery check asserts canonical tool outcomes for approval expiry,
denial and cancellation, plus absent filesystem effects. All three journeys retain
private evidence and must observe fixture cleanup before reporting overall success.
`python3 voyage/tests/goals.py --bin-dir target/debug --only web --web-root /path/to/webhelm`
uses the built React client in Chromium and a real Helm PTY against one isolated
Vessel. After the Goal review checks it holds a synthetic streamed response until
both clients display its partial text, disconnects only Web, observes TUI completion,
and verifies automatic Web reconnection reaches the canonical run/revision/cursor
without duplicate text or repeated inference. It checks independent catalogue
hydration and long polling across reconnect. The local TLS proxy is fixture evidence;
it does not prove the production proxy or account login.

Add `--long-history` to the process replay command to complete 80 synthetic turns
while advancing one event reader and leaving another behind. The fixture requires
actual 2,048-event retention overflow, explicit gap recovery, a bounded recent
snapshot and lossless revision-fenced 17-message history pages. It records the
largest event page and checks observation never repeats inference. This exercises
retention and paging, not TCP congestion or client heap limits. For actual socket backpressure, run:

```sh
python3 voyage/tests/socket_backpressure.py --bin-dir target/debug
```

This Linux fixture uses a real scoped gateway and two independent WebSockets.
The slow peer advertises a small TCP receive window, stops reading, and keeps
sending pings; sixteen authorized subscriptions exercise the output budget while
one fast peer continues reading and issuing scoped snapshot requests. It requires
an observed kernel transmit queue, bounded transport closure independently of
heartbeat expiry, exact continued fast-peer text, retained-event overflow and
explicit authorized snapshot recovery, one synthetic inference, bounded measured
gateway RSS growth and observed fixture cleanup. Its narrow standard-library
WebSocket client is test-only. Evidence includes actual timings and counts; this
does not certify arbitrary network conditions, a browser client or production TLS.
The browser rendering/heap fixture lives in WebHelm's
[`tests/browser-conversation-browser.mjs`](https://github.com/o-psi/webhelm/blob/main/tests/browser-conversation-browser.mjs).
Deployed authenticated rollout remains a separate #372 gate.

## Everyday workflow checks

These focused offline Linux checks exercise normal product behavior using real
Vessel-supervised voyages and scripted loopback providers:

| Workflow | What is checked |
| --- | --- |
| [Stop and continue](test-stop-continue.md) | Cancel inference, observe cleanup, then complete a new message with retained history. |
| [Two voyages](test-two-voyages.md) | Overlapping real file writes, two running owners, and separately retained conversations. |
| [Disconnect and reconnect](test-client-reconnect.md) | Real Helm detaches without cancelling work, then reconnects and continues the conversation. |
| [Run a command](test-command-workflow.md) | A native shell command transforms data; actual output, exit status, and saved reply agree. |

```sh
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/stop_continue.py --bin-dir target/debug
python3 voyage/tests/two_voyages.py --bin-dir target/debug
python3 voyage/tests/client_reconnect.py --bin-dir target/debug
python3 voyage/tests/command_workflow.py --bin-dir target/debug
```

Use Python without `-O`. Each guide describes bounded execution, private local
evidence, cleanup and limits. These process checks are separate from the workspace
Rust coverage percentage; they do not certify live models or the full-screen TUI.

## Delivery recovery checks

```sh
cargo build -p helm -p vessel -p voyage --locked
python3 voyage/tests/delivery_recovery.py --bin-dir target/debug
```

This focused Linux process check uses isolated HOME/XDG directories and a local
synthetic provider. It verifies event polling overlapping idle suspension and
submission, a lost acceptance response, exact duplicate admission, durable
non-admission and late-request refusal across suspension and Vessel restart,
identity/payload conflicts, scoped resolution rights and caller binding, cleanly
stopped owner resolution without restart, and distinct approval
expiry/refusal/cancellation.
Evidence is retained under the printed `/tmp/vdr-*` directory. Cleanup succeeds only
after fixture-owned processes disappear and matching durable cleanup evidence is
observed. This is not a live-provider, native macOS/Windows, or full TUI check.

## Deleted working directories

Deleted-directory recovery has a focused offline Linux check:

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 voyage/tests/missing_workspace.py --bin-dir target/debug
```

It checks saved history and delivery resolution with a missing workspace, same-session
continuation in a private recreated directory, retained access restrictions,
parent Git isolation, duplicate refusal, and recovery after a failed startup.
The [runtime contract](runtime-contract.md#deleted-working-directories) describes
filesystem limits and the explicit steps needed to use Git again.

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
[UX readiness](ux-readiness.md). The current operator instruction requires objective
executable verification, not human testing or product-owner visual/adoption sign-off.
A build/package pass is not a complete functional-journey verdict. The full interface
scope remains tracked on [#14](https://github.com/o-psi/helm.vessel.voyage/issues/14);
a completed slice does not close real remaining dependencies.

The focused offline Helm journey check exercises actual PTYs at 40×18, 80×24 and
120×32, Unicode draft preservation, name-based switching, concurrent voyages,
detach/reconnect, branch/archive targeting, pasted-consent refusal, typed manual
tool execution and private workflow submission. It uses isolated HOME/XDG paths and
scripted loopback responses, not the operator's credentials or a paid provider:

```sh
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/ui_journeys.py --bin-dir target/debug
```

This is a scoped executable check, not restored broad regression infrastructure,
public TLS evidence or native macOS/Windows certification. Its private temporary
evidence records binary hashes, canonical results, request counts and cleanup.
Failures must be reported as failures; the command's existence is not evidence
that it passed.

## Direct image paste verification (#74)

The explicitly scoped image checks and supervised workflow live alongside the
account-limit checks; this does not recreate the removed broad test suites. See
[Images and screenshots](multimodal-implementation.md#verification) for commands
and limits. Fixtures use synthetic pixels and local provider responses only;
no test reads the operator's actual clipboard, captures the desktop, or spends live-provider budget.
The PTY regression uses private fake clipboard helpers and exercises inline paste,
editing, cancellation/errors, draft migration and supervised image delivery.

## Named provider accounts

The focused offline account journey uses synthetic API bindings and a loopback
provider with real independent Vessel-supervised Voyage processes:

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 tests/provider_accounts.py --bin-dir target/debug
```

It covers two concurrent account identities, staged current/next-turn selection,
exact conflicting/replayed selection envelopes, suspension/resume and branching,
same-identity API rotation (including split-stream redaction during an active run),
logout refusal without fallback, and explicit scoped
account/enrollment metadata access. It uses the current public `/v1/vessel/command`
contract while preserving the older concurrent fixture unchanged. It does not
establish native platform, live-provider, or public TLS results.

The #262 sign-in regressions additionally exercise plain/empty pending HTTP bodies,
code visibility across polls, private failure phase/status diagnostics, cancellation
before a fresh enrollment, and success refreshing Helm choices without changing
selection. These are offline fixtures; they do not establish live sign-in success.

Rust account tests retain actual loopback device polling/exchange races, identity
conflicts, private storage failure cases, and independent process refresh fencing.
Helm account tests cover its private view and durable public selection envelopes.
These tests are included in workspace coverage; the Python journey is separate.
When using a shared build target, retain every current Cargo compiler-artifact
executable in the LLVM report and exclude historical stale binaries, not workspace
packages. See [#251](https://github.com/o-psi/voyage/issues/251).

## Provider attempt recovery (#261, #269)

See [provider failures and recovery](provider-attempts.md) for the focused native
process fixture, diagnostic contract, activity/timeout semantics and bounded
history-based continuation. The 66 offline cases use Responses, Chat Completions
and Anthropic loopback endpoints with actual independent Voyage processes. They
cover connection recovery, interruption/exhaustion, retained partial lineage,
completed tool-result preservation, activity-only progress and comment-only timeout,
plus the existing rejection, cancellation, paging and exact-command checks.
The separate Helm PTY failure check verifies truthful notices after three partial
attempts exhaust their shared budget. These process checks remain separate from
workspace Rust coverage. A separate `provider_restart_recovery.py` check kills its
owned process during backoff, recovers without attestations and verifies no saved
inference replay after restart before explicitly submitting a new turn. These
checks do not establish live-provider, WebSocket or native macOS/Windows behavior.

## Runtime root-consent regression (#327)

The focused offline root tests cover generic-Decide refusal at both routing ends,
durable typed consent/deduplication, cancellation, unrestricted unattended refusal,
exact directory identity, run/access-generation revocation, child isolation, Helm
scope presentation, file access and required Linux sandbox mounts:

```sh
cargo test -p voyage -p voyage-protocol -p vessel -p helm --locked root_ -j 8
```

The Linux sandbox case launches bubblewrap and requires working user namespaces;
setup failure is a failing check, never an unsandboxed fallback. This does not
exercise live provider billing or certify native macOS/Windows behavior.

## Public nightly acquisition

Run `python3 packaging/test_public_nightly.py -v` after changes to the public
nightly resolver or bootstrap. It exercises anonymous metadata, full archive
extraction and failure handling with local HTTPS fixtures. It is an offline check:
verify the actual published release and isolated installation separately.

## Bootstrap-only checks

Run `python3 packaging/test_bootstrap.py -v` for offline bootstrap regressions,
including POSIX shell syntax, no-argument defaults, explicit argument forwarding,
preflight refusals and success-only guidance. These use fake installer/systemctl
executables: they do not establish real service activation or hosted downloads.
See the [installer guide](../installer/README.md)
for bootstrap prerequisites and the distinction from the Rust wizard.

## Host-browser packaging (#333)

The browser asset inventory extends the Linux release contract. Focused local
checks (no hosted quality job) are:

```sh
python3 -m unittest discover -s packaging -p test_browser_assets.py -v
python3 -m unittest discover -s packaging -p test_package_linux.py -v
cargo test -p voyage-installer --locked browser_assets_are_verified -j 8
```

These checks verify staging and integrity, not browser execution. Browser
execution and cleanup use `npm test --prefix voyage/browser` after pinned npm
preparation, and `python3 voyage/tests/host_browser.py --binaries
/absolute/path/to/built/bin --web-resources
/absolute/path/to/webhelm/resources/js` with actual locally built Helm, Vessel and
Voyage and the matching `o-psi/webhelm` checkout.
The process journey checks both Helm clients, DOM replay, ordinary first-action
claim and private takeover,
multiple voyages, suspended-owner preparation and observed cleanup. Crash
qualification remains a separate adverse check.

The viewer also uses `node tests/browser-next-browser.mjs` (from the private `o-psi/webhelm` checkout) for real Chromium
DOM replay and element input at desktop/mobile sizes and
`node tests/browser-layout-browser.mjs` (from `o-psi/webhelm`) for the production React shell.
The worker test includes real same-origin and cross-origin nested frames, a
private sign-in field, a cookie-gated image, localized canvas fallback, stale
frame references and parent-page message exclusion. The viewer Chromium fixture
checks nested frame replay, media overlays and typed input at both desktop and
mobile sizes. These fixtures are synthetic local sites; a deployed
TLS proxy and public-site qualification remain separate checks.
`node --test tests/host-browser.test.mjs`, `npm run typecheck`
and `npm run build` (in `o-psi/webhelm`) check the client. The browser journeys do not
certify arbitrary public websites, native macOS/Windows behavior or resource
budgets on a deployed host.

## Persistent Goal controls (#378)

`cargo test -p helm --locked --lib process_client::ui::goals -j 4` exercises
the TUI command review, explicit Resume, owner/Goal/incarnation fences, idle and
cleanup checks, unknown/mismatched receipts, plain-text rendering and scrollable
small-terminal review. Runtime Goal tests are described in [goals.md](goals.md).
These focused checks are part of the final workspace coverage obligation.

In the separate Web repository, `tests/react-goals.test.ts` and
`tests/react-goal-panel.test.ts` cover canonical review, owner-only mutations,
finite limits, replacement consent, stale drafts, safe text, keyboard focus and
ID-only receipt recovery. `node tests/browser-layout-browser.mjs` exercises the
built production bundle at desktop/mobile sizes with synthetic transport. Its
Goal checks verify explicit continuation consent, exact single submission,
canonical state and focus restoration; it neither contacts a provider nor proves
a separate Voyage process survived restart. Cross-client/process and restart
journeys remain separate required evidence before #378 closes.

With the three debug binaries built, run
`python3 voyage/tests/goals.py --bin-dir target/debug` for real separate-process
continuation, limits, approval, input/cancel/restart, authenticated scope/revocation
and three-level delegated usage. The `--only` option selects a focused journey
when investigating a failure. Use `--only web --web-root /absolute/path/to/webhelm`
to exercise the built Web bundle and TUI against one real canonical Voyage over
paired browser sockets through fixture TLS. These local, synthetic-provider checks
are separate from Rust coverage and production OAuth/TLS verification. See
[Goals](goals.md#offline-process-verification) for evidence boundaries.

## Execution profiles

`cargo test -p helm -p vessel --locked profiles -j 8` covers profile storage,
revision conflicts, exact mutation replay, account-scope visibility and TUI profile
selection/management. The conversation/file check above additionally creates a
profile, copies it into a real supervised voyage, edits and deletes the profile,
and verifies the resumed voyage retains its original model. With Helm also built,
`python3 voyage/tests/conversation_files.py --bin-dir target/debug --profiles-tui`
additionally opens the real TUI picker, selects the named profile and checks that
selection sends no inference. These checks use
synthetic accounts and local provider responses. Web profile tests run with the
existing Web test scripts; they do not establish live sign-in or provider behavior.

### Separate Helm Web source

Run the browser client checks in the private `o-psi/webhelm` checkout; the public repository no longer contains those tests. The public repository does not contain those tests. See its README for setup and shared browser asset synchronization.
