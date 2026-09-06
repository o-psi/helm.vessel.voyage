# Coordination control

Vessel can retain explicitly registered voyage identifiers, coordinator and
participant **nominations**, and immutable control receipts. An outbound Helm
connection maintains finite control leases for its current nominations. Every
control record reports `execution_authority: "none"`: registration, configuration,
source rebinding and these leases cannot admit tasks, disclose history, approve
tools, activate a coordinator or establish a handoff.

The foreground control host has no agent executor. Distributed execution and the
unified voyage interface remain planned under the
[coordination contract](voyage-coordination.md). Ordinary local voyages need none
of this setup. Existing [presence](attachment-presence.md) and
[dedicated remote sessions](remote-sessions.md) retain their separate behavior.

## Enable and connect

Use the existing [private enrollment setup](attachment-cli.md). Start Vessel with
`--attachment-directory`, its canonical `--public-origin`, and the additional
`--coordination-control` flag. This enables the negotiated `coordination_control`
feature and independently versioned tables in the private enrollment database.
The existing operator token protects control HTTP operations. HTTPS is required
except for explicitly enabled literal-loopback development.

An enrolled Helm maintains a foreground outbound control connection with:

```sh
helm attachment connect --coordination
```

No inbound Helm port is opened and no provider is loaded. The connection acquires
and renews its control leases automatically; there is no manual renewal command.
JSON notices report current configured records and nominated roles. A lease means
that the authenticated connection currently maintains the nomination, not that
Helm accepted an execution role. Disconnect, credential revocation, expiry, record
revision changes and Vessel restart invalidate the relevant leases. Ctrl+C or
SIGTERM stops this host; it does not revoke durable configuration.

Both peers must negotiate this feature. Older clients can still use ordinary
presence against an enabled Vessel. A control client refuses a server without
control support; it does not silently report successful registration.

## Register an existing voyage

Select an existing session explicitly. Optional absolute `--session-directory`
and `--installation-directory` choose the existing session store and private
installation identity location.

```sh
helm attachment connect --coordination --session SESSION_REFERENCE
```

This previews the exact Vessel origin, owner, machine binding, original session
UUID and source revision. It saves a bounded private registration intent and
prints a confirmation digest, command ID and deadline. It does **not** publish the
registration. The session must be available for exclusive ownership; a busy or
invalid source fails before publication. Its file bytes, canonical text, draft,
provider state and local execution ownership are not transferred or rewritten.
Only the previewed identifiers and revisions are published.

Repeat the same selection with the displayed values:

```sh
helm attachment connect --coordination --session SESSION_REFERENCE \
  --registration-id COMMAND_UUID \
  --registration-expires-at-ms DEADLINE \
  --confirm-registration PREVIEW_DIGEST
```

The initial control record is `configured`, revision 1, with its source Helm
nominated as coordinator and participant. This creates no additional session and
confers no authority on those nominations. The source owner is released after the
registration response; the ordinary local voyage remains usable.

The private `coordination-intents.json` journal lives beside the selected stable
installation identity. Retrying uses its original operation even if the local
session subsequently changed. An uncertain response is not proof of failure:
repeat the same intent to retrieve the immutable receipt. A receipt reports the
original revision; inspect current control state separately.

If the original request was never accepted and its deadline passed, authenticated
receipt observation establishes non-admission under the same serialized clock
watermark as registration. Only then can an explicit fresh preview be requested:

```sh
helm attachment connect --coordination --session SESSION_REFERENCE --new-registration
```

This preserves the prior intent and creates another preview, requiring its own
confirmation. It does not automatically retry a changed operation. Corrupt,
oversized, conflicting or uncertain local intent files are preserved and reported
as errors. Do not delete or edit them to force a new request. Cancellation or a
blocked output can leave a durable intent or an accepted registration with an
unobserved receipt; inspect/retry rather than assuming no publication occurred.

## Inspect, configure and revoke

These operator APIs accept existing enrollment authority, not provider credentials.
Use `Authorization: Bearer` with the operator token. Mutations also require exactly
one `x-voyage-request: 2` header. A supplied `Origin` must equal the canonical public
origin; duplicate authorization, origin and request headers are rejected. Responses
are not cacheable.

