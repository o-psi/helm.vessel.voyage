# Dedicated remote sessions

A foreground Helm worker can expose one newly created managed session to its enrolled Vessel owner. The operator can list that session, inspect its current state, submit a turn against an explicit revision, poll progress, and request cancellation. Tools, providers, credentials, policy and cleanup stay on Helm. Helm opens the outbound connection; it needs no inbound task port.

This is an explicit single-owner mode. Ordinary `helm attachment connect` remains presence-only. Existing private sessions are not selected, imported or shared. The remote API provides no creation, model change, filesystem-root change, approval delegation, recovery, or transcript-history operation. Local crash recovery is documented below. Consent metadata is not an execution grant.

Every Helm session is a [voyage](voyages.md), including a local chat or this
dedicated remote session. Planned multi-Helm participation extends voyages across
an explicitly scoped set of Helms, with separate interface, coordinating and
executing roles. This worker does not implement that distributed coordination or
the Helm interface for remote work. Its fixed workspace/model binding does not
make a repository or component map part of the voyage definition. Browser console
work is deferred.

## Start the foreground worker

First complete [dedicated enrollment](attachment-cli.md). Start Vessel with its normal private enrollment authority and operator credential, adding `--remote-execution`. Its loopback listener still needs the documented local TLS proxy for a non-loopback origin. The insecure-loopback flag is development-only.

On Helm, select the normal provider configuration, access mode and local workspace:

```sh
helm --config /absolute/provider.toml --workspace /absolute/project \
  remote-worker --directory /absolute/dedicated-remote-installation \
  --enrollment-directory /absolute/enrollment --origin https://vessel.example
```

The dedicated directory contains a separate installation identity and SQLite journal. The first invocation creates one new session. Subsequent invocations reopen only the session bound to the same destination, enrolled machine, owner and epoch, and require its original workspace and model. Another directory deliberately identifies a different local installation. A private journal containing other sessions cannot be relabelled as a new remote session.

The worker holds the managed execution fence while idle, running and cleaning up. Ctrl-C cancels current work and awaits managed cleanup. If owned-resource cleanup or its durable confirmation is unconfirmed, the worker exits nonzero and retains the admission blocker. Transport loss or revocation stops new dispatch, cancels the current turn conservatively, and retains truthful durable state. A reconnect uses a fresh proof and connection generation; it never replays a tool or invents a new command ID. Enrollment rotation or reassignment requires an explicit new binding rather than silently moving this session.

Native providers and built-in tools use ordinary configured local policy, including the administrator ceiling and explicit unattended approval setting. A required human approval fails explicitly. Effectful MCP configurations and the compatibility bridge are rejected before admission because their resource cleanup is not yet observable through this coordinator. Linux has the owned-process cleanup implementation; other platforms retain conservative unsupported/unconfirmed behavior and were not natively tested for this delivery. Application policy is not an OS sandbox.

## Withdraw a dedicated grant locally

`helm remote-consent` operates locally without loading provider configuration or
contacting Vessel. It requires an existing absolute dedicated installation. Inspect
its exact session ID and grant revision, then preview a permanent withdrawal:

```sh
helm remote-consent --directory /absolute/remote-installation inspect
helm remote-consent --directory /absolute/remote-installation preview \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision 0
helm remote-consent --directory /absolute/remote-installation withdraw \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision 0 \
  --confirm CONFIRMATION_DIGEST
```

Choose a fresh operation UUID and preserve the exact request and preview digest.
Review the destination origin, machine, owner, enrollment epoch and local session
before confirming. Grant revision zero means the fixed grant has not been withdrawn;
it is separate from the canonical session revision that changes during work. The
digest binds this exact request and destination; it is not a credential. A wrong or
stale confirmation makes no change. After an uncertain acknowledgement, repeat the
identical withdrawal command to recover its immutable receipt, or inspect locally.
Another operation cannot replace that receipt.

