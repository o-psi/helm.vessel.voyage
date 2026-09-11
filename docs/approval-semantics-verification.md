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

Implementation commit: `577f784`. Checked combined source:
`ccfa30fe97e1bacbc7c1d7c38ce4a59d32c5571a` (includes the published #18/#19
non-Rust changes). Executing platform: Linux. Provider: isolated synthetic HTTP
fixture; no paid calls or operator credentials. Checks used the shared target and
`issue-wave-build.lock`.

```sh
cargo test -p voyage --locked --lib response_ -j 8
cargo build -p vessel -p voyage --locked -j 8
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

Private evidence from the passing patched run was retained at
`/tmp/vdr-n_mzfaa_`: per-case decision/receipt/history/outcome records, provider
requests, and cleanup observations. The fixture confirms absence of its Voyage
processes and matching durable cleanup records, then observes supervisor and
gateway process exit. Evidence directories contain synthetic fixture credentials
and are not publication artifacts.

### Failures found while preparing the checks

- The first gateway launch used an obsolete `--attachment-directory` flag copied
  from an older fixture. The gateway refused startup; the new fixture was corrected
  to use current CLI options. This was not a passing runtime check.
- The initial journal unit fixture omitted process-command table initialization;
  the expiry assertion failed. The fixture initialization was corrected and the
  final two-test run passed.
- The first workspace run omitted the required pinned Vessel identity header and
  timed out waiting for decisions. The fixture now sends `X-Voyage-Vessel` and
  fails immediately on an observation error. The final workspace cases passed.

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

Full workspace coverage and publication are pending the explicitly serialized
build handoff. The #21 coverage waiter was withdrawn before acquiring the lock;
no coverage clean, measurement or report ran. A focused test pass is not a coverage
measurement. Preserve the last published `coverage/latest.json` until a new real
measurement is available.
