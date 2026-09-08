# Process access, participants and owner transfer

These commands address independent voyage processes through Vessel. The private
supervisor, grant gateway, participant execution and transfer courier are implemented
for Linux. Linux evidence does not establish native macOS or Windows behavior.
Configuration, provider credentials and execution policy remain on the executing host.
See [operations](operations.md) for ordinary connected sessions and
[security](security.md) for the policy and sandbox boundary.

## Workspace pairing for human Helm connections

Use the [Vessel connections guide](vessel-connections.md) to connect from inside
an already-running Helm. `vessel pair-invite` issues a short-lived invitation bound
to the desktop principal, approved canonical workspaces and explicit rights.
Redemption over `/v1/vessel/pair` produces a distinct versioned workspace credential;
it does not reinterpret or broaden the session grants below. Workspace grants are
checked at gateway dispatch and by each running voyage's authority watcher.
`vessel revoke-connection` is the local owner's revision-bound revocation operation.
The gateway admits at most 32 distinct SSE subscriptions per request; Helm rotates
bounded subscription groups. Provider credentials remain on the executing host.

## Scoped remote access

A local account uses a private bearer credential to its Vessel's literal-loopback
HTTP endpoint. The credential is published as `process-http.json` in the owned 0700
Vessel directory, is never placed in command arguments, and changes on service
start. Commands use bounded POST requests. Helm opens an authenticated SSE response
for durable invalidations, then retrieves canonical snapshots and output through
ordinary commands. A grant credential for remote HTTPS access instead binds one
principal, session UUID, canonical workspace, explicit rights and expiration.
Enrollment alone never grants process execution. An optional machine/epoch binding
also checks the existing enrollment database at admission and execution dispatch.

Run the private loopback HTTP service before enabling the scoped HTTPS gateway. The
gateway uses the existing enrollment listener behind a local HTTPS proxy:

```sh
vessel local-serve --directory /home/alice/.local/state/voyage/vessel \
  --voyage-binary /home/alice/.local/bin/voyage

# Set VESSEL_OPERATOR_TOKEN through the service's private environment.
vessel --bind 127.0.0.1:8080 \
  --attachment-directory /home/alice/.local/state/voyage/enrollment \
  --process-directory /home/alice/.local/state/voyage/vessel \
  --public-origin https://vessel.example.com
```

