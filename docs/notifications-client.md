# Notification client

The CLI/TUI notification slice has offline Linux real-component evidence.
See [verification](notifications-verification.md) for exact results and remaining
#72 scope. The accounting backend integration and broader crash matrix are not
implicitly established by these client checks.

See [architecture](architecture.md) for Helm/Vessel/Voyage authority boundaries
and [validation](quality.md) for verification requirements.

## CLI

Use one selected authenticated Vessel connection:

```text
helm connect inbox destinations
helm connect inbox configure destination.json --command-id UUID
helm connect inbox accept DESTINATION --command-id UUID
helm connect inbox test DESTINATION --command-id UUID
helm connect inbox list DESTINATION --after 0 --limit 50
helm connect inbox watch DESTINATION --after 0 --seconds 60
helm connect inbox inspect DESTINATION EVENT
helm connect inbox seen DESTINATION EVENT
helm connect inbox dismiss DESTINATION EVENT
helm connect inbox revoke DESTINATION --command-id UUID
```

`open` aliases `inspect`. Connection flags (`--directory`, `--no-start`, or
`--access-file`) precede `inbox`; CLI operations require exactly one connection.
Configuration is a local JSON file, read with a 64 KiB bound into the strict
`voyage_protocol::notifications::Destination` type, not arbitrary JSON passed
through to the service. Do not put secrets, transcripts, labels, URLs, shell
commands, or free-form message content in it. Unknown fields are refused.

Destination fields:

| Field | Value |
| --- | --- |
| `id` | Fresh immutable destination UUID |
| `recipient_grant_id` | Explicit current recipient grant UUID |
| `recipient_principal_id` | Recipient principal UUID bound to that grant |
| `recipient_grant_revision` | Observed grant revision |
| `source_vessel_id` | Actual owner Vessel UUID |
| `source_session_id` | Actual source voyage UUID |
| `event_kinds` | Nonempty unique subset of `completed`, `incomplete`, `failed`, `cancelled`, `interrupted`, `attention`, `budget`, `test` |
| `expires_at_ms` | Future UTC epoch milliseconds, no more than 30 days |
| `notification_ttl_ms` | Positive lifetime, no more than 7 days |
| `quiet_hours_utc` | `null` or `{ "start_minute": 1320, "end_minute": 420 }` |

Quiet hours use UTC minutes after midnight, start inclusive, end exclusive;
wraparound is allowed, equal endpoints are invalid. They suppress attention
presentation, not explicit inbox reads or expiry. There is no local timezone or
DST conversion. Passive attention counts use a bounded 15-second probe; there are no unsolicited bells/popups.

Source consent and recipient acceptance are separate: the local source owner
configures disclosure, and the configured recipient explicitly accepts storage
and subscription. Local-recipient configuration uses the host Vessel UUID for
both recipient grant and principal, with revision 1. A remote recipient can list
its own destinations, accept, read, update receipts, inspect and revoke; source
configuration and synthetic test publication require the local source owner.
Vessel, not this client, checks these permissions and current grant validity on
every request. Neither notification references nor command UUIDs grant authority.

Choose and retain command UUIDs before configure/accept/test/revoke. A timeout
or lost response is an unknown outcome, not a refusal: inspect state, and retain
the identical command UUID and input for any explicit retry. Never substitute a
new UUID to replay an uncertain mutation. The client sends each mutation once
and does not retry it automatically. A revoked destination UUID cannot be reused.
Changing consent requires a new destination and separate acceptance.

Ordinary commands print JSON. Destination-list responses contain `destinations`
and `local_recipient_id`; inbox responses contain `page`, `producer`,
`attention_deferred`, `budget_delivery` and `destination_expires_at_ms`. Watch prints one JSON page per line, polls at most
once per second, has a 1–3600 second deadline (default 60), and stops on the first
error or Ctrl+C. Each request has a ten-second client timeout; the overall watch
deadline also bounds an in-flight read. It never retries a mutation. List limits
are 1–100. Resume from the returned `page.next_after`; `page.has_more` means another page
is available. Each inbox response also reports producer status, quiet-hour attention
deferral and destination expiry; the TUI shows producer failures/gaps rather than
treating an empty inbox as success. The cursor tracks original acceptance sequence, not subsequent
receipt updates. Rescan from zero to refresh seen/dismissed state. A quiet empty
page, unavailable owner, expiry, or producer gap is not evidence of task success.

`test` produces a synthetic event only; it never reports a successful source run.
Reading does not implicitly record seen state. Seen/dismissed receipts belong to
the destination and do not attest that a human read anything, resolve an owner
request, cancel work, or prove cleanup. Receipt transitions are monotonic.

## TUI

In an existing voyage's composer, on its selected Vessel connection:

```text
/inbox
/inbox destinations
/inbox list DESTINATION [AFTER]
/inbox open DESTINATION EVENT
/inbox seen DESTINATION EVENT
/inbox dismiss DESTINATION EVENT
```

`/inbox` is discoverable in slash-command completion and opens usage help.
The destination UUID is always explicit: Helm never guesses which subscription
or recipient should be modified. CLI-only setup/acceptance/watch are listed in
that help. Use CLI when no voyage is selected.

Results use the existing passive overview panel, fetched asynchronously with
route-generation and source-view incarnation guards. The command preserves the
composer text; Esc closes the overview, leaving that text available for editing.
It never changes the selected voyage, activates a private terminal, opens a URL,
runs a command, or sends an approval/denial. Existing modal, permission and
private-terminal input routing is unchanged. No background notification steals
focus. A bounded passive metadata-count probe can update the footer without
changing the composer, modal, active voyage or private terminal.

Open fetches the service's fresh owner envelope:
`current`, `resolved_or_expired`, `stale`, `unavailable`, `expired_or_revoked`, or
`decision_authority_unavailable`, optional typed
notification metadata, and an optional authorized exact owner decision. The TUI
renders only typed identifiers/enums/times; unsolicited free-form fields are not
displayed. An included decision must match the event's decision/run/incarnation
reference. The inbox does **not** render a response widget or invoke `Respond`.
To read the full authorized request and approve/deny, explicitly select its owner
voyage and use the existing Questions and permissions flow, which validates the
current owner/request again. Stale/unavailable metadata grants no fallback action.

## Verification scope

Focused tests cover CLI input bounds and required mutation identity,
non-echoing malformed destination diagnostics, watch cursor progression, strict
slash-operation parsing, unsolicited-content exclusion, unknown Open statuses,
and synthetic test wording. All eight inbox-focused tests passed. The real-component
journeys and remaining failure-matrix limits are recorded in
[verification](notifications-verification.md).