The committed withdrawal permanently retires this exact grant. It atomically fences
later remote admission (including receipt observation), metadata/replay reads and new
public event publication, and requests cancellation of any active run. The worker
checks current durable authority before dispatch, disconnects and performs bounded
owned cleanup after authority loss. It exits nonzero to report that the requested
remote operation no longer has authority. The immutable receipt records the last
public cursor committed before retirement; no later public event can be committed. Cancellation requested is not proof that
effects stopped: inspect `pending_cleanup_run` and the local run state/cleanup projection, and use the existing local recovery,
cleanup attestation and unknown-tool reconciliation workflow when needed. Canonical
failure/cancellation and cleanup evidence can still persist after withdrawal.

Withdrawal cannot recall bytes already read or queued for delivery before its commit,
Vessel's bounded in-flight delivery buffers, or external copies. It suppresses subsequent
public outbox commits, including late provider output and terminal/cleanup events;
remote observers therefore must not infer a final outcome from a stalled stream.
Canonical history, prior local public events and command receipts remain on Helm.
No transcript deletion, backup expiry or historical retention guarantee is implied.
The retained Journal enforces retirement; deliberately restoring an older complete
backup or modifying storage as its owner is outside this local trust boundary.

Restarting the worker cannot restore this grant. Deliberately authorizing future
remote work requires a different explicit dedicated installation directory and a new
empty session; this does not copy or disclose the retired history. General positive
sharing/import, recipient-specific consent and coordination-role activation remain
unimplemented. The general consent declaration library remains inert metadata.

Stop older workers before explicitly upgrading an existing journal:

```sh
helm managed --directory /absolute/remote-installation upgrade
```

Upgrade requires no active run and all execution fences; recover interrupted work
first if needed. It preserves history, bindings and receipts. Older writers are
refused after the schema changes. A failed withdrawal transaction leaves its previous
grant unchanged and does not leave a partial cancellation intent; preserve the request
and resolve the reported storage/lock problem before an explicit identical retry.

## Authenticated operator requests

Use the existing Vessel operator credential. Every endpoint checks it; POST requests also require `x-voyage-request: 2`, and supplied browser origin/site headers must match the configured origin. Do not put credentials in URLs. Successful session-content responses use `Cache-Control: no-store`.

Vessel normalizes `--public-origin` once for enrollment, attachment and remote
authorization. For example, `HTTPS://VESSEL.EXAMPLE:443/` becomes
`https://vessel.example`. A supplied `Origin` header must contain that exact
canonical origin, without a trailing slash. Foreign, duplicate and malformed
origins remain forbidden; forwarded host headers do not select the trusted origin.

`GET /v1/diagnostics` reports enabled connectivity separately from current connections. The following routes require a current connection that negotiated managed execution, not a presence-only connection:

- `POST /v1/remote/MACHINE_UUID/command`
- `GET /v1/remote/MACHINE_UUID/events?session_id=SESSION_UUID&after=0&limit=32`

The POST JSON envelope contains `command_id` (UUID), `expires_at_ms` (Unix milliseconds, at most five minutes ahead for a new request), and `operation`. Supported operations are:

```json
{"type":"list","after":null,"limit":20}
{"type":"inspect","session_id":"SESSION_UUID"}
{"type":"submit","session_id":"SESSION_UUID","expected_revision":0,"prompt":"Describe the requested work."}
{"type":"cancel","session_id":"SESSION_UUID","run_id":"RUN_UUID"}
```

Replace UUID placeholders with actual identifiers. A list returns only the dedicated session's metadata. Inspect returns its authoritative current SQLite revision, latest run state, cumulative usage, cleanup status and public cursor. It never returns canonical conversation, provider continuation state, tool arguments/results, local paths, or direct human terminal input. Vessel fills the authenticated machine, owner and current connection fields itself.

Preserve the original command ID, deadline and operation when retrying an uncertain submit or cancel. A changed request under the same ID is rejected. An identical submit observes its original run, even after a later run or an expired original deadline; it cannot allocate another executor. A successful receipt observation does not mean that run completed. Cancellation acknowledgment means the intent was recorded, not that effects stopped. Inspect the exact run outcome and cleanup evidence.

