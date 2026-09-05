# Dedicated remote sessions

A foreground Helm worker can expose one newly created managed session to its enrolled Vessel owner. The operator can list that session, inspect its current state, submit a turn against an explicit revision, poll progress, and request cancellation. Tools, providers, credentials, policy and cleanup stay on Helm. Helm opens the outbound connection; it needs no inbound task port.

This is an explicit single-owner mode. Ordinary `helm attachment connect` remains presence-only. Existing private sessions are not selected, imported or shared. There is no remote creation, model change, filesystem-root change, approval delegation, local recovery capability, or transcript-history API in this release. Consent metadata is not an execution grant.

## Start the foreground worker

First complete [dedicated enrollment](attachment-cli.md). Start Vessel with its normal private enrollment authority and operator credential, adding `--remote-execution`. Its loopback listener still needs the documented local TLS proxy for a non-loopback origin. The insecure-loopback flag is development-only.

On Helm, select the normal provider configuration, access mode and local workspace:

```sh
helm --config /absolute/provider.toml --workspace /absolute/project \
  remote-worker --directory /absolute/dedicated-remote-installation \
  --enrollment-directory /absolute/enrollment --origin https://vessel.example
```

The dedicated directory contains a separate installation identity and SQLite journal. The first invocation creates one new session. Subsequent invocations reopen only the session bound to the same destination, enrolled machine, owner and epoch, and require its original workspace and model. Another directory deliberately identifies a different local installation. A private journal containing other sessions cannot be relabelled as a new remote session.

The worker holds the managed execution fence while idle, running and cleaning up. Ctrl-C cancels current work and awaits managed cleanup. Transport loss or revocation stops new dispatch, cancels the current turn conservatively, and retains truthful durable state. A reconnect uses a fresh proof and connection generation; it never replays a tool or invents a new command ID. Enrollment rotation or reassignment requires an explicit new binding rather than silently moving this session.

Native providers and built-in tools use ordinary configured local policy, including the administrator ceiling and explicit unattended approval setting. A required human approval fails explicitly. Effectful MCP configurations and the compatibility bridge are rejected before admission because their resource cleanup is not yet observable through this coordinator. Linux has the owned-process cleanup implementation; other platforms retain conservative unsupported/unconfirmed behavior and were not natively tested for this delivery. Application policy is not an OS sandbox.

## Authenticated operator requests

Use the existing Vessel operator credential. Every endpoint checks it; POST requests also require `x-voyage-request: 2`, and supplied browser origin/site headers must match the configured origin. Do not put credentials in URLs. Successful session-content responses use `Cache-Control: no-store`.

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

Progress uses a separate contiguous public cursor, not filtered private journal sequence numbers. Replay pages are bounded to 128 events and a frame budget; the retained suffix has count and byte limits. Receivers replace cumulative usage and deduplicate by cursor. A `snapshot_required` response means inspect the current snapshot and continue from its latest cursor. Browser disconnection does not cancel work. Vessel keeps no transcript cache; polling requests bounded replay directly from Helm.

Text is provisional until the durable terminal outcome. `completed`, `incomplete`, `cancelled`, `failed` and `interrupted` describe execution; cleanup is independently `pending`, `unconfirmed`, `observed` or `operator_attested`. Only `observed` confirms the coordinator's owned-resource observation. Cleanup events may follow a terminal event under the explicitly negotiated managed-execution feature. No terminal-looking agent event is promoted to durable success before the journal's terminal transaction.

Configured secrets are removed at the transactional public-text projection boundary. A suffix that might complete a secret remains private until later text or terminal flushing. Public text and its raw offset commit together, and replay never applies redaction again. Unresolved suffixes are withheld if crash recovery has no initialized redactor. This does not promise detection of unknown secrets or arbitrary encoded/fragmented transformations of a credential.

The HTTP relay retains up to 4,096 immutable mutation fingerprints per connection. Reusing a mutation ID for changed content is rejected even after its HTTP waiter times out or disconnects. At capacity, exact retries remain available; stop and restart the foreground worker to establish a fresh connection before issuing new mutation IDs. List, inspect and watch polling use fresh transport IDs and do not consume this limit. At most 32 generations are retained, and retired generations are periodically removed; delayed responses from them cannot satisfy a current connection's waiter. These relay records contain digests and identifiers, not prompts, and do not replace Helm's durable receipts or current authorization checks.

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

Fresh dedicated journals use schema 7. Existing local schema 6 remains supported; upgrading is explicit and requires process quiescence, no active run and all execution fences. Upgrading never transfers private history into remote authority. Old writers fail after the schema changes. There is no automatic operator-state migration or supported schema downgrade; preserve the original private stores when rolling back a binary.

Tracking: [#9](https://github.com/o-psi/voyage/issues/9), [#77](https://github.com/o-psi/voyage/issues/77), [#78](https://github.com/o-psi/voyage/issues/78), [#79](https://github.com/o-psi/voyage/issues/79). These broader issues remain open for their excluded session lifecycle, approval, sharing and product surfaces.

`tests/system/remote_session.py` runs real Helm and Vessel binaries against isolated native OpenAI Chat, Responses and Anthropic HTTP fixtures. It checks file effects, pre-effect durable cleanup registration, exact retries, restart observation, cancellation, split-secret/Unicode replay, forced-death recovery and local attestation, revocation, and private-scope denial. Journal/runtime tests cover transactional projection rollback, schema compatibility, receipt fencing, tool invocation identities and local recovery attribution. Offline fixtures do not establish live provider or deployment compatibility; final Linux baseline evidence is reported with the PR. No paid/live provider calls or native-platform jobs are required for this slice.
