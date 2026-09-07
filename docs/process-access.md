# Process access, participants and owner transfer

These commands address independent voyage processes through Vessel. The private
supervisor, grant gateway, participant execution and transfer courier are implemented
for Linux. Linux evidence does not establish native macOS or Windows behavior.
Configuration, provider credentials and execution policy remain on the executing host.
See [operations](operations.md) for ordinary connected sessions and
[security](security.md) for the policy and sandbox boundary.

## Scoped remote access

A local account can use its private Vessel socket. SSH access delegates the remote
account's authority, not an enrolled session grant. A grant credential instead binds
one principal, session UUID, canonical workspace, explicit rights and expiration.
Enrollment alone never grants process execution. An optional machine/epoch binding
also checks the existing enrollment database at admission and execution dispatch.

Run the private service before enabling the HTTP gateway. The gateway uses the
existing enrollment listener behind a local HTTPS proxy:

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
limits frames and concurrent requests, and returns `Cache-Control: no-store`.

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

Both accounts must independently pin the other Vessel's public identity. Retrieve
`helm connect ... admin identity` on each host, verify the exchanged UUID and public
key through an authenticated channel, then run `admin trust /path/to/peer.json` on
the other. The courier never automatically trusts a key supplied in an artifact.
These are public identities, not bearer credentials.

Create a private local directory for the courier journal. Inspect the source to
obtain its exact incarnation and revision, then run:

```sh
helm connect --directory /home/alice/.local/state/voyage/vessel admin move SESSION_UUID \
  --incarnation SOURCE_INCARNATION --expected-revision SOURCE_REVISION \
  --destination-ssh bob@destination \
  --destination-directory /home/bob/.local/state/voyage/vessel \
  --workspace /home/bob/work/project \
  --config-path /home/bob/private/voyage.toml \
  --journal /home/alice/private/move.json
```

Omit `--destination-ssh` for a second local Vessel. Destination service discovery is
explicit; start it beforehand. The source connection can also use the ordinary SSH
options. Grants do not authorize this account administration workflow.

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

The local v1 protocol uses a four-byte big-endian length and JSON payload, bounded
to 4 MiB, over an owned private Unix socket. The HTTP adapter accepts the same typed
`VesselRequest` at `POST /v3/process/command`, with Bearer authentication and the
`X-Voyage-Grant` UUID header. `outcome_unknown` distinguishes uncertain dispatch from
definite initial refusal. Successful envelope receipt is not proof of run cleanup.

Supervisor registrations and command IDs, grant hashes/credentials, accepted
participant bindings/assignments, trusted Vessel keys, signed transfer preparations
and recovery evidence live beneath the private Vessel directory. Do not copy these
files between unrelated installations or edit cleanup/transfer state to bypass a
fence. Unknown owners and unresolved obligations retain capacity. Defaults bound
connections to 64, grants/bindings/assignments/transfer retention to 4,096 entries
where applicable, and lifecycle command retention to 65,536 records.