Progress uses a separate contiguous public cursor, not filtered private journal sequence numbers. Replay pages are bounded to 128 events and a frame budget; the retained suffix has count and byte limits. Receivers replace cumulative usage and deduplicate by cursor. A `snapshot_required` response means inspect the current snapshot and continue from its latest cursor. Disconnecting an HTTP observer does not cancel work; loss of the worker's Vessel connection does, as described above. Vessel keeps no transcript cache; polling requests bounded replay directly from Helm.

Unknown or private session IDs return a bounded `result` envelope with
`reply: {"type":"denied","code":"unauthorized"}`. A cursor ahead of the
current public journal returns the same envelope with `invalid_request`; inspect
the dedicated session and retry from its reported cursor. In these refusal
responses, `command_id` correlates the replay request ID, using the existing v2
result vocabulary. Malformed HTTP cursors/limits are rejected before dispatch.
Rejected observations do not replace the worker connection, create a run or
cancellation intent, or stop already-admitted work. Actual authority, journal and
transport failures still fail closed. Clients must accept a denied result as well
as `replay` and `snapshot_required` when polling.

Text is provisional until the durable terminal outcome. `completed`, `incomplete`, `cancelled`, `failed` and `interrupted` describe execution; cleanup is independently `pending`, `unconfirmed`, `observed` or `operator_attested`. Only `observed` confirms the coordinator's owned-resource observation. Cleanup events may follow a terminal event under the explicitly negotiated managed-execution feature. No terminal-looking agent event is promoted to durable success before the journal's terminal transaction.

When an external SQLite connection temporarily blocks terminal persistence, the
owner retries only that terminal transaction for up to two seconds after obtaining
its local store lock. Each retry requires a typed Busy error and confirmation that
no transaction remains open; provider requests and tool effects are never repeated.
Cancellation is checked again for every attempt. If persistence still fails, the run
can remain active and cleanup is not reported as observed; use the local recovery
workflow below. Withdrawal continues to block public events during these retries.


Configured secrets are removed at the transactional public-text projection boundary. A suffix that might complete a secret remains private until later text or terminal flushing. Public text and its raw offset commit together, and replay never applies redaction again. Unresolved suffixes are withheld if crash recovery has no initialized redactor. This does not promise detection of unknown secrets or arbitrary encoded/fragmented transformations of a credential.

The HTTP relay retains up to 4,096 immutable mutation fingerprints per connection. Reusing a mutation ID for changed content is rejected even after its HTTP waiter times out or disconnects. At capacity, exact retries remain available; stop and restart the foreground worker to establish a fresh connection before issuing new mutation IDs. List, inspect and watch polling use fresh transport IDs and do not consume this limit. At most 32 generations are retained, and retired generations are periodically removed; delayed responses from them cannot satisfy a current connection's waiter. These relay records contain digests and identifiers, not prompts, and do not replace Helm's durable receipts or current authorization checks.

A named policy profile can be selected with the normal `--policy-directory`,
`--policy-profile`, revision, digest and required confirmation flags before
`remote-worker`. Its current snapshot is rechecked inside admission and at
dispatch boundaries, including inherited child authority. A stale/deleted profile
refuses new admission; inspection and exact-run cancellation remain available
under the current connection lease. Recovery accepts no profile selection.

## Recover locally after a crash

Forced process death can leave an active run and an unresolved cleanup obligation. No new turn may dispatch through that blocker. Stop the foreground worker, then use the same private installation directory:

```sh
helm remote-worker --directory /absolute/dedicated-remote-installation --recover
```

Recovery is config/provider/enrollment-connection independent. It holds the execution fence and records `interrupted`; it does not claim processes survived or that cleanup was observed. An active owner or wrong installation identity is rejected.

After independently establishing that the selected run's effects and owned processes have stopped, the local operator can explicitly attest:

```sh
helm remote-worker --directory /absolute/dedicated-remote-installation --recover \
  --acknowledge-cleanup RUN_UUID
```

