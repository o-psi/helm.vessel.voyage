# Useful-work evidence for #22

## Scope and decision

The current instruction for [#22](https://github.com/o-psi/helm.vessel.voyage/issues/22)
requires **no human testing, visual acceptance, scheduled trial or cutover gate
yet**. This supersedes the issue's historical human-trial prerequisites, not its
correctness requirements. No default tool, production service, account or deployment
was changed. The issue remains open for the unverified scope below; this is not
replacement-readiness or live-provider certification.

Use the [architecture](architecture.md), [current state](current-state.md) and
[readiness map](readiness.md). Helm detachment, Voyage interruption and Vessel
failure are different events. Independent voyages are not subordinate agents;
participant assignments and owner transfer require their own authority/evidence.
The retired outbound worker and removed evaluation runner are not prerequisites.

## Evidence classes and identity

Recorded on Linux 6.18.50-2-lts, x86_64, with Python 3.14.7, 2026-09-11.
The isolated `issue-wave-22` checkout starts at
`c4a3f4a26fe1eae1cf72a5a23e49540ea44e492a`. Only this guide and `eval/README.md`
are changed by this delivery. Runtime/test source is unchanged. No Rust coverage
refresh is required for these documentation-only changes; existing coverage is
not represented as newly measured.

1. **Controlled offline process checks:** existing focused Python checks with real
   Helm/Vessel/Voyage binaries, synthetic local provider responses and isolated
   HOME/XDG/workspaces. Actual file bytes, canonical history and cleanup are
   checked. Model choices/replies are scripted, not live semantic acceptance.
2. **Current production-run work:** the ongoing owner voyage
   `9687a6a8-b00a-4fb9-9a47-9ab5308c0b17` performed bounded research, coding/data,
   writing, process interruption and native Vessel coordination. This is historical
   evidence from this run, not a reproducible controlled model benchmark. Its host
   binary is not assumed to be the freshly built fixture binary. Provider model,
   request/token/spend totals and final run cleanup were not measured here and are
   unknown, not zero. Current-run delegation was allowed; no new provider/account
   was configured or launched for this delivery.
3. **Historical issue evidence:** retained at its original revision/configuration;
   neither documentation edits nor fixture passes repair a past semantic failure.

Raw local evidence is retained in `target/issue-wave-22/` (build log, binary hashes,
per-check logs, one-off work artifacts and verification results), plus each focused
check's printed private temporary directory. Do not publish raw provider requests,
snapshots or private discovery credentials. These records are local evidence, not
files distributed in the archive. The public summary below contains no credentials.

## Controlled checks

Commands use the shared target and hold
`flock /home/psi/voyage/.local/issue-wave-build.lock` across
the build and checks. This proves our lock discipline, not universal compliance
by other builds. The build initially waited on Cargo's own directory lock; the
coordinator later reported another owner's shared-artifact contamination and
requested exclusive handoff to #213. Our build had already finished; the running
final fixture was allowed to finish its bounded cleanup, with no next Cargo or
coverage command queued. These are the artifact hashes captured immediately after
our build, not a coverage-artifact integrity claim:

```sh
export CARGO_TARGET_DIR=/home/psi/voyage/target
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/conversation_files.py --bin-dir "$CARGO_TARGET_DIR/debug"
python3 voyage/tests/command_workflow.py --bin-dir "$CARGO_TARGET_DIR/debug"
python3 voyage/tests/stop_continue.py --bin-dir "$CARGO_TARGET_DIR/debug"
python3 voyage/tests/two_voyages.py --bin-dir "$CARGO_TARGET_DIR/debug"
python3 voyage/tests/client_reconnect.py --bin-dir "$CARGO_TARGET_DIR/debug"
python3 voyage/tests/delivery_recovery.py --bin-dir "$CARGO_TARGET_DIR/debug"
```

| Binary | SHA-256 |
| --- | --- |
| `helm` | `8428abaecd8f29c0f5b18da1cb0a161008141cf1b73c5bf31835f4df0628e54a` |
| `vessel` | `17ff2578d41a89b24779887fdffffb13e686dd9ffb888d84d622e1f8e6deed44` |
| `voyage` | `3793ee0e12fcefb9da2dfe444508d8f86ff2783b9232f45585b08f53a6f8aba0` |

The build exited 0; all six checks exited 0. The build/check interval after lock
admission was 15:41:27–15:45:05 UTC (including an initial Cargo lock wait). Five
checks completed before the handoff request. The final check finished at 15:45:05;
the PTY subsequently reported exit 0 and `lslocks` no longer listed our holder.
The account owner and coordinator were notified; no further target writes or
builds were launched. This timing is not provider latency or a productivity score:

| Existing check | Observed result and independently inspected output |
| --- | --- |
| `conversation_files.py` | Exit 0: actual read/patch, surrounding text preserved, retained follow-up context and cleanup. |
| `command_workflow.py` | Exit 0: unchanged three-row sales input, exact JSON apples 500 / oats 480 / total 980 cents, stdout/command exit/canonical reply agree, command process gone. |
| `stop_continue.py` | Exit 0: inference connection actually closes on cancellation, interrupted history retained, new run/incarnation completes with context, matched durable cleanup. |
| `two_voyages.py` | Exit 0: two live owners overlap actual file writes, distinct retained histories and cleanup. |
| `client_reconnect.py` | Exit 0: real plain Helm detaches without cancelling the owner, another client reconnects and continues; no network-failure or visual test. |
| `delivery_recovery.py` | Exit 0: scoped/principal refusal, exact identity/payload conflicts, stopped-owner resolution, durable non-admission across restart, lost-response exactly-once behavior, overlapping event polling, approval expiry/denial/cancellation and observed cleanup. |

Cleanup precision matters: the stop/continue and delivery-recovery shared fixture
requires matching durable stopped evidence. Two-voyage and reconnect teardown
observes process disappearance/reaping but does not explicitly assert every durable
cleanup field. Do not generalize either into external-effect rollback. Parent
verification separately reread the unchanged sales input and exact JSON result,
and verified 1 stop/continue plus 12 delivery-recovery `stopped.json` records against
their registrations: session and incarnation matched and cleanup was observed.
Command-workflow cleanup recorded supervisor reaped, no remaining voyage/command
process and provider stopped; two-voyage cleanup recorded no owned PIDs, supervisor
exit 0 and stopped provider thread.

No general suite was recreated. See the [quality guide](quality.md) for existing
focused checks. Checkout availability matters: the root has intentionally deleted
`scripts/check-quality` and `tests/concurrent_voyages.py`; their tracked copies in
the isolated worktree do not authorize restoring or invoking those retired root
entrypoints. `docs/cutover.md` and `eval/run.py` are absent and were not restored.

## Objectively checked current-run work

### Coding and data transformation outside a repository

A one-off `work/reconcile.py` reconciled `work/incidents.csv` in the ignored evidence
directory, not the source tree. The six input rows were:

```csv
id,service,minutes,credit_cents
a,api,12,125
b,db,7,240
a,api,12,125
c,api,5,-25
d,db,invalid,100
e,cache,0,0
```

Required behavior: deduplicate the repeated `a`, reject malformed `d`, retain the
negative credit and zero-valued cache entry, aggregate by service, preserve input.
The utility reported 4 accepted records, 1 duplicate and 1 rejected record. A
separate in-memory SQLite ledger of the four expected accepted records independently
computed the totals; explicit assertions compared the complete service map,
accepted count, duplicate line 4 and rejection line 6. Result: **24 minutes,
340 cents**, with API 17/100, DB 7/240, cache 0/0. Input hash verification passed.
This checks the seeded requirements, not arbitrary CSV robustness or a production
incident diagnosis. Conflicting-duplicate and negative-minute branches were not
exercised; no broad test suite is added.

| Artifact | SHA-256 |
| --- | --- |
| `work/incidents.csv` | `0f9f5207487d7cb88bdea505556e9d62cca8ce1ff3335ec6723d04ada54c876c` |
| `work/reconcile.py` | `5017cabe349adf7dcdcd88ce64b0b2bf6b2a5cc111c91fccc56639cf66e07aea` |
| `work/summary.json` | `ad744eb0b9afc1712f03e3d10d8bd7ea7ff1c037f8bd65da405d45ec35fc2dfb` |

### Administration and interruption

Read-only diagnosis explained why the evidence build had not started: no build log
existed yet, and `lslocks` subsequently showed one WRITE holder and WRITE* waiters
on the agreed build lock. Once admitted, the build log also reported a Cargo build
directory lock wait. No lock was bypassed, no competing process was killed, and no
service configuration was changed. This is a verified local contention observation,
not a claim to diagnose every concurrent owner's work.

Owned terminal `bcb62976-cb88-4990-ab5a-67fbbb697496` started a bounded Python
120-second sleep and reported PID 3353446. `process.interrupt` was followed by a
read showing exit code 1, Interrupt and KeyboardInterrupt; `/proc/3353446` was then
absent. The terminal was explicitly terminated. The interruption was expected,
not successful completion of the sleep. This single-process exercise does not
prove detached descendant cleanup, private human-input fidelity or crash recovery.
Those are separate from #32's terminal implementation work.

### Local research, delegation and writing

Read-only agent `44a6c952-b8d6-46b0-8619-b3aeab936ce3` completed a bounded source
comparison. Its result was read and incorporated, rather than equating agent
completion with correctness. Parent inspection confirmed:

- `voyage/tests/client_reconnect.py:199–243` checks SIGINT Helm detach while the
  same run/incarnation remains active, then another client and retained history.
- `voyage/tests/delivery_recovery.py:163–195` observes process disappearance and
  requires session/incarnation-matched durable cleanup evidence.
- `voyage/tests/command_workflow.py:194–251` checks output bytes, input preservation,
  canonical tool/reply agreement and command-process start identity/disappearance.
- The old `eval/README.md` called the quality guide “non-test checks,” although
  focused checks now exist. This delivery corrects that statement without restoring
  the removed evaluator.

One delegated limitation was corrected explicitly: the agent used the root-style
Git wrapper inside a linked worktree and could not establish Git status. Ordinary
`git status --short` in that worktree succeeded; the parent's root inspection also
confirmed deleted root entrypoints. The agent's observation that those files exist
in the worktree is not evidence that they exist in the root checkout.

Writing deliverables are this scoped report and the corrected evaluation guide.
Acceptance is factual attribution, explicit evidence classes, retained failures,
no human gate, no invented provider results, working relative links and no unrelated
file changes. Source/diff/link/manifest checks, not an authored PASS marker, verify
these bounded requirements. This is not an aesthetic/visual acceptance claim.
No deliberately failed child was spawned simply to manufacture a passing negative
case; failed-subordinate synthesis remains unverified by this delivery.

### Current native Vessel operations

`vessel.routes` identified this owner and only the `local` target. Capabilities
reported Linux, protocol 1, version 0.1.0, Vessel
`6e50b524-7ee0-4126-9288-28815e6ab1df` and current supervision/lifecycle features.
Feature advertisement is not proof that every feature was exercised.

The owner inspected coordinator `04d1e095-9507-4f99-b801-3384ef983cbd`, used its
observed revision/incarnation/run to send the disjoint-scope steering message with
command `9d60fd5f-8d27-4b39-b49f-e77539232189`, and later queried that exact receipt.
The first response was queued; the later receipt was **applied**, canonical index
186, revision 226. No replacement command or blind replay was used. The coordinator
subsequently confirmed the reservation. This demonstrates one actual local
coordination path; it is not remote HTTPS, participant execution, transfer or
comprehensive deduplication evidence. No unrelated voyage was cancelled or archived. During the later build handoff,
steering the coordinator after it had finished was definitively refused. A fresh
inspection showed suspension and observed cleanup; a new status submission
`e0cb8851-f2c6-40a1-ae7c-1b0acb1db56b` was accepted. This was a deliberate new
follow-up after definite refusal, not replay of an uncertain command.

## Failed attempts and historical evidence retained

- Initial context collection attempted the absent `docs/cutover.md`; the shell
  exited 1. A later glob for fixture-named files found none and exited 2. Neither
  failure established missing runtime functionality.
- An evaluation-guide patch was rejected because its unified-diff line count was
  wrong; no file was written by that attempt. The corrected full-file write was
  subsequently reviewed. This is a corrected authoring failure, not first-attempt
  editing success.
- The research agent's worktree Git-wrapper failure and its correction are retained
  above. Failed/unrun commands are not folded into passing checks.
- Historical #80 local Ministral evidence delivered four structured question
  results but one final reply misinterpreted a custom answer as unavailable.
  Eight complete HTTP 200 responses reported 42,437 input and 156 output tokens;
  a prior interrupted request had unknown usage, not zero. This remains a semantic
  failure, not four-case correctness or human-trial success.
- Historical #83 evidence includes a narrow clean completion, malformed accounting
  calls, provider chat-template failure, timeout and failed follow-up. The later
  counted CSV oracle is not the original supplied-number task. #83 was closed as
  superseded/not planned, not as a passing semantic evaluation; its removed forced
  reconciliation mechanism must not be restored to satisfy #22.
- Historical subscription HTTP 401 observations retain their recorded scope.
  #170's later three native ChatGPT OAuth replies with remembered/reloaded history
  are narrow adapter evidence, not universal current account failure or broad
  supervised-workflow acceptance. No silent subscription-to-paid-API fallback.

## Remaining executable scope — not human gates

- Representative bounded live-model acceptance across these categories, failed
  child/partial-result synthesis, conflicting-source reasoning and consequential
  false-success detection remain unverified as a controlled model evaluation.
  Existing current-run success and scripted fixtures cannot substitute for that.
  A new provider/account or separately run live suite still needs an approved
  provider and explicit budget; this work makes no such calls.
- No approved isolated remote HTTPS deployment was available. Deployment-specific
  TLS/proxy behavior, participant admission/revocation/membership changes, unknown
  remote admission/effects/cleanup and owner transfer remain unrun here. Production
  infrastructure was not repurposed. Local features/fixtures are not remote proof.
- Configuration/session migration and retained identity under all supported legacy
  states, crash/restart adverse workflows, native macOS/Windows behavior and a
  representative rollback/fallback task are not newly established by this report.
  Do not confuse ordinary cancellation/continuation with abrupt-process recovery.
- Concurrent #10/#14/#18/#21/#32 owners retain credential/audit, interface,
  installation, decision-semantics and terminal implementation scope. Their issue
  states and independently published evidence must be read before broader claims.
  #19's readiness documentation is reused, not duplicated.

These are concrete unrun or dependent checks. A human review appointment, trial
clock, visual sign-off or default-tool switch is neither requested nor needed to
continue implementation. Preserve this distinction when updating or closing #22.