- `GET /v2/coordination/INSTALLATION_UUID/SESSION_UUID` returns the current record
  and currently valid control leases, filtered against live authenticated sockets.
- `POST /v2/coordination/command` accepts the strict mutation envelope below.

```json
{
  "command_id": "11111111-1111-4111-8111-111111111111",
  "expires_at_ms": 1800000000000,
  "address": {
    "installation_id": "22222222-2222-4222-8222-222222222222",
    "session_id": "33333333-3333-4333-8333-333333333333"
  },
  "expected_revision": 1,
  "operation": {
    "type": "configure",
    "coordinator": {"machine_id": "44444444-4444-4444-8444-444444444444", "epoch": 1},
    "participants": [{"machine_id": "44444444-4444-4444-8444-444444444444", "epoch": 1}]
  }
}
```

Replace example identifiers with inspected values and choose a deadline strictly
in the future, at most five minutes away. Participants must be unique, currently
enrolled bindings; the nominated coordinator must be among them. The source must
also retain a current enrollment binding. Configuration increments record and
scope revisions; changing the nominated coordinator also increments its control
epoch. These are metadata generations, not execution fencing or handoff evidence.

Use `"operation": {"type": "revoke"}` with a fresh command ID and exact current
revision to revoke the control record. Revocation invalidates its leases and
prevents further configuration or source rebinding. It does not claim to cancel
work owned by another runtime. Revoked addresses and receipts are retained.

Every mutation uses an immutable command ID and exact expected revision. An exact
retry returns its original receipt even after expiry or later changes. Reusing
that ID with different content, or submitting a new command with a stale revision,
is a conflict. Timeout or storage uncertainty requires current-state inspection
and an exact retry; never infer acceptance from a transport write.

## Recover an enrollment binding

Credential rotation or re-enrollment does not silently adopt an old nomination.
An operator can explicitly repair the source binding using the same mutation
endpoint, a fresh immutable command ID and exact current record revision:

```json
{
  "type": "rebind_source",
  "expected_source": {"machine_id": "44444444-4444-4444-8444-444444444444", "epoch": 1},
  "source": {"machine_id": "44444444-4444-4444-8444-444444444444", "epoch": 2}
}
```

This is the `operation` value inside the envelope. A different currently enrolled
machine may be selected explicitly after re-enrollment. Wrong old bindings, stale
revisions, revoked new enrollments and revoked records are rejected. Rebinding
preserves the session address, original registration revision and old receipts,
increments the record revision and invalidates control leases. Coordinator and
participant nominations are unchanged; use a separate explicit `configure`
operation to update them.

A changed enrollment can observe the original registration receipt through its
same authenticated machine identity or an explicitly rebound current source.
It cannot replay the stored registration under a different binding. The local
intent stays unchanged. This recovery transfers no canonical history or execution
ownership and is not coordinator handoff.

## Bounds and recovery limits

One authority database retains at most 64 control records and 4,096 immutable
control mutation receipts, including revoked records. One installation retains
at most 64 registration intents, including superseded unaccepted intents. Each
scope has at most 16 participants. These are lifetime capacities: accepted IDs
are not pruned or reused. Ordinary mutations stop once total receipt use reaches 4,032, reserving 64
additional slots for one final durable revocation per retained record. Ordinary
capacity exhaustion cannot prevent those revocations. Exact accepted retries
remain observable at either limit. Plan
within these limits rather than treating this capability as unlimited storage.

Control leases last 10 seconds and are tied to the current transport, credential
epoch, record revision and server generation. Renewal happens every two seconds.
Vessel restart increments the durable generation and clears old leases; every
host must authenticate again. Unknown schema versions, generation/revision
overflow and malformed bounded records fail without replacing the evidence.
Concurrent storage operations, connection loss, output blockage and shutdown have
bounded waits; uncertain persistence still requires receipt inspection.

Automated fixtures cover the Linux control workflow and adversarial transport,
CAS, storage and recovery cases without provider calls. Native Windows/macOS
control workflow execution is not established by those Linux results.
