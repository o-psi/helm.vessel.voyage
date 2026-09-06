# Inspect and revoke enrolled devices

An operator can discover enrolled Helms even when they have no live connection,
inspect the enrollment audit trail, and revoke a lost device using its exact
current credential epoch. These observations grant no execution, coordinator,
viewing, steering or approval authority. They do not share voyage history.

Enable the existing [enrollment authority](attachment-cli.md) with Vessel's
`--attachment-directory`, canonical `--public-origin` and operator credential.
Coordination control and dedicated remote execution do not need to be enabled.
Use HTTPS through the configured proxy; explicit literal-loopback HTTP is for
development. See [security operations](security-operations.md) for credential
storage and deployment boundaries.

## Operator requests

The read-only inspection APIs use JSON POST requests, so cursors stay out of URL
query strings. Each request needs exactly one `Authorization` header with the
existing operator Bearer or Basic credential, exactly one `x-voyage-request: 2`,
and `Content-Type: application/json`. A supplied `Origin` must match the canonical
public origin. Ambiguous duplicate headers and cross-site origins are rejected.
The APIs authenticate before reading a request body and return `Cache-Control:
no-store`, including errors.

| Endpoint | Purpose |
| --- | --- |
| `POST /v2/enrollment/machines` | Read durable enrolled identities, including offline and revoked machines. |
| `POST /v2/enrollment/audit` | Read retained enrollment audit events. |
| `POST /v2/enrollment/revoke` | Existing operator revocation, revalidating the exact current epoch. |

Start either inspection with:

```json
{"limit": 100}
```

A page contains `version: 1`, the authority's `owner_id`, results, and `next`.
When `next` is non-null, repeat the same endpoint and page size with that opaque
cursor:

```json
{"limit": 100, "cursor": "CURSOR_FROM_PREVIOUS_PAGE"}
```

Page size defaults to 100 and must be between 1 and 128. Request bodies are limited
to 4 KiB and cursors to 2,048 characters. Cursors are authenticated, bound to the
authority, owner, projection and page size, and expire five minutes after the
initial page. Paging does not renew that deadline. A restart preserves a valid
cursor when its authority, data and deadline are still valid. A cursor from another
authority or endpoint is rejected even when the same operator token is configured.

## Discover an offline device and revoke it

1. Read every machine page. Entries contain only `machine_id`, `epoch` and
   `revoked`; they exclude enrollment keys, invitation material and session data.
   The list is not restricted to live sockets. Use your recorded device IDs or
   compare `helm attachment status` on retained machines to identify the intended
   device. An offline entry alone does not prove a machine is lost.
2. Confirm the target ID and its current epoch. Each page has an inventory
   `revision` derived from durable enrollment, rotation and revocation events.
   If any of those events occurs during pagination, a continuation returns 409;
   restart from the first page instead of combining different snapshots. Ordinary
   connection authentication does not change this inventory revision.
3. Revoke the selected device with the existing endpoint, a fresh transaction ID,
   and its exact inspected epoch:

   ```json
   {
     "machine_id": "11111111-1111-4111-8111-111111111111",
     "expected_epoch": 3,
     "transaction_id": "22222222-2222-4222-8222-222222222222"
   }
   ```

   Inspection is never the authorization check: revocation validates the epoch
   again in its transaction. A concurrent rotation makes the old epoch invalid;
   inspect again and make an explicit new decision. Do not automatically retarget
   a failed revocation to a newer epoch.
4. Inspect the machine and audit trail after revocation. Its durable `revoked`
   state prevents fresh authenticated connections with the revoked credential.
   Existing connection checks also reject invalid epochs/revoked credentials.
   This does not attest that every remote process has terminated; each executing
   Helm retains its own cancellation and cleanup obligations.

If a revocation response is lost, keep its original transaction ID and exact
request for the existing bounded retry procedure. Inventory/audit observations
help establish current state; they do not recreate an expired receipt or repair
an uncertain local enrollment transaction.

## Audit pagination and bookmarks

Audit pages contain `events`, a fixed inclusive `through` sequence, `next`, and
`retention: "all_retained"`. Events contain only sequence, optional machine ID,
kind and timestamp. Kinds are `invitation_created`, `enrolled`, `authenticated`,
`rotated`, and `revoked`. Invitation creation has no machine ID. Events do not
include keys, proof bodies, invitation verifiers, provider state, arbitrary log
text, session history or an invented per-event credential epoch.

The first page fixes `through`. Events appended afterward are excluded from that
pagination run, so later activity cannot shift pages or create duplicates. Keep
the same cursor chain until `next` is null. To inspect later events, start a fresh
request with the last sequence you have retained:

```json
{"limit": 100, "after": 42}
```

`after` is an initial audit bookmark, not a machine-inventory option; it cannot be
combined with a cursor. A bookmark beyond the current end is rejected. Sequence gaps within the requested range, a disappeared cursor boundary,
malformed rows and oversized stored text produce errors rather than silent
omission or replacement.

The current enrollment audit does not prune events. Ordinary audited operations
stop at 100,000 retained events; reserved capacity permits final revocation of the
10,000 maximum retained machines, up to 110,000 events. These are existing lifetime
bounds, not a rolling retention window. Inspection neither appends audit events
nor repairs, deletes or rewrites authority state. Audit pages are observations of
stored history, not a cryptographic attestation of the entire database.

## Errors and limits

Malformed input returns 400; authentication and origin/role denials return 401 or
403; expired or changed snapshots return 409. Oversized bodies return 413.
Unavailable capacity/storage contention returns 503; malformed persisted data
returns 500 without including its contents. Preserve corrupt storage for repair.
A changed or expired cursor requires a new inspection, not alteration of its data.

Authenticated reads share four operator request slots, with a ten-second HTTP
processing deadline; blocking database work retains its separate bounded permit
and five-second response deadline. This prevents disconnected or slow requests
from creating an unbounded database queue. A slow request cannot change authority.

Enrollment's existing lost-response recovery remains limited to five minutes.
After that window, operator repair is still a separate incomplete workflow; these
inspection APIs do not extend old-key recovery, reset pending transactions or
silently re-enable a detached identity. Broader worker/interface delegation and
coordinated authority remain governed by the planned
[voyage model](voyages.md), not by this inventory.
