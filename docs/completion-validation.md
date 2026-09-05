# Completion gate acceptance evidence

Tracking [#81](https://github.com/o-psi/voyage/issues/81),
[#82](https://github.com/o-psi/voyage/issues/82), and
[#83](https://github.com/o-psi/voyage/issues/83).

The [operator contract](completion-gate.md) defines scope, one-pass reconciliation,
request costs, failure recovery and provisional output. This map identifies executable
coverage; a test's presence is not a passing result. Delivery still requires the
integrated release suite, actual platform CI and separately budgeted live evidence.

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
Helm/Vessel wire contract is introduced by the fixtures; worker/frontend coverage
must still be inspected when their implementation changes. Live provider smoke
previously returned HTTP 401 and remains a separate acceptance hold until successful
authenticated execution is observed.

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
