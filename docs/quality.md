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
export RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward"
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
never publish the reduced inventory or hide missing source (#251).
The maintained forwarding wrapper uses Cargo's
[`RUSTC_WORKSPACE_WRAPPER` filename namespace](https://doc.rust-lang.org/cargo/reference/environment-variables.html)
without changing compiler arguments. Use its checkout-absolute path to separate
workspace objects from other checkout paths while keeping dependency caches.
Maintain an exclusive window through the final source/object/profile audit,
because `target/debug/helm`, `vessel` and `voyage` remain shared publication paths.
Preserve foreign raw profiles; with a shared target, clear and merge only the
measured workspace's profile prefix. Reject foreign maps rather than renaming
them or changing exclusions. The wrapper's actual source-switch qualification
belongs in #251; this documentation does not establish that a measurement passed.

Verify the extracted release with
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

The staged system-gateway transport has ordinary-UID tests for bounded frames,
partial/malformed input and both kernel peer-credential checks. The route tests
cover grant construction, private-envelope refusal, pairing preflight, browser
socket grant binding and recorded disconnect provenance:

```sh
cargo test -p vessel --locked gateway_ipc -j 8
cargo test -p vessel --locked gateway_route -j 8
```

These tests do not run a root supervisor or unprivileged network gateway. A
separate disposable Ubuntu VM test is required for actual peer identities,
public routing and failure/restart behavior; the exact fixture commands and
results belong in [#380](https://github.com/o-psi/helm.vessel.voyage/issues/380).
Even that route test does not establish a working system installer or bound
Voyage launch.

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

The protected guardian has an ignored integration test in the same disposable
native-root fixture. Supply root-controlled Vessel and Voyage executables:

```sh
VOYAGE_DISPOSABLE_ROOT_FIXTURE=1 VOYAGE_TEST_VESSEL_EXECUTABLE=/opt/voyage-test/vessel VOYAGE_TEST_CATALOGUE_EXECUTABLE=/opt/voyage-test/voyage cargo test -p vessel --locked --lib -- --ignored --exact process::guardian::tests::native_guardian_attests_pipe_launch_and_proves_descendant_cleanup
```

The native guardian fixture also exercises public `StartConfigured` over an
authenticated full-access connection using root-private `default-execution.json`,
ordinary runtime configuration/account state, exact replay, creation resolution,
not-admitted delayed-start fencing and observed cleanup. Run only on a disposable
host with the fixture's explicit ordinary account. Default local tests verify
nonowner and user-install refusal; they do not establish root/native launch.
Plain system `Start` and account/settings/model helpers remain unavailable.

It starts an actual ordinary-identity Voyage through a root-private pipe, checks
kernel UID/GID/groups, cleared inherited capabilities and authenticated Health,
then verifies projection/cleanup-marker forgery, duplicate/stale admission and
wrong pipe/identity refusal. A controlled runtime fixture double-forks into a
separate session; protected completion requires its observed death and reaping.
Killing a guardian leaves cleanup unresolved even after test teardown removes its
pinned child. Account state is synthetic and local to the fixture; no provider is
contacted. The fixture runs no system installation and establishes no client,
owner-review or adoption acceptance. It also exercises actual service detachment
and restart, protected Stop, a fresh bound incarnation, duplicate and conflicting
restart requests, stale Stop refusal, owner exclusion and continuing identity
invalidation. It also checks internal root-only initial admission, an exact
duplicate without relaunch, a conflicting binding, and crash-after-admission
recovery that observes an unavailable owner without launching a guardian. These
checks do not establish public bound creation or a system installation. Both
supplied executable wrappers must be named `vessel`/`voyage`
in the same protected directory. Its ignored native execution is
separate from default workspace coverage.

`cargo test -p voyage --locked --lib server::bound -j 8` checks bounded framed
input, truncated EOF and timeout cancellation without leaving a blocking stdin
reader. Local-actor tests cover execute-only ancestor traversal, denied listing
and symlink/non-traversable ancestor refusal.

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

The bounded Changes observation has a real-process offline check after building
`vessel` and `voyage` from the changed source:

```sh
python3 voyage/tests/workspace_changes.py --bin-dir target/debug
```

It reads unstaged/staged Git changes from a suspended executing Voyage, rejects
parent traversal and verifies the same incarnation, conversation revision and
provider-request count. Rust unit tests separately cover the byte limit and
public/private permission mapping. The Web production-bundle layout fixture
checks the panel's file navigation on desktop/mobile and confirms that opening it
does not dispatch `operator_tool`. This is local synthetic evidence, not an
authenticated production-console or external Git repository claim.

The same offline process check now exercises suspended `controls` discovery:
advertised tool metadata and policy-checked filesystem skill names/descriptions,
with a scoped WorkspaceRead grant and no skill-body disclosure. A composer menu must
be tested against an old capability response, a stale connection and a partial
catalogue, and selecting metadata must not submit a run automatically.
The same process check also verifies the `files` control returns bounded workspace
names without file contents, excludes symlinks and dependency directories, and
reports incomplete traversal. The Web composer unit and production-bundle browser
checks exercise filename insertion without an automatic submit.

The same process journey now qualifies `workspace_file`: explicit regular-text
preview through a suspended owner, untracked files without staging, bounded
Unicode truncation, symlink/traversal/binary/directory refusals, History-only
refusal and independent WorkspaceRead admission. It must retain the original
incarnation, canonical history and provider-request count. The Web Files journey
checks explicit selection before content reads, draft retention without Send,
connection/capability fences and desktop/mobile layout. Prepared source is not
passing evidence; retain the actual run result before claiming qualification.

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

The read-only Linux system-host assessment has focused argument and protected
path checks:

```sh
cargo test -p voyage-installer --locked system_preflight -j 8
cargo test -p voyage-installer --locked system_service -j 8
cargo test -p voyage-installer --locked system_install -j 8
voyage-installer system-assess --execution-user USER --gateway-user USER
```

The `system-assess` command observes the selected host but does not establish a supported
system installation, native root authority, workspace access, service startup or
gateway readiness. Run it as an ordinary user first; any root inspection belongs
in the designated disposable native Linux fixture.
The ignored `system_service::tests::native_systemd_accepts_pinned_root_and_ordinary_gateway_units`
test requires that fixture, an explicit `VOYAGE_DISPOSABLE_ROOT_FIXTURE=1`, and
`systemd-analyze verify`; it checks unit syntax, not activation or rollback.
The ignored `install::release::system_tests::root_staged_release_is_readable_by_an_ordinary_runtime_and_retains_browser_assets`
test requires the same disposable root fixture. It creates and removes a fresh
`/opt/voyage` in that fixture and verifies ordinary execution/read access and
tamper refusal. It does not install or start services.
The fresh system installer path must be exercised in a separate disposable
native Linux VM with two ordinary accounts, a root-controlled unpacked full
release and an external fixture provisioner. Check dry-run non-mutation, exact
root-owned release and worker hashes, protected control/runtime modes, effective
root/gateway UIDs and executable paths, pairing and scoped route, actual reboot,
and failure rollback after a deliberately failing provisioner. Preserve the
retained failure transaction and inspect that both service PIDs are zero and
both managed units disabled/removed. The fixture establishes fresh-install
behavior; update, rollback, public bound Voyage creation and production adoption
remain separate gates. Retain exact source/binary identities and results in #380.

System gateway service activation additionally requires a root-owned, 32-byte
external key on tmpfs supplied through `VOYAGE_CREDENTIAL_KEY_FILE` by its named
provisioning service. A unit syntax check alone does not establish key readiness
or pairing; native activation must test the real service dependency and route.

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
/absolute/path/to/webhelm/resources/js` with actual built Helm, Vessel and
Voyage and the matching `o-psi/webhelm` checkout. The fixture defaults its existing
WebSocket package to that checkout's `node_modules/ws`; use `--ws` for another
already installed package. It never downloads dependencies during the journey.
The process journey checks both Helm clients, DOM replay, ordinary first-action
claim and private takeover,
multiple voyages, external styles, cookie-gated images, open shadow DOM,
cross-origin child replay and element control, suspended-owner preparation and
observed cleanup. Its idle attached fixture viewer renews the status lease, just
as the production viewer does; it does not disable the runtime's 20-second fence.
Input-to-visible timing is recorded as local fixture evidence, not a latency
guarantee. Crash
qualification remains a separate adverse check.

The optimized v1.0.3 candidate passed the maintained agent plus both-client
journey, including actual suspended-owner preparation and observed cleanup.
See [the exact browser qualification record](testing/host-browser-v1.0.3.md)
for source/binary identity, covered behavior and remaining production TLS limits.

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

The Helm account/profile socket journeys use the existing private loopback
transport with scripted host metadata. They prepare full capability/host refusal,
exact enrollment resolve/cancel, cached and explicit usage, default uncertainty,
profile CAS save/default/delete and composer-retention assertions. They neither
invoke a model nor start a service or real browser. Source preparation is not a
passing result; the coordinated workspace verification must execute these cases.

A pending profile save now retains its exact command ID and origin: destination,
workspace, authenticated host, connection loss generation and runtime incarnation.
Only a matching save response containing the exact reviewed profile confirms it.
Unconfirmed outcomes remain bounded local bookkeeping with no automatic resend.
Review restoration requires the same current origin; closed, reactivated or changed
destinations cannot inherit the previous review through a later catalogue failure.
The prepared cases cover those boundaries and verify read-only loads never erase
an unrelated original save. Native root/grant and live-provider evidence remain
separate from these presentation/transport checks.

### Separate Helm Web source

Run the browser client checks in the private `o-psi/webhelm` checkout; the public repository no longer contains those tests. The public repository does not contain those tests. See its README for setup and shared browser asset synchronization.

### System lifecycle qualification remaining

On disposable native Linux guests, qualify explicit system upgrade for active and
inactive installs, exact root/gateway release PIDs and readiness, preserved paired
identity, credentials and running independent voyages. Verify inactive-only rollback
refuses candidates that were started; do not infer database-schema compatibility.
Inject unit publication/readiness failure and interruption, observe retained journal
and bounded refusal without replay. Service-removal uninstall must observe managed
PIDs zero and disabled/absent exact units while retaining state, releases, provisioner
and independent voyages. Root-local command tests do not qualify remote system updates.

#### Concrete system lifecycle fixture sequence

Use separate clean disposable native guests for active, inactive and interruption
cases. Never convert a developer or production install for these checks. Unpack
both checksum-verified full archives into root-owned `/root/release-old` and
`/root/release-new`, with the new SemVer strictly greater and both manifest targets
matching. Stable manifest versions may use the published `vMAJOR.MINOR.PATCH`
spelling; comparison normalizes that one prefix while retaining the exact original
manifest, release ID and receipt version. Include equal-version, prerelease,
downgrade and malformed-prefix refusal. Prepare the external key provisioner as above. Invoke the **new installer**
against the old runtime for initial staging, so the baseline has the protected
default execution contract (older fresh-install binaries did not provision it):

```sh
/root/release-new/bin/voyage-installer install --scope system \
  --bin-dir /root/release-old/bin --execution-user voyageordinary \
  --gateway-user voyageother --gateway-origin https://helm.example.test \
  --credential-key /run/voyage-secrets/connections.key \
  --credential-unit voyage-key-provision.service --no-start
/root/release-new/bin/voyage-installer upgrade --scope system \
  --bin-dir /root/release-new/bin --dry-run
/root/release-new/bin/voyage-installer upgrade --scope system \
  --bin-dir /root/release-new/bin
/root/release-new/bin/voyage-installer status --scope system
/root/release-new/bin/voyage-installer rollback --scope system
/root/release-new/bin/voyage-installer status --scope system
/root/release-new/bin/voyage-installer uninstall --scope system
/root/release-new/bin/voyage-installer status --scope system
```

For the inactive sequence, assert both managed MainPIDs remain zero and unit-file
states remain disabled; the upgraded units pin the new manifest release ID;
rollback pins the old ID. Snapshot protected default identity UUIDs, execution
account, external provisioner digest and retained control/runtime files before
upgrade, then assert their exact retention after rollback/removal. Removal must
leave both reviewed unit files absent and no managed PIDs, while retaining releases,
private state and provisioner/key. A subsequent fresh install over retained state
must refuse adoption rather than overwriting it.

On a separate active guest replace `--no-start` with `--start`; establish pairing
and exact retained authorization first. Upgrade must report the new root/gateway
process executables with root/ordinary UIDs and public readiness. Verify pairing,
public routing and retained identity again, including after an actual guest reboot.
An explicit rollback must fail before publication, with unchanged current units,
release and lifecycle record: candidate startup may have opened persistent state.
A live independent Voyage fixture must retain the same incarnation and process
start identity across supervisor upgrade/removal; a PID alone is insufficient.
Do not claim that managed MainPID zero proves independent descendant cleanup.

For interruption, launch upgrade under a bounded fixture controller, watch the
atomic lifecycle journal and kill only its installer process after an observed
nonterminal phase. Save the observed phase and subsequent exact journal. If the
operation already completed before interruption, the injection did not establish
the gate. A new upgrade/rollback/uninstall attempt must refuse the retained
nonterminal operation without replay, unit changes or a second candidate start.
A fault after the durable startup-attempt flag must retain that flag and refuse
old-binary rollback. Do not manually erase the journal to resume an uncertain effect.
Record exact source/archives, commands, exit statuses, managed PIDs and journal
phases; these are additional native evidence, separate from Rust line coverage.

## Protected administrator review transactions

```sh
umask 077
cargo test -p vessel --locked execution_reviews:: -j 8
cargo clippy -p vessel -p voyage-protocol --locked --all-targets --all-features -j 8 -- -D warnings
```

The transactional tests run as the ordinary developer identity, using in-memory
SQLite and the existing private-directory fixture. They cover explicit owner
selection/revision, exact replay/conflict, approval without process launch,
stale/expired/current-identity refusal, cancelled late approval, independent and
owner-wide grant revocation, truthful cleanup-pending receipts and transactional
schema migration rollback. Ordinary root-operator refusal is checked before any
mutation. A zero-match test filter is not passing coverage of this surface.

These tests do not establish native root enrollment, host/account/release fact
construction, administrator process launch, a public owner route or either
client. Default public capability remains unavailable. Complete disposable native
Linux authority, stale/duplicate/cancel/revoke/guardian cleanup and TUI/React
journeys before claiming #344 acceptance. First explicit root owner enrollment
activates protected catalogue schema 3; ordinary/user catalogues remain schema 2.
Older schema-2-only binaries cannot consume activated schema 3, so actual
compatible downgrade/rollback remains separately required.

### Provider boundary coverage source increment (#353)

The retained `chatgpt_oauth_boundary_tests.rs` and `multimodal_boundary_tests.rs`
add bound-account OAuth refresh/revocation assertions, private legacy migration
and logout fencing in an isolated subprocess, and image capability/provenance/
metadata/aggregate-byte admission assertions. All HTTP endpoints are scripted
numeric loopback and credentials are synthetic. Child HOME and XDG paths point
only into the temporary fixture; the parent environment is not mutated. Image
raster bytes are generated locally. Inference is forbidden in discovery fixtures.

This increment is source preparation only under the user's coordinated final-test
direction. It has not been compiled/executed or measured. No coverage percentage,
live authentication, provider spend or native service behavior is established.
The final integrated provider test pass must verify these cases along with the
existing provider tests, and final workspace coverage must retain the full source
and current object set. No denominator exclusions are introduced.

### Execution-identity account and configuration source increment (#344)

The private `identity-helper` and supervisor account/start/profile source is
prepared for the final coordinated milestone pass. No test or build has run for
this increment, following the operator's implementation-first direction. Prepared
regressions cover private-file limits/ownership, symlinks/hardlinks, exact digest
fencing, missing-file non-creation, separate identity profile state and exact
actor/namespace-bound mutation receipts. These are not passing evidence.

Final verification must additionally use actual dropped helpers and system/public
routes in disposable native Linux: ordinary account listing/defaults/models and
profiles, configured/account/settings creation, exact retry and not-admitted
resolution, account and connection revocation, interrupted capture, changed
configuration after capture, and observed guardian cleanup. The isolated provider
must be synthetic. Keep production deployment/TLS and published exact-source
archive qualification distinct. Do not execute a newer library test with an older
Vessel helper and infer that it verifies the new public route.

### Runtime coverage source pass after provider boundaries (#353)

Additional source-prepared fixtures target retained HTML gaps in these areas:

- Workflow collector: Linux PTY descriptors owned by an isolated test child,
  hidden Unicode input, bounded validation retries, cancellation, timeout,
  restored termios and post-collection signal monitoring. No human terminal is
  attached. Child completion markers prevent zero-test filtering from passing.
- Attachment owner: accepted-only workflow metadata, transient exact-run secret
  bindings, refusal after execution starts, live-turn cleanup refusal and exact
  actor reconciliation for simulated interrupted resources. Synthetic SQL state
  changes exercise bookkeeping; they do not prove native process-tree cleanup.
- Subagents: ordered inbox handoff, active followup identity, queued descendant
  cancellation, full-inbox shutdown and released capacity after executor unwind.
  The executor is synthetic and performs no provider, tool or worktree effects.
- Extensions: a private child with fixture HOME/XDG paths, an explicitly owned
  harmless process and host ledger. Bounded reads reject binary, oversized,
  hardlinked and renamed private sources. Revoked authority refuses new workers;
  aborted observer waiters retain pending obligations until process and workers
  are actually drained. This is not executable sandbox/namespace qualification.
- Filesystem tools: missing hashes, bad patch context/malformed hunks, cancelled
  publication, non-following directory traversal and argument-safe ripgrep search.
  Effects stay in private temporary roots; the Linux search fixture requires `rg`.

All of these are **unexecuted source preparation** under the user's final combined
validation direction. No new passing count, coverage percentage, denominator
exclusion, unreachable-path classification or native acceptance is claimed.
The source pass also catches subagent executor unwind and records an authored
failure without exposing its payload or replaying it. It releases the execution
slot; separate resource-cleanup ledgers remain authoritative and are not marked
observed by this handler. Explicit cancellation still takes precedence.

### Retired journal two-phase qualification pending

The `transition-helper` source has not been compiled/executed. At the final
integrated pass, verify private anonymous root-pipe admission, full namespace maps,
original UID observation/freeze, live startup/execution-lock refusal, exact revision/
history/config/pending digests, interruption without replay, Goal continuation
withdrawal and immutable source receipt retries. Root must receive no conversation,
SQLite content or target configuration bytes.

On disposable native two-identity fixtures, publish target ownership through the
root controller's safe descriptor path, then invoke target commit only in its
pinned UID/GID and account namespace. Refuse wrong identities, changed private
configuration, changed prepared facts and conflicting source/target command IDs.
Verify SQL configuration/revision/receipt commit together, lost-output lookup never
reapplies, and prepared-but-uncommitted runtime startup refuses. Preserve original
canonical Session IDs/messages, uncertain effects and external cleanup obligations.
Neither interruption nor the helper's successful receipt is native cleanup evidence.

### Bound scope bridge qualification deferred to final integrated pass

Verify kernel root-peer authentication before any secret write, two-second bounded
connect/frame I/O, unavailable/malformed/oversized replies, exact session/incarnation/
workspace/grant revision and expiry checks. Exercise live and suspended authorized
requests with an explicit lease; refuse legacy user handles and bound requests
without one. Verify per-dispatch revocation and account/history rights without
ordinary-runtime reads of root-private grant files.

Private lease cache verification must cover 0700 directory/0600 single-link records,
nofollow reads, 64 KiB/64-record limits, exact binding keys, redacted Debug and absence
from journal/history/events. Goal continuation must re-query current authority using
the cached lease and current registration after a clean restart; identity/grant
execution epochs must invalidate old leases. These are pending checks, not passing
coverage or native evidence. No tests/compilation were run for this source increment.

The supervisor counterpart also requires final native checks of the exact runtime
UID, registration token/incarnation, root lease and protected grant epoch. Exercise
ordinary restart with unchanged scope, ordinary-to-administrator and reverse
transition with old tokens, connection/participant revocation, source/target
namespace changes, supervisor restart with a retained Goal lease, and suspended
observation without root-directory access. Preserve no-replay oracles and prove
owned cleanup. Prepared source and successful source formatting do not establish
these outcomes; no broker test was executed during implementation preparation.

### Scope bridge adverse source cases (#353)

Prepared cases now cover exact/stale grant and runtime metadata, expiry/workspace/
owner/size refusal, ordinary abstract-socket peer rejection before any credential
write, truncated/malformed/oversized frames and bounded idle reads. Cache cases
assert 0700/0600, single-link/nofollow records, exact keys, capacity and redacted
Debug; incoming bound requests cannot borrow a cached handle when the wire handle
is missing. Clean-registration credential reuse requires new current metadata;
a readable cache never proves a valid root execution epoch. The native wrong-peer
case explicitly requires ordinary Linux UID and does not simulate UID 0 authority.

These are source-only cases, not executed verification. Actual root broker epoch,
parent/connection revocation and suspended/live roundtrips remain final integrated
native gates. Existing legacy-file authority test construction was updated for the
new AuthoritySource representation without weakening its assertions.

### Identity-scoped enrollment and usage source qualification pending

No tests or provider calls have run for the new ordinary system account worker.
The final coordinated set must verify actual UID drop/private-root refusal before
authority metadata, root-peer authentication, bounded channel failure, revocation
between poll/exchange/publication, duplicate start waiters, socket loss with retained
worker, negative-admission and interrupted-effect no-replay, supervisor restart,
changed default identity, private-code exclusion and one usage refresh at a time.
Prepared negative peer/descriptor cases are unexecuted. Use synthetic provider
fixtures and disposable native hosts; skipped or inaccessible provider/host work
is not passing evidence. Preserve existing user-service account behavior.

### Helm account/profile/connected frontend coverage source pass (#353)

A further unexecuted source pass targets the retained Helm account/profile and
frontend/browser gaps. Profile cases exercise the actual scripted WebSocket
loading sequence, optional account-label failure, nil host refusal, late replies
for abandoned destinations, absent defaults and retained values after failure.
Account cases cover exact usage identity/capability, private code disposal,
closure without fictitious cancellation, hidden/busy/resize input fencing,
unavailable transports and authored failure notices without private codes/URLs.

The connected frontend seam retains production owner/configuration logic with an
explicit fixture client: fresh configured start never submits, active resume
refuses overrides, idle resume pins the observed revision/model and no-override
resume retains the live owner. Viewer cases assert uncertain control poisoning,
read-only mirror refusal and stale binding/one-use bootstrap boundaries. Synthetic
peers bind numeric loopback; no real Vessel, provider or browser is contacted and
no operator default configuration/storage is connected by these fixtures.

Ordinary `Configure` and `Receipt` requests address the stable session and let
Vessel select its current owner. The idle resume fixture changes observed owners
while retaining the saved model or explicit override, exact mutation ID and
observed revision fence. Receipt lookup preserves the exact session/command and
rejects a different session; it remains readable after an owner restart. Live
resource commands retain their separate exact incarnation fences. The coordinated
run exposed two fixture assertions that incorrectly applied those live fences to
ordinary configuration and receipt reads; the corrected assertions require the
same public routing contract as production.

The corrected fixture assertions require coordinated local execution and workspace
coverage after integration. Source correction alone establishes no passing count,
coverage improvement, native terminal acceptance or 100% reachability claim.
Final integrated verification must include existing tests and the unchanged full
workspace denominator. Execution UI hooks owned by the concurrent identity work
were not modified by this pass.

### Owned-PTY terminal and transport source journeys (#353)

Additional unexecuted Linux cases run an isolated test child on a child-owned PTY
with a scripted numeric-loopback peer. They prepare real attach/resize/snapshot/
render/restoration paths for explicit detach, owner change after attach, already
exited programs and observed exit. Only Ctrl+] is injected by the fixture; no human
terminal or real program receives input. Mode restoration and completion markers
must be observed in the final run. Unknown outcomes refuse composer resumption;
restoration alone is not an explicit detach or remote cleanup observation.

Transport source cases preserve exact receipts and private socket identity, refuse
uncertain input retransmission after loss, and distinguish complete no-dispatch
preparation from partial/positive effect claims. No native terminal acceptance or
passing evidence is inferred from these source-only additions.

### Notification and plain observer source journeys (#353)

Eight additional unexecuted behavioral cases exercise explicit typed notification
operations, exact mutation command identities, recipient-scoped seen/dismiss
receipts, configure input and local bounds, and malformed watch pages. Scripted
peers refuse any extra owner navigation, decision response, cancellation or unseen
receipt. Plain observer cases prepare root-decision typed responses and changed-run
refusals, event-stream loss/refusal/wrong-session observation refresh without
resubmission, valid invalidation without refresh, and output identity/UTF-8 cursor
failures leaving the previous cursor intact. Retained HTML for inbox, plain session
and output is source guidance; it is not a measurement of these edited cases.

No Cargo, runtime checks or coverage were run for this source increment. The final
coordinated run must establish compilation, behavior and coverage over the same
whole-workspace denominator, including all production targets and ignored-test
accounting.

### Passive inbox identity correlation source preparation (#353)

The passive overview now refuses a receipt for a different requested event and
refuses page entries pairing a receipt with a different notification event. Six
additional unexecuted sources cover those fences, exact owner run/incarnation/
decision references, unavailable owner states without fabricated authority, typed
metadata/private-text refusal, page/inventory bounds, producer/budget uncertainty,
and attention probes retaining unknown rather than inventing an unread count.
Only the overview and its own new test module changed; execution UI hooks remain
owned by the concurrent transition work. These edits still require the coordinated
final compilation, runtime checks and workspace coverage measurement.

Two further plain-interface source journeys exercise EOF, explicit quit, blank
input and input failure through the actual chat event loop while scripted peers
permit only observation. A second source matrix refuses malformed machine output
without repeating an admitted command. These are prepared, not executed evidence.

### Executable extension adapter acceptance source (#353)

The actual `ExtensionTool` now has an isolated offline source journey with a
scripted SDK peer and its own private resource ledger. Twelve scenarios prepare
tool/command/lifecycle output acceptance, progress sanitization, read-only/approval/
secret refusal before launch, wrong output owner, schema mismatch, split/private
result refusal, output limits and unresolved cleanup. Quarantined attempts must
not launch again, private contexts must be removed, and outcome/cleanup remain
separate exact records. The fixture adapter launches no executable or OS process;
positive release requires completion of its owned Tokio peer task. It does not
establish native executable isolation, descendant cleanup or live-provider behavior.
These source cases have not been executed and await the single final verification
set. The current published coverage summary remains unchanged.
### Bound initializer and opaque transfer source preparation (#353)

Nine new unexecuted adversarial source cases cover private transfer bytes and
hashes, symlink/hardlink/mode/source-directory refusals, initialization provenance
conflicts, uncertain capture without default recapture, nil participant/self-branch
references, metadata-only portable checkpoint observation, the separate 16 MiB
opaque artifact bound, and retained source inode replacement during chunk reads.
The small credential/provenance record limit remains 64 KiB; opaque signed
transport artifacts use a separate bounded descriptor read instead of increasing
that credential-cache denominator or weakening its private-file boundary.

No Cargo/test/build/Clippy/coverage or native journey was run for this source
increment. Formatting and diff checks are source checks only. Final acceptance
requires the integrated implementation and unchanged whole-workspace coverage
scope, including real bound initializer/transfer/restart fixtures and the exact
transition recovery hooks; these prepared tests are not passing evidence.

The system adoption helpers are hidden typed Vessel subcommands,
`migration-user --directory ABS --pipe-fd FD` and
`migration-control --directory ABS --pipe-fd FD`. They require their root-created
private pipe and their own original-user/full-host authority checks; parsing a
command line does not provide migration authority. The final disposable native
adoption verification must use the same built Vessel as the installer, qualify
quiescence/reboot, retained namespace and exact services, interrupted preparation/
capture/freeze/install/fence recovery, explicit activation and refused legacy
administrator promotion. Source registration alone is not passing evidence.

### Offline binary entry and execution presentation cases (#353)

Additional source cases exercise the actual Cargo binary entry points through
`CARGO_BIN_EXE_helm`, `CARGO_BIN_EXE_vessel` and `CARGO_BIN_EXE_voyage`. The tests
in `helm/tests/offline_main_cli_tests.rs`,
`vessel/tests/offline_auth_main_tests.rs` and
`voyage/tests/offline_runtime_main_tests.rs` isolate HOME/XDG state, exclude
credentials from the inherited environment, and retain only LLVM's output
profile destination for the final measurement. They cover generated documents,
no-start/scoped route refusal, gateway validation before database/listener,
invalid grant capture, runtime validation, bounded startup refusal, content-free
catalogue refusal and exact resource-confirmation checks. None starts a service,
contacts a provider or supplies human terminal input. Runtime subprocess cases
have a ten-second deadline and bounded captured output.

Execution presentation source cases additionally fence stale incarnation replies
and preserve unknown operation IDs and unsent drafts without writing private
receipt storage or dispatching approval. These cases were prepared from retained
coverage evidence; they have not been run during the implementation-first pass.
The final coordinated tests and workspace coverage measurement remain required.

Retired user profile command IDs are now refused before both identity receipt
lookup and the profile mutation transaction, through the same migration tombstone
check as lifecycle receipts. Two prepared adversarial sources verify that old
cached receipts cannot become current authority, validation callbacks do not run,
and Save/Delete/SetDefault IDs cannot mutate either namespace after migration.
The retained profile catalogue and original receipt remain intact. These sources
are unexecuted pending the final coordinated verification.

The next #353 source batch adds real Helm extension CLI lifecycle cases: packaging
never replaces an archive, installation stays inactive, exact digest review binds
activation, updates clear activation, stale deletion/revocation refuses, skill
snapshot import stays inactive, and invalid package-index origins refuse before
network admission. Model-facing GitHub adapter cases reject unknown approval/admin
fields, malformed arguments, absent owning runs and disabled local capability
without constructing remote requests or publication state.

Private extension-store fault cases distinguish pre-rename failure (old bytes and
cleaned temporary files) from post-rename lost response (committed candidate bytes,
no rollback or automatic retry). Missing read-only paths remain absent, exact
size bounds hold, and symlink/FIFO inputs refuse without following or waiting.
These source cases add no coverage exclusions or measured percentages. The
single coordinated verification/coverage pass is still pending.
### Bound recovery and initializer integration audit source preparation (#353)

The retained pre-change HTML identifies 36 uncovered cells in recover_command,
38 in participant observation, 71 in lifecycle and 330 in bound lifecycle (line
cells are source guidance only; current integrated totals require measurement).
New private helper sources prepare refusal before file access on absent retirement,
revoked authority or malformed actor/attestation references; isolated child-owned
journals prepare exact metadata receipt replay/conflict, owned startup-fence refusal
and no replay of claimed operator reconciliation with a missing receipt. Their
HOME/XDG/host-resource ledger roots are private to the fixture child; no operator
store or real provider is used. A separate real journal case prepares Goal fencing
with unchanged objective and idempotent continuation denial.

The synthetic helper check used by these journal cases tests private state
semantics, not a positive Root transport or native administrator boundary. Final
qualification still needs actual Root peer/UID drop/current owner callbacks,
revoked execution grant versus cleanup permission, original administrator namespace,
private Root result recovery after loss, per-resource uncertainty preservation,
participant epoch rotation and bound polling/cancellation. No cases were executed,
no exclusions were added, and no 100% coverage or acceptance claim is made.
For #379 agent interaction, `npm test --prefix voyage/browser` includes real
Chromium paging, native form/keyboard/drag actions, same- and cross-origin frames,
stale-reference refusal, exact receipts and private diagnostics exclusion. Run
`python3 voyage/tests/host_browser.py --agent-interactions --binaries /absolute/path/to/built/bin --web-resources /absolute/path/to/webhelm/resources/js --ws /absolute/path/to/ws`
for the scripted agent loop followed by both existing Helm viewer journeys and
observed cleanup. This adds no paid provider calls or hosted quality jobs.

The grounded control sources also cover open shadow controls before light-DOM
controls, exact paged handle identity, shadow-local accessible labels, nested
hit testing, mutation/new-root invalidation and explicit traversal truncation.
The Linux worker suite passed all 24 cases with no failures, cancellations or
skips in 16.803 seconds after the final oversized-observation fix. The toolchain
was Node 26.8.2, pinned Playwright 1.63.0 and Chromium 152.0.7977.82. The grounded
case keeps its 60-second timeout. An earlier reproduction reached the wide
observation in 3.259 seconds but cancelled at 60 seconds and did not finish
cleanup until 265.735 seconds. Rendered text/control geometry are now withheld
after the node walk truncates; the 100001-node fixture traps those reads and
requires that neither runs. Its hidden subtree isolates traversal from enormous
inline layout while visible controls before and after it exercise the boundary.

The four real guardian crash/shutdown cases observed zero remaining live
processes, zombies, process groups, socket descriptors/files and profile
directories. Uncertain effect receipts remain retained; only cooperative shutdown
releases the worker lock. Raw failing and corrected logs remain under ignored
`target/verification-v103/browser-final/`. These checks cover the Linux worker and
guardian, not the full Helm client journeys, root-bound public transport,
production TLS, installers or other platforms.

### Traversal-only runtime parent regression

The native USER adoption journey exposed a leaf-storage failure under the root-owned
`0711` runtime parent. Linux private-directory opening now uses an `O_PATH` parent
capability for existing leaves, still validating the leaf's own UID, type and private
mode. Publishing a private file syncs its owning directory after the atomic rename. Creating a new directory
first obtains a readable parent handle, then retains and syncs that parent barrier;
missing durability access refuses before `mkdir`. Focused cases cover existing-leaf
read/publication, creation refusal and shared-leaf/symlink refusal. Final native
qualification must retry the original-UID target handoff on the corrected binary;
these source cases do not establish other operating-system behavior.