The attestation records the verified local installation actor in the same transaction as the immutable cleanup acknowledgment. It is distinct from an observation and is never available over the remote API. An exact retry does not rewrite its provenance.

Use `helm managed --directory /absolute/dedicated-remote-installation list` to read the current local revision and cleanup blocker without loading provider configuration. If the canonical history has unanswered tool calls, explicitly append unknown outcomes before another turn:

```sh
helm remote-worker --directory /absolute/dedicated-remote-installation --recover \
  --reconcile-tools RUN_UUID --expected-revision REVISION
```

This requires the latest terminal run and completed cleanup. It preserves existing history and adds fixed unsuccessful/unknown results without executing tools or asserting rollback. The receipt records the local recovery actor, not an impersonated remote owner. Inspect the current snapshot to observe the advanced revision: old terminal/tool events remain unchanged and no successful tool-finished event is invented.

## Storage and validation boundaries

Fresh dedicated journals use schema 8, including permanent local grant withdrawal. Existing local schemas 6 and 7 remain readable for local recovery; the current remote worker requires schema 8 before connecting. Upgrading is explicit and requires process quiescence, no active run and all execution fences. Upgrading never transfers private history into remote authority. Old writers fail after the schema changes. There is no automatic operator-state migration or supported schema downgrade; preserve the original private stores when rolling back a binary.

The worker's durable grant observer and its Journal commits share a short process-local
mutex so ordinary authority polling cannot interrupt the same owner's valid transaction.
Only that owner connection defers dirty-page spill until commit; SQLite can otherwise
acquire its exclusive lock earlier during a large write ([SQLite cache-spill behavior](https://www.sqlite.org/pragma.html#pragma_cache_spill)).
Dirty-page memory can exceed the usual cache target until the bounded transaction ends;
existing snapshot, output and database capacity limits remain in force. This is not a
process memory sandbox. Provider/tool/network work holds no observer/commit mutex.
Independent-process contention still returns a typed Busy error; no mutation is
silently retried and no cached positive consent replaces the current durable check.
After cleanup and disconnection, the worker retries only Busy/Locked authority reads
with a cancellable delay; it reconnects only after a fresh successful observation.
Permanent withdrawal or malformed authority evidence ends the worker instead.

Tracking: [#9](https://github.com/o-psi/voyage/issues/9), [#77](https://github.com/o-psi/voyage/issues/77), [#78](https://github.com/o-psi/voyage/issues/78), [#79](https://github.com/o-psi/voyage/issues/79). These broader issues remain open for their excluded session lifecycle, approval, sharing and product surfaces.

`tests/system/remote_session.py` runs real Helm and Vessel binaries against isolated native OpenAI Chat, Responses and Anthropic HTTP fixtures. It checks file effects, pre-effect durable cleanup registration, exact retries, restart observation, cancellation, split-secret/Unicode replay, forced-death recovery and local attestation, revocation, selected-profile denial/freshness, failed-publication rollback, private-scope denial, rejected replay during active work, and nonzero worker exit after unconfirmed cleanup both during Ctrl-C and ordinary task completion. Journal/runtime tests cover transactional projection rollback, schema compatibility, receipt fencing, tool invocation identities and local recovery attribution. Offline fixtures do not establish live provider or deployment compatibility; final Linux baseline evidence is reported with the PR. These fixtures use no paid/live provider calls. Routine quality gates are Linux-only;
native-platform behavior requires separate evidence.

`tests/system/remote_withdrawal.py` exercises local confirmation, durable retirement,
late provider output, cancellation, owned effects, crash/attestation/reconciliation,
lost output acknowledgements and deliberate fresh empty grants across native providers.
An external RESERVED writer also forces an explicit Busy withdrawal; authenticated
transport loss then settles the run before the identical retry, which must retain
an immutable receipt without claiming a cancellation target.
Journal tests cover transaction ordering, immutable receipts, malformed bounded metadata,
quiescent schema upgrades and owner/observer contention during small and large writes.