Only literal loopback development origins can use HTTP, with
`--allow-insecure-loopback`. The listener still binds a literal loopback address.
The gateway validates an optional browser Origin against the configured origin,
limits frames, SSE subscriptions and concurrent requests, and returns
`Cache-Control: no-store`. Gateway request-body collection has a ten-second total
deadline before dispatch; this deadline does not cancel admitted commands or SSE
streams. Public deployment also needs the proxy controls and remaining release
gates in the [first-release audit](security.md#first-release-audit-2026-09-08). SSE carries bounded metadata-only invalidations and
keepalives. A slow or disconnected Helm cannot block canonical writes; it reconnects
from a durable cursor and uses a snapshot after a retained-event gap. Stream loss
does not imply cancellation and never causes command replay. WebSockets are not used
for control or observation; private bidirectional PTY attachment remains a separate
transport concern.

Issue a credential on the executing host. Replace all example UUIDs with the actual
session and independently selected principal identities. The credential output must
be in an existing private directory; do not put its contents in a command argument.

```sh
vessel process-grant \
  --directory /home/alice/.local/state/voyage/vessel \
  --output /home/alice/private/session-access.json \
  --session 11111111-1111-4111-8111-111111111111 \
  --principal 22222222-2222-4222-8222-222222222222 \
  --workspace /home/alice/work/project \
  --endpoint https://vessel.example.com \
  --rights observe,history,execute,steer,decide,cancel --ttl-seconds 86400

helm connect --access-file /home/alice/private/session-access.json list
helm connect --access-file /home/alice/private/session-access.json
```

Transfer the credential only through a private authenticated channel. Helm requires
an owned private regular file, rejects credentials embedded in URLs, and does not
follow HTTP redirects. Provider keys are not part of the credential. Grant issuance
retains an immutable `.pending-grant.json` request for `process-grant --resume` with
the same `--directory` and `--output` if the response is lost.

`observe` is the default right; canonical text requires `history`. The other rights
are `execute`, `steer`, `decide`, `cancel`, `lifecycle` and `terminal`. Private terminal
operations require both `terminal` and `execute`. Configuration changes, GitHub
administration, imports, recovery and owner-transfer administration require account
owner authority. Registry tool execution still passes the executing host's policy
and approval checks. A grant cannot broaden those checks.

Revoke using the grant UUID from the private credential and its current revision:

```sh
vessel process-revoke --directory /home/alice/.local/state/voyage/vessel \
  --grant 33333333-3333-4333-8333-333333333333 --expected-revision 1 \
  --command-id 44444444-4444-4444-8444-444444444444
```

Runtime admission and ongoing execution recheck grant revision, expiry, revocation
and optional enrollment epoch. Revocation requests cancellation of owned execution;
cleanup is recorded only after it is observed. A revoked client cannot infer that a
previously transmitted command was refused: retrieve its durable receipt using
currently authorized access. Reusing an ID with a changed payload is rejected.

## Participant setup

A participant receives a bounded subordinate task under a local binding. It runs a
separate child session with its own UUID. The parent remains the canonical owner and
records the assignment before network admission, then records the attributed result
and cleanup. Children are excluded from the ordinary top-level catalogue. Only
explicitly selected conversation text crosses the boundary; provider continuation
state and executing-host credentials do not.

On the participant, issue a grant scoped to the **parent** session UUID, using the
participant's accepted local workspace, with `observe,history,execute,cancel`. The
parent UUID need not name a local root session there. Keep that credential on the
parent's executing machine. Prepare a reviewed binding JSON file on the participant:

```json
{
  "binding_id": "55555555-5555-4555-8555-555555555555",
  "revision": 1,
  "parent_vessel_id": "66666666-6666-4666-8666-666666666666",
  "parent_session_id": "11111111-1111-4111-8111-111111111111",
  "principal_id": "22222222-2222-4222-8222-222222222222",
  "workspace": "/home/bob/work/accepted-project",
  "config_path": "/home/bob/private/participant.toml",
  "max_context_bytes": 32768,
  "max_assignments": 4,
  "expires_at_ms": 1800000000000,
  "revoked": false,
  "cancel_existing": false
}
```

Set `expires_at_ms` to an actual future Unix millisecond timestamp within thirty
days. Obtain the parent's public Vessel UUID with `admin identity`. Accept the file
using the participant account:

```sh
helm connect --directory /home/bob/.local/state/voyage/vessel admin \
  accept-participant /home/bob/private/binding.json \
  --command-id 77777777-7777-4777-8777-777777777777
```

Add this configuration on the parent executing host, using the recipient's actual
Vessel UUID and binding identity:

```toml
[[participants]]
name = "builder"
credential_file = "/home/alice/private/participant-access.json"
participant_vessel_id = "88888888-8888-4888-8888-888888888888"
binding_id = "55555555-5555-4555-8555-555555555555"
binding_revision = 1
```

The authoritative runtime tool registry exposes `participant` with `submit`,
`observe`, `cancel` and `list` actions when configured. Submission contains a task
and selected `user`, `assistant` or `tool` context. It requires parent execution
policy and approval when applicable. The child intersects portable parent limits
with participant-local policy and workspace; it cannot configure away that ceiling
or recursively use the parent's participant endpoints.

An unknown admission blocks replacement assignments. Cancellation sends an exact
assignment fence: even if the recipient has not admitted the request yet, it records
a terminal tombstone excluding a late submission. A binding can be removed with
`admin remove-participant BINDING --command-id UUID --expected-revision N`; existing
work drains by default, or `--cancel` withdraws its derived execution authority.

After a parent restart, reconcile the retained obligation without starting a new run:

```sh
helm connect --directory /home/alice/.local/state/voyage/vessel admin \
  assignment 11111111-1111-4111-8111-111111111111 \
  --run RUN_UUID --assignment ASSIGNMENT_UUID --participant builder
# Add --cancel to fence and cancel that exact assignment.
```

Unresolved participant obligations block new parent runs, archive, deletion and
transfer. Local operator attestation cannot manufacture remote cleanup evidence.
Parent run cleanup cancels outstanding subordinate execution through its owned
cancellation context; Helm disconnection does not end that run. The automatic
observer runs for up to one hour; later reconciliation remains
available through the command above. Task text is limited to 8 KiB, selected context
to 64 messages and the binding's byte budget (at most 32 KiB), and each parent run
retains at most 256 assignments. Lists contain bounded metadata; complete attributed
results are stored separately with a 1 MiB bound.

## Signed ownership transfer

The Helm courier currently supports two local Vessels accessible to the executing
account. Cross-machine courier transport is unavailable after SSH removal; scoped
HTTPS grants do not authorize owner-transfer administration.

Both Vessels must independently pin the other's public identity. Retrieve
`helm connect --directory /absolute/vessel admin identity` for each, verify the
UUID and public key through an authenticated channel, then run
`admin trust /path/to/peer.json` on
the other. The courier never automatically trusts a key supplied in an artifact.
These are public identities, not bearer credentials.

Create a private local directory for the courier journal. Inspect the source to
obtain its exact incarnation and revision, then run:

```sh
helm connect --directory /home/alice/.local/state/voyage/vessel admin move SESSION_UUID \
  --incarnation SOURCE_INCARNATION --expected-revision SOURCE_REVISION \
  --destination-directory /home/alice/.local/state/voyage/second-vessel \
  --workspace /home/alice/work/destination-project \
  --config-path /home/alice/private/destination-voyage.toml \
  --journal /home/alice/private/move.json
```

Destination service discovery is explicit; start the second local Vessel beforehand.
Grants do not authorize this account administration workflow. Old journals with a
non-null SSH destination are refused without rewriting them or contacting that
destination. Do not change a pending remote journal into a local operation;
reconcile its original outcome on the executing hosts.

The destination validates local configuration/policy and reserves capacity before
signing a preparation. The idle source requires resolved cleanup, permanently
fences further admission, increments its ownership generation, and signs the
portable checkpoint's identity and digest. The courier transfers bounded chunks;
the destination verifies the pinned source signature, preparation, complete digest
and generation before admitting the same session UUID. A portable command-ID ledger
returns transferred dispositions for retained commands and rejects changed payloads
without replay. Canonical text is preserved;
provider continuation state, live resources and source configuration are excluded.

Repeat the **same command with the same journal** after connection loss. The journal
retains exact command IDs, endpoint identities, preparation and manifest. Chunks and
activation tolerate exact retries. Do not invent a new transfer ID to escape an
unknown outcome. The source stays relinquished and cannot restart after ownership
moves. There is no automatic timeout takeover, reverse transfer, or expiry-based
reclamation of an unresolved preparation. A preparation lasts at most thirty
minutes for initial relinquishment; the Helm courier uses a five-minute command
admission window; committed transfers remain recoverable by exact
identity. Portable artifacts are bounded to 16 MiB and chunks to 64 KiB.

## Cleanup in a surviving owner

The snapshot's optional `cleanup` object reports the latest run's `run_id`,
`phase` (`running`, `blocked` or `observed`), `pending` component labels, an authored
interruption `reason`, and whether the live owner can retry. It is diagnostic
progress; `pending_cleanup_run` and resource observations remain authoritative.
Older snapshots omit this object. New process startup marks unfinished progress
from a previous lifetime as blocked and not retryable by that new lifetime.

The original voyage observes cleanup asynchronously, retaining task handles across
bounded observation windows. A new, valid Submit can request another bounded
cleanup batch without admitting the message while cleanup remains unresolved.
A rejected Submit keeps its rejected receipt; use a new command ID and current
revision for a later explicit send. An uncertain Submit must still be resolved
under its original identity. Reading progress does not send a draft or repeat work.

## Unavailable-owner recovery

Use `admin recover SESSION --incarnation OLD --command-id UUID` for a registered
unavailable process. The separate recovery executable acquires the same OS session
fence and refuses a live owner. It does not construct providers or executors.
`--acknowledge-cleanup RUN_UUID`, repeated `--acknowledge-resource RESOURCE_UUID`,
and `--reconcile-tools RUN_UUID --expected-revision N` identify explicit operator
dispositions; tool effects are never replayed. A recovery response lists pending run/resource UUIDs and the current revision when
obligations remain. Use a new explicit command ID for the reviewed dispositions.
Only resolved obligations permit restart. `recovered.json` distinguishes operator attestation from observed cleanup;
`stopped.json` continues to mean runtime-observed stop. Preserve the exact recovery
command ID when retrying an uncertain response.

## Wire and retained state

The public Vessel service API uses JSON over HTTP(S), bounded to 4 MiB. Commands
use `POST /v1/vessel/command`; durable invalidations use an SSE response from
`POST /v1/vessel/events`. Both use `protocol: 1`, defined by `VESSEL_API_VERSION`
independently of the private runtime's `PROCESS_PROTOCOL`. The public definitions
are in `crates/voyage-protocol/src/vessel.rs`; private execution messages remain in
`crates/voyage-protocol/src/process/`.

Local requests authenticate with the private service credential. Scoped HTTPS
requests add the grant Bearer credential and `X-Voyage-Grant` UUID header. The private
Vessel-to-voyage connection still uses length-prefixed JSON over an owned Unix
socket, with runtime token and session/incarnation authentication.

`resolve_start` resolves the exact original creation command ID, session, workspace
and optional host configuration path without creating or restarting anything.
It returns an observed `created` result, a durable `not_admitted` fence, or `unknown`
for unconfirmed evidence. Delayed Starts with fenced IDs are refused. Workspace
clients require `create`, their approved workspace, and no arbitrary config path;
legacy clients retain their exact session scope. Older servers without the
`start_resolution` capability cannot prove absence of admission this way.

Public session operations are explicit top-level commands, not a `forward` envelope.
For example, a submission is:

```json
{
  "protocol": 1,
  "command": {
    "op": "submit",
    "session_id": "11111111-1111-4111-8111-111111111111",
    "command_id": "44444444-4444-4444-8444-444444444444",
    "expected_revision": 0,
    "expires_at_ms": 1800000000000,
    "prompt": "Describe this workspace"
  }
}
```

Use the actual session revision and a current command deadline. Vessel selects the
session's owner under lifecycle arbitration and resumes it only when permitted.
Helm does not inspect and select a runtime before ordinary reads or submission.
`execute_tool`, `terminal`, `cancel`, `steer` and `respond` additionally require the
exact observed `incarnation`; stale or missing identities are refused. `stop` keeps
its explicit lifecycle identity requirement. Session reads and new turns follow the
current owner even if an optional old incarnation is supplied.

The public operation list includes snapshot, history/message chunks, run output,
events, receipts/resolution, submission, steering, cancellation, decisions, tools,
workflows, terminal interaction and configuration/lifecycle controls. Management
operations include catalogue, start, inspect, restart, stop, grants, participants
and owner transfer. Private runtime initialization, health and relinquishment are
not exposed through an arbitrary command tunnel. Adding an internal runtime command
does not add a public operation: Vessel's explicit adapter and authorization mapping
must be updated separately.

A successful session response has this shape:

```json
{
  "protocol": 1,
  "result": {
    "session_id": "11111111-1111-4111-8111-111111111111",
    "incarnation": "22222222-2222-4222-8222-222222222222",
    "result": { "status": "accepted" }
  },
  "error": null,
  "outcome_unknown": false
}
```

The operation result shown is illustrative; admission receipts also identify the
command and run. There is no embedded `RuntimeResponse`, private protocol version,
runtime token or runtime authorization binding. Errors and `outcome_unknown` appear
only in the outer service response; authoritative rejection details remain in its
`result`. A successful command response is not proof of completed execution or
cleanup. Vessel never acknowledges admission before the runtime's durable receipt.
Operation-specific result bodies remain JSON projections rather than fully typed
schemas for every tool/control surface.

SSE requests contain `protocol` and a list of `{session_id, incarnation, after}`
subscriptions. The incarnation records the client's last observation; Vessel selects
the current owner. Updates identify the observed owner. An owner transition sets
`owner_changed` and requests snapshot recovery; cursors remain session-scoped.
Retained-event gaps likewise require an authorized snapshot. Clients reconnect after
transport/observation failures without resubmitting commands.

This replaces the former `/v3/process/*` public tunnel. Upgrade Helm, the local
Vessel supervisor and any scoped gateway together; restart the service to expose the
new API. Old public endpoints and the pre-HTTP local socket fallback are not served.
Already-running voyage processes retain private protocol v1 and do
not need to be killed for the public API change. Pending Helm operation payloads
retain their serialized names, command IDs, revisions and deadlines, so recovery
continues to resolve the original command rather than replay it. Enrollment-relay
wire protocols and transport-lease authority are separate and unchanged.

Supervisor registrations and command IDs, grant hashes/credentials, accepted
participant bindings/assignments, trusted Vessel keys, signed transfer preparations
and recovery evidence live beneath the private Vessel directory. Do not copy these
files between unrelated installations or edit cleanup/transfer state to bypass a
fence. Unknown owners and unresolved obligations retain capacity. Defaults bound
connections to 64, grants/bindings/assignments/transfer retention to 4,096 entries
where applicable, and lifecycle command retention to 65,536 records.
