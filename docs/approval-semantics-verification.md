# Approval admission and scoped responder verification

Status: focused Linux evidence for [#21](https://github.com/o-psi/helm.vessel.voyage/issues/21),
not complete remote acceptance. No human testing or product acceptance gate is
imposed by this record. Missing executable/deployment evidence remains missing.

## Architecture and authority

[Helm presents decisions; Vessel authenticates and routes; the independent Voyage
owns them](architecture.md). This change retains those boundaries. It does not
introduce a second approval engine, an executing Helm, or outbound workers.

An executing principal A and an approving principal B may differ. A needs
`execute`; B needs an explicit `decide` grant for the session or its canonical
workspace. Neither execution nor history/observation, steering, cancellation,
coordination, or pairing alone authorizes a response. Default workspace pairing
rights do not include `decide`.

The current supported granularity is one `decide` right for approval categories
and questions together, not distinct filesystem-write and command-approval rights.
Questions remain model input, not authorization for a tool action. This record
makes that implementation limit explicit; it does not manufacture product-owner
acceptance of the historical finer-grained proposal.

### Least-privilege decision access

- `Decisions` and `Respond` require `decide`; listing pending decisions does not
  require a transcript grant.
- `Snapshot`, ordinary `Receipt`, and history require `history`.
- Exact-envelope `Resolve` inherits the right of its original operation and
  enforces the original principal/payload binding. A decider can recover its own
  exact response without broad history or execution rights. It must retain that
  envelope and command identity; a new ID is not recovery.
- `observe` is separately needed for lifecycle/event inspection, and `catalogue`
  for workspace catalogue discovery. The fixture supplies known session identities;
  it does not claim that a decide-only credential supplies every Helm UI feature.

See [process access](process-access.md) and
[connections](vessel-connections.md) for public transport and grant contracts.

## Concrete correction: authority after queued admission

Previously, the last responder authorization check preceded asynchronous command
binding, the admission mutex, the owner store mutex, and decision persistence.
Revocation or expiry during those waits could leave the subsequent response using
an earlier positive authorization result. A post-dispatch transport check could
withhold a reply, but could not undo committed consent.

`server/commands.rs` now supplies the existing runtime authorizer to
`attachment/runtime/decisions.rs`. The callback runs in
`attachment/journal/decisions.rs` **inside the decision write transaction, after
acquiring the owner store**. It rereads the current session/workspace grant chain
and required `decide` right. The deadline clock is sampled there too. No grant,
credential, principal, wire schema, or local execution policy is broadened.
SQLite contention currently fails boundedly (`busy_timeout = 0`); it is not an
indefinite approval wait.

This is a local consent-admission check, **not a distributed revocation/dispatch
transaction**. Grant files and the journal are distinct stores. Revocation after
that check can race a commit. A committed answer remains immutable; subsequent
approver revocation or interface loss does not retrospectively revoke it. The
requester's execution authority, access generation and cancellation are separate
checks around approval consumption. Individual tools have their own effect
boundaries. An `applied` response receipt proves that consent committed, not that
an effect ran, cleanup completed, or uncertain work may be replayed.

### Exact action and review limits

The tool retains its parsed arguments across its approval await. A response selects
an existing decision UUID/run/incarnation and does not supply replacement tool
arguments. Different tool occurrences get different decision identities; an exact
retry recovers the same response instead of dispatching another tool.

The existing preview is a redacted action/target/reason, with worker attribution
where applicable. It is not a full-action digest or complete file-content/patch
review. Two writes to the same path can have identical previews. This delivery
neither claims an argument-substitution defect from that fact nor claims to have
resolved informed-review completeness. Filesystem stale-base and parent/path races,
post-consent execution-authority changes, and crash/effect boundaries still need
the separately enumerated acceptance checks below.

## Executed checks

Implementation commits: `577f784` (admission), `36805b0` (current native fixture),
and `307b495` (unused-browser cleanup). Final checked source:
`307b49503008a9c0223cac058687d42edc4ddf02`, after merging published account/#254 main.
Executing platform: Linux x86-64, Rust 1.98.1 / LLVM 22.1.8. Provider: an explicitly
anonymous native OpenAI Responses loopback fixture; no paid calls, named-account
enrollment or operator credentials. Checks used the shared target and
`issue-wave-build.lock`.

```sh
cargo test -p voyage --locked --lib response_ -j 8
cargo test -p voyage --locked --lib unused_browser_cleanup_needs_no_write_but_fences_still_do -j 8
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/approval_semantics.py --bin-dir /path/to/shared/target/debug
```

The two focused Rust tests passed:

1. `response_authority_and_time_are_checked_inside_write_transaction`: the
   authorizer runs while the journal holds its write transaction; denial leaves
   the request unanswered; the freshly sampled deadline refuses expiry; a timely
   answer survives late observation and an exact duplicate returns its receipt.
2. `queued_response_rechecks_authority_after_owner_lock`: holds the real owner
   mutex, polls the response into its blocking task, invalidates a deterministic
   authority fixture, then releases the mutex. The callback observes invalidation
   and refuses. No sleeps or production fault-injection hooks are used. This is
   an admission-ordering unit check, not a deployed grant-revocation journey.

The Python fixture passed ten cases plus observed cleanup:

| Grant / case | Observed result |
| --- | --- |
| Session, approve | Independent B has decide but not execute/history; one successful write occurrence and expected file content |
| Session, deny | Zero successful write occurrences; no output file |
| Session, competing approve/deny | One immutable response winner; effect count follows that winner |
| Session, revoked approver | Response refused; bounded request expiry; zero effects |
| Session, expired request | Late response refused; zero effects |
| Session, expired approver grant | Response refused; zero effects |
| Session, cancelled run | Response refused; zero effects |
| Workspace, approve | Explicitly paired/pinned workspace decider succeeds without transcript or execution rights |
| Workspace, deny | Zero successful write occurrences; no output file |
| Workspace, competing approve/deny | One immutable winner; no extra successful tool occurrence |

Every case rejects a non-decider despite observe/history/execute/steer/cancel rights,
refuses wrong session/run/incarnation, and refuses snapshot disclosure to a
decide-only credential. Approval cases recover the exact response using `Resolve`,
refuse broad `Receipt`, reject a changed response under the same command ID, and
reject a competing principal reusing that ID. Fixture-owned journal joins establish
requester/responder principal separation without granting public transcript access.
Canonical tool outcomes and actual file existence/content are checked together;
this is not a syscall-level count of filesystem writes.

Private evidence from the final passing merged-source run was retained at
`/tmp/vdr-3mvosmsb`: per-case decision/receipt/history/outcome records, provider
requests, and cleanup observations. The fixture confirms absence of its Voyage
processes and matching durable cleanup records, then observes supervisor and
gateway process exit. Evidence directories contain synthetic fixture credentials
and are not publication artifacts.

### Additional cleanup correction and adverse evidence

An unused browser broker previously wrote an identical empty checkpoint during
`finish_run`. A deterministic second SQLite writer made that call fail with
`DatabaseBusy`, unnecessarily coupling an approval-only run's cleanup to a browser
write. The regression failed on the old code before the correction.

The corrected path skips persistence **only** when active entries, observations and
live waiter guards are all empty; it clears the in-memory run and changes no durable
state. Nonempty observations still require a checkpoint. The negative regression
also creates real dispatched metadata: contention must retain it, and after the
lock is released the fence persists `unresolved` with `cleanup_pending: true`, not
successful cleanup. All three focused Rust tests passed on the final source.

The earlier `/tmp/vdr-q82oecm_` process run remains failed evidence: its revoked
approver case completed and expired its decision but retained a **Tools and
terminals** cleanup blocker. Later guardian process-exit observation did not clear
that journal obligation, and no `stopped.json` was created. Read-only inspection
found an entirely empty browser state, but the per-attempt error was discarded;
therefore the new contention regression does **not prove** that this defect caused
that historical failure. Neither a separate passing reproduction nor the final
passing matrix repairs or reclassifies the original journal.

### Failures found while preparing the checks

- The first post-account-merge matrix was refused before execution because its
  implicit legacy ChatGPT login was retired. The approval fixture now explicitly
  uses anonymous native loopback Responses rather than migrating credentials or
  weakening named-account scope.
- The first gateway launch used an obsolete `--attachment-directory` flag copied
  from an older fixture. The gateway refused startup; the new fixture was corrected
  to use current CLI options. This was not a passing runtime check.
- The initial journal unit fixture omitted process-command table initialization;
  the expiry assertion failed. The fixture initialization was corrected and the
  final two-test run passed.
- The first workspace run omitted the required pinned Vessel identity header and
  timed out waiting for decisions. The fixture now sends `X-Voyage-Vessel` and
  fails immediately on an observation error. The final workspace cases passed.

### Final development binary identity

These SHA-256 values were recorded under the same build lock immediately before
the final matrix. They identify development binaries, not installed HTTPS services.
Helm was built but this API fixture did not exercise its UI.

| Binary | SHA-256 |
| --- | --- |
| `helm` | `a409822fe1ef0ec5c4701603c6a9be4b4999c0edf762e4488b533f1485ceb92f` |
| `vessel` | `7edd7833ad6ecb460c6b3299edcde6b96c9fc1e59aac48eb54aa9b61a1825a4e` |
| `voyage` | `02b05c262475e38e9bae6f153d5d31ccda0cb594b5bbde13e3efbb50d7b5837b` |

## Evidence boundaries and unfinished acceptance

This fixture uses the public **HTTP command gateway on literal loopback**, with
explicit development opt-in. Current Helm uses the authenticated full-duplex
WebSocket with `voyage.vessel.v1`; there is no implied Helm HTTP/SSE fallback.
The matrix is not a Helm end-to-end test, a deployed HTTPS test, or native
macOS/Windows certification. #14 owns the concurrent Helm UI journey evidence.

The coordinator established no authorized isolated deployed HTTPS route for this
run. Historical [installed connection evidence](vessel-connections-verification.md)
remains valid for its recorded scope, not as current cross-principal decision
certification. Do not repurpose production grants/tunnels or disable TLS validation.

#21 remains open for:

- Actual current Helm WebSocket/installed HTTPS decision journeys, binary hashes,
  local/remote interface races, review/paste/narrow/reconnect evidence.
- Independently authorized responses to worker requests; approval review that is
  sufficiently informative without disclosing credentials or private input.
- Deterministic current-grant revocation/expiry at response/effect boundaries,
  including approved-but-not-executed and unknown-effect/cleanup outcomes. The
  unit-ordering fixture and pre-response gateway revocation are not that full matrix.
- Changed file bases/path identities/access, distinct actions with identical
  previews, transport loss and process interruption at request/response/effect
  boundaries, without replay or revival of old decisions.
- Explicit disposition of the historical finer permission granularity and full
  action-digest/informed-review requirements. Existing occurrence binding must
  not be confused with complete presentation of an action.

## Workspace coverage

After the explicit build-slot handoff, the final unchanged source passed **177 Rust
tests, zero failures, one ignored live-provider test**. The full coverage command
and compact measurement are committed in [coverage/latest.json](../coverage/latest.json).
The report uses all 11 current Cargo workspace executables (10 executed test
binaries), 398 unique current source files, zero mismatched functions and no foreign
source paths. Default mixed-artifact output is retained privately, not published
as the measurement. The corrected totals are:

- Lines: 17,692 / 77,670 (22.78%).
- Functions: 1,752 / 7,825 (22.39%).
- Regions: 27,330 / 126,038 (21.68%).

No measurement scope was narrowed. Raw reports, corrected HTML, logs, fingerprints
and binary hashes remain under ignored `target/coverage-report/issue21/`. The
coverage comparison and exclusions are in the compact record. This verified slice
does not close the remaining #21 acceptance obligations above.
