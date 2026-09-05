# Completion gate acceptance evidence

Tracking [#81](https://github.com/o-psi/voyage/issues/81),
[#82](https://github.com/o-psi/voyage/issues/82), and
[#83](https://github.com/o-psi/voyage/issues/83).

The [operator contract](completion-gate.md) defines scope, one-pass reconciliation,
request costs, failure recovery and provisional output. This map identifies executable
coverage; a test's presence is not a passing result. Observed Linux release results
are attributed to exact integration commits below. Later changes require their own
checks; full #83 acceptance also requires separately budgeted live evidence. The operator
removed required macOS/Windows runs for cost; Linux results do not establish
unrun platform behavior.

| Acceptance criterion | Executable evidence |
| --- | --- |
| Fresh root identity, owned todos, nested children, unrelated previous runs | `completion_readiness.py`: initial and resumed CLI runs, nested child/grandchild ownership, empty scope |
| Durable publication and missing/corrupt ownership fail closed | `completion_readiness.py`: inspect disk during HTTP dispatch; delete/corrupt known ledger before resume; assert no dispatch |
| Cross-process ownership and restart | `completion_readiness.py`: real competing CLI; `completion::runtime` process-exclusion and owner-exit recovery tests |
| Actual final boundary, clean single-request completion | `completion_gate.py`: empty run, each native transport, immutable completed decision |
| Withheld proposal, targeted system update, review and recheck | `completion_gate.py`: saved provisional proposal inspected at next HTTP dispatch; real read_file/todo/completion calls; revised accepted final |
| Evidence synthesis instead of status-only gaming | Fixture reads exact measured value before evidence/accounting; `completion-evidence-verification` behavioral scenario challenges an incorrect claim |
| Blocked/deferred versus successful work | Fixture preserves statuses and requires impact accounting; exit, decision and saved summary remain incomplete |
| Ignored update and repeated false final | Fixture allows exactly one additional proposal and requires incomplete outcome; runtime gate deadline tests cover tool loops |
| Children do not gate themselves | Nested ownership fixture completes children while root reviews their results and preserves pending child todos |
| Cancellation, provider failure, recovery | Gate fixture injects native HTTP failure and real SIGINT during reconciliation; canonical proposal survives and saved run is interrupted |
| No duplicate final or synthetic human turn; resume | Gate fixture counts canonical proposals/finals, rejects persisted system instructions, preserves prior transcript and annotations on fresh-run resume |
| Stale evidence, store failures, concurrent writers, late registration | `completion::runtime` review/fingerprint, publication, failed seal and barrier-controlled mutation/registration tests |
| Scoped abort, failed children, policy and deadline limits | `agent::gate_tests`: scoped shutdown, unrelated work, missing store, cancellation, bounded provider/tool waits, inherited scope |
| TUI, plain frontend, terminal ownership | `completion_handoff.py`, TUI rendering/outcome tests and session classification tests; gate fixture exercises actual one-shot CLI |
| Persistence migration and immutable historical decisions | `completion` schema validation and storage/runtime restart tests; [storage contract](completion-storage.md) |

The native fixture runs OpenAI Chat, OpenAI Responses and Anthropic against loopback
HTTP without real credentials or provider capacity. Both CI workflow counterparts
execute it using the release binary. Existing ownership/handoff assertions remain
in place; the new fixture does not substitute for those lifecycle checks.

Run from the repository root after building the intended binary:

```sh
HELM_BIN=target/release/helm python3 tests/system/completion_gate.py
HELM_BIN=target/release/helm python3 tests/system/completion_readiness.py
HELM_BIN=target/release/helm python3 tests/system/completion_handoff.py
python3 eval/run.py validate
```

Manifest validation checks 12 scenario definitions, not agent behavior. Offline
fixtures cannot prove semantic honesty, actual external work, hardware power-loss
durability, native subscription authentication, every terminal layout, or platform
behavior on systems where they did not run. The optional compatibility bridge has
separate fake-process runtime tests; these HTTP fixtures do not exercise it. No new
Helm/Vessel wire contract is introduced by the fixtures. Local managed Journal
execution is covered by `local_managed.py` and attachment runtime tests.
[Dedicated remote sessions](remote-sessions.md), delivered in
[PR #129](https://github.com/o-psi/voyage/pull/129), add outbound execution and
`remote_session.py` lifecycle coverage. Ordinary attachment remains presence-only.
The remote fixture verifies effects, retries, cancellation, public replay and
recovery; it does not repeat the entire local false-final/reconciliation matrix
or exercise a distributed voyage. Subscription provider smoke previously
returned HTTP 401 and remains a separate authentication/cutover hold until
successful authenticated execution is observed.

For an ordinary completed task, use `completion snapshot`, read each returned owned
ID with `completion read`, verify the underlying evidence through the live tools,
then `completion account` with the snapshot's current revision/fingerprint and
`completed_with_evidence` or `incorporated`. If the record changes, fetch a fresh
snapshot and read it again; replaying a stale review must fail.

For work awaiting access, retain the todo's actual blocked status and blockers,
then account with `blocked_with_impact` and a concrete description of what remains
and who can unblock it. For intentionally postponed pending work use
`deferred_with_impact`. Both allow truthful accounting but produce an incomplete
run, including a nonzero one-shot exit. Neither disposition changes the task to
completed. A later CLI prompt starts a new run; use explicit adoption if that new
run is to own the prior task. Resuming a transcript alone does not adopt old work.

## Observed integration run (2026-09-05)

The integrated Linux debug binary built from `09ed9e3` plus `25e054f` passed all
21 native gate cases (seven each for Chat, Responses and Anthropic), the complete
ownership fixture, and the PTY handoff fixture. The handoff wait now distinguishes
canonical text publication from durable acceptance; all original ownership,
interruption, transcript, runtime-release and completion assertions remain.
The older root debug binary fails before its first response because it has not
published the accepted session; it is not evidence of the integrated behavior.

These historical results are debug/offline Linux evidence. Later release and live
results appear below; unrun platform behavior is not established by them.

## Integrated release verification (2026-09-05)

At `f09e4dd` (Rust source last changed in `20bfd2a`), Linux formatting, strict
workspace/all-target/all-feature Clippy, all 447 workspace tests and the locked
release build pass. All 14 release system fixtures pass: native transport/no-turn
limit, titles, steering, questions, tool output, archives, subagent resources,
context budgets, completion ownership, three-provider gate, frontend handoff,
Vessel lifecycle, plain SIGINT and evaluation-runner failure recovery. Tool output
also passes with fragmented 73-byte PTY reads. Twelve eval definitions validate.
Unique package `completion-20260905-f09e4dd` and its SHA256 check pass.

Plain SIGINT coverage now proves active cancellation preserves separate partial
output and 7/3 usage, and idle SIGINT exits while stdin stays open. Evaluation
runner tests use actual offline Helm subprocesses to verify writes, provider error,
Unicode timeout evidence and subsequent-scenario continuation; these are not live
behavioral evaluations. Read-only archive/resource scenarios retain all original
assertions and now prove denied accounting leaves 3/12 owned results unresolved,
with durable incomplete outcomes unchanged by a separate successful lookup run.

At that historical checkpoint, the bounded subscription retry returned HTTP401.
Successful subscription authentication remains unverified; later local-provider
evidence does not remove that separate hold.


## Managed integration and bounded local verification (2026-09-05)

Integration `3950fef` preserves PR112 ancestry and includes main `cfd5ace`. Linux
formatting, strict workspace/all-target/all-feature Clippy, all 713 tests both
normally and under actual two-CPU affinity, the locked release build, all 19
integrated workflow system fixtures, 12 eval definitions, unique packaging and
SHA256 checks pass. The managed fixture also passes 20 repeated child-cleanup
cases. Independent review found no blocker in the cancellation watcher or fixture
corrections. These results are Linux-only.

A real external cancellation transaction exposed transient SQLite Busy reads in
the cancellation watcher. It now retries only Busy observations within the
existing five-second budget and stops between completed reads without abandoning
in-flight work. Non-Busy errors and exhausted budgets remain unconfirmed cleanup.
Separate fixture corrections synchronize partial-output inspection after the
actual durable event and inspect admission/cleanup before the first root provider
response for each session. Repeated child/parent HTTP handlers no longer inject
raw database reads into concurrent checkpoint commits. A deterministic held-reader
test proves a failed COMMIT rolls back every mutation and an explicit same-delta
retry publishes one event. No production mutation retry was added. See
[the integration results](https://github.com/o-psi/voyage/issues/83#issuecomment-5554021877).

One bounded local `completion-evidence-verification` run used llama.cpp b10819 and
Ministral 3 3B Q4_K_M, CPU-only with four threads, context 32768 and requested/server
output limit 1024. The original prompt was retained with transparent guidance to
read owned IDs and copy the current revision/fingerprint when accounting. Isolated
HOME/XDG/config/workspace and a literal-loopback proxy excluded operator credentials
and enforced at most 12 model requests and 300 seconds of execution.

The run **did not pass**. It exited 1 after 178.36 seconds and ten HTTP200 responses.
It read both seed files and wrote a 545-byte report identifying claimed 99 versus
measured 42 with matching source hashes. The report additionally claims counting
operations that were not independently performed. Six invalid completion reads
were rejected; no account call was attempted. The todo remained
pending with no structured evidence, despite evidence-like title/progress text.
Context preflight rejected another dispatch at estimated 34959 tokens versus
32768. The canonical session and sealed ledger honestly retain Interrupted,
one unresolved obligation and zero accounted obligations; no final was accepted.

At the time of that live run, the advertised completion schema required only
`action` and permitted fields from all actions, while the strict decoder required
`kind`/`id` for `read` and rejected `reason`/`revision`. This contract gap corresponded
to the observed invalid calls. The [action schema correction](completion-tool-contract.md),
integrated through PR121 at `68f5c9a`, now advertises action-specific requirements
and has independent schema and six native HTTP complete/stream exposures. It
retains strict decoding and all evidence/accounting checks; it does not convert
this live failure into successful reconciliation. The local run reported 41678 input and 941 output tokens, exactly matching
canonical usage; the largest observed response was 250 output tokens. Both owned
Helm and model-server processes were reaped. Full prompt, raw requests/responses,
artifacts, canonical state, usage and cleanup evidence are retained locally.

This supplies useful local failure and recovery evidence, not successful semantic
reconciliation, subscription authentication, cutover or full #83 acceptance.
No additional live attempt is implied by these results.

## Main follow-up and remote lifecycle evidence (2026-09-05)

The bounded local-model follow-up on main `995a937` (runtime `31453fa`) also failed.
It used the same disclosed evidence scenario and API guidance, with the context
increased to 65,536 to account for 8,399 additional serialized tool-definition bytes.
The 12-request/300-second limit, 1,024 output-token request, four CPU threads and
isolated local provider boundary remained explicit; this was not an identical-budget
comparison with the earlier attempt.

The run timed out after 300.007 seconds and five started requests. Four completed
responses reported 24,430 input and 448 output tokens, matching canonical usage;
the fifth stream was interrupted without usage. Missing usage is not zero work.
The model read both files and wrote a 486-byte comparison with supplied values and
correct source hashes, then supplied objects instead of strings for todo evidence.
Strict decoding rejected it. No structured evidence, completion accounting or
accepted final was recorded.

The harness sent SIGTERM; Helm exited -15 and the model server exited 0. Both
processes were observed reaped. Raw session state remained Provisional and the
ledger remained Open. This is abrupt-termination evidence, not an observed graceful
seal or proof that execution survived. Recovery must preserve that distinction.
See [the recorded result](https://github.com/o-psi/voyage/issues/83#issuecomment-5554402413).
No later successful semantic evaluation is established here.

Separately, PR #129 delivered dedicated remote execution to main at `fd5295e`.
Its recorded Linux evidence includes 894 workspace tests normally and with two-CPU
affinity, all 26 system fixtures, strict quality checks, locked release, 12 evaluation
definitions and package checksums. The actual Helm/Vessel fixture exercises three
native adapters, effects, exact retries/reconnect, cancellation, forced-death
recovery/attestation, revocation, redacted replay and unconfirmed-cleanup failures.
See [the delivery evidence](https://github.com/o-psi/voyage/issues/78#issuecomment-5554950951).
These are historical offline Linux results, not new runs for this documentation
audit or proof of cross-machine voyage coordination.

## Remaining acceptance and merge boundaries

- Verify subsequent schema/runtime changes at their final integrated Linux head.
  Completion and Todo action-schema corrections reached main through
  [PR #88](https://github.com/o-psi/voyage/pull/88). Historical results below earlier
  commit headings remain limited to those commits.
- Obtain an approved, bounded real-model run that actually reads source evidence,
  records structured todo evidence, reads/accounts the owned records with fresh
  revisions and reaches the intended durable outcome. Inspect the artifact and
  ledger: the evaluation scenario's `42` substring checks alone do not prove
  accurate counting, semantic honesty or correct accounting. A deliberately
  incomplete run must retain truthful impact and cannot substitute for a required
  completed task.
- Extend dedicated remote worker/Vessel verification to the full reconciliation
  matrix: withheld false finals, reviewed incomplete work, stale/late participant
  results, cancellation and provisional-output recovery. Existing remote lifecycle
  coverage is useful evidence, not completion of that matrix. Planned
  [voyages](voyages.md) additionally need evidence across permitted Helms with a
  coordinator separate from the interface. An accepted local run must not imply
  that the open-ended voyage has ended or that another Helm's results were reviewed.
- Retain the separate subscription-authentication and cutover hold. Local-model
  evidence cannot validate subscription credentials. No native-platform result is
  claimed; the operator removed macOS/Windows as required gates for this delivery.

The compatibility bridge's fake-process regression proves request-only guidance
rebuilds the thread, preserves canonical history and rejects a second unsupported
final. It does not demonstrate successful authenticated bridge accounting. The TUI
has durable checkpoint/classification tests plus an actual PTY cancellation,
handoff and resumed-turn fixture; those are stronger than rendering-only tests,
but are not a live-model TUI semantic evaluation.

These broader acceptance items keep full #83 and subscription cutover open.
The delivered local runtime and dedicated remote lifecycle retain their recorded
verification and scope limits. Neither establishes successful live-model
reconciliation or delivery of the multi-Helm voyage product.
