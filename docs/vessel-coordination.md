# Model-driven Vessel coordination

Voyage's native `vessel` tool lets the model inspect its environment and operate
other voyages through the public Vessel API. Steering and independent voyage
creation are first-class capabilities, not shell recipes or Helm UI automation.

The tool is paired with a bundled [coordination skill](../voyage/skills/vessel-coordination.md).
Voyage adds this guidance to the ephemeral model instructions only when `vessel`
is in that executor's actual registry. It does not create synthetic conversation
messages or require installing an extension package. Workers inherit the tool
only when their delegated registry permits it.

## Product boundaries

- **Vessel** continues to supervise independent voyage processes and route commands.
- **Voyage** owns the tool, policy checks, model interaction, and retained operation
  intents. Vessel does not acquire an embedded agent loop.
- **Helm** presents the tool activity and remains the human interface. Closing it
  does not stop independent work.

There is no artificial related-voyages-only filter. A local route uses the executing
account's Vessel API; configured grant routes use their actual server-side rights.
Configured routes are not implicit machine enrollment and do not extend a remote
grant. Provider credentials stay on the execution host. Tool output never includes
Vessel bearer credentials or a private terminal channel.

## Tool interface

`target` selects a configured alias and defaults to `local`. Every action rejects
unknown arguments. `routes` reports `self_session_id`, whether local discovery is
configured (not a reachability claim), and configured aliases without credential
paths or contents. `capabilities` contacts the selected Vessel and reports its
identity/version and actual route capabilities/rights.

| Action | Required arguments beyond `action` | Optional arguments |
| --- | --- | --- |
| `routes`, `capabilities` | None | `target` |
| `list`, `operations` | None | `offset`, `limit` |
| `search` | `query` | `offset`, `limit` |
| `inspect` | `session_id` | — |
| `details` | `session_id` | `path`, `offset`, `limit`, `expected_revision` |
| `message` | `session_id`, `index`, `expected_revision` | `offset` |
| `run_output` | `session_id`, `run_id` | `offset` |
| `controls` | `session_id`, `section` (`models` or `policy`) | `run_id` |
| `history` | `session_id` | `offset`, `limit`, `expected_revision` |
| `history_search` | `session_id`, `pattern` | `role`, `offset`, `limit`, `expected_revision` |
| `follow`, `wait` | `session_id` | `after`, `limit`, `wait_ms` |
| `receipt` | `command_id` | `session_id` |
| `create` | `command_id`, `session_id`, `workspace`, `task` | `config_path` |
| `submit` | `session_id`, `command_id`, `expected_revision`, `prompt` | — |
| `steer` | `session_id`, `incarnation`, `run_id`, `command_id`, `expected_revision`, `prompt` | — |
| `cancel` | `session_id`, `incarnation`, `run_id`, `command_id`, `expected_revision` | — |
| `rename` | `session_id`, `command_id`, `expected_revision`, `name` | — |
| `archive`, `restore` | `session_id`, `command_id`, `expected_revision` | — |

All network actions also accept `target`. `operations` and a `receipt` without
`session_id` read the owning voyage's private local journal and do not contact a
Vessel. `operations` returns operation metadata, not retained prompts. A receipt
with `session_id` reads the target's server receipt, including stopped archive
receipts. Stored local uncertainty and a subsequently observed server receipt
remain distinct observations, not a rewritten history.

`inspect` returns a compact overview rather than a transcript or provider catalogue.
It retains registration, revision, run state/failure, cleanup and resource observations,
plus bounded latest assistant/live text, message timestamps and the latest turn.
An idle process is not proof of task success; the latest assistant message can belong
to an earlier turn. If no assistant text is in the recent snapshot window, the
overview says so and can show explicitly labeled accumulated run text instead.
Live text is provisional. Large observations are explicitly
marked `detail_omitted` with an exact `read` request; an omitted blocker is unknown,
not absent. If the overview cannot fit even with shorter excerpts, the response
withholds it and offers a one-field detail read. Unavailable snapshots retain
registration separately from the snapshot error.

Follow the overview's `details`, `history.read`, progress `read` and `events` requests
when more information is needed. Recent history starts at the last five messages,
not at the beginning. `details` defaults to the public snapshot's field directory;
its `path` is a JSON pointer, such as `/cleanup`, `/decisions`, `/run` or `/turns`.
Objects page fields, arrays page entries, and strings page redacted UTF-8 bytes.
Large nested values carry their own `read` request. Follow `next_read` exactly.
Detail continuations include the observed revision; changed revisions return
`revision_changed` and a fresh inspection request. Revisions do not freeze live
output, decisions or other transient state: these reads are fresh observations.
Use `controls` for dedicated model/policy metadata.

Page sizes are 1–128 (default 50), with detail pages automatically reduced to fit
the tool output budget. The existing `search` action covers catalogue metadata.
`history` automatically fits a page to the output budget, shortening large messages
into explicit excerpts and reducing the page count when necessary. It preserves
canonical message indices, `next_read` for the first unreturned message and
`previous_read` for preceding messages. When a page fits, complete projected
messages are preserved. Every returned message links to `message`. That action reads the complete public message as JSON text
chunks, including fields omitted from the bounded history projection. Concatenate
`data` before parsing JSON. `run_output` reads the chosen run's accumulated text.
Their offsets count **redacted UTF-8 bytes**, and their exact `next_read` requests
preserve the route and message/run identity. Each call reassembles at most 4 MiB of
source text before redaction, then emits up to 4 KiB of text (less when the output
budget requires it). Larger records return `source_limit`; a run growing during
assembly returns `source_changed`. Neither is a complete read. Public snapshot
fields can already be truncated upstream; use `message`/`run_output` for text
expansion. These reads do not expose private terminal input or configuration secrets.

`history_search` searches **one voyage's redacted message content**, including
human/assistant text and textual tool results. It does not search attachment bytes
or structured tool-call arguments; use the complete message read for those fields.
Patterns are Rust regex, with Unicode and inline flags such as `(?i)` and `(?m)`;
look-around and backreferences are unsupported. Patterns are 1–4096 UTF-8 bytes,
with bounded nesting and compilation/cache memory. An optional `role` filters
`user`, `assistant`, `tool` or `system`; omitted/null searches all roles. Matches
never cross message boundaries. Empty patterns refuse; zero-width patterns work.

Each call scans at most 32 messages, stopping at the requested matching-message
limit or after processing 8 MiB of content (a final complete message can cross that
threshold). It returns one entry per matching message, showing the first match's
redacted byte offsets and a bounded excerpt. `read` expands that message and
`context_read` opens nearby history. `matched_messages`, `unsearched_messages` and
`scanned_messages` describe **this page**, not totals over the whole voyage.
Output budgeting can reduce the match page; the continuation rewinds to the first
withheld entry so it cannot lose a result. Follow `next_read` until `has_more` is
false—even when a page has zero matches. Continuations retain the regex, role,
route and observed revision. There is no full-text index or cross-voyage scan;
select voyages with catalogue search and search each relevant history.

A truncated history projection is expanded before matching, and redaction happens
before regex evaluation and excerpt extraction. A full public message over the
existing 4 MiB source limit produces an explicit `unsearched` entry; its prefix is
not searched as if complete. Accumulate these gaps across pages: any gap prevents
a claim that the complete conversation had no matches. Permission failures and
revision changes stop the read rather than returning an apparently complete result.

```json
{"action":"history_search","session_id":"00112233-4455-4677-8899-aabbccddeeff","pattern":"(?i)error|quota","role":"tool","limit":10}
```

Event follow/wait uses a durable observation cursor and a 0–30,000 ms wait
(default 0), bounded further by the executing tool timeout. Event replay gaps
require a fresh inspection; an ended wait is not completion. Use `snapshot.revision`
from `inspect` for mutations, the registration's `incarnation` for live-resource
operations, and `snapshot.run.run_id` for the run. Do not infer these identities
from names. Archived catalogue metadata carries the revision needed for restore.

Creation paths must be absolute paths on the target host. `config_path` selects
an existing owned private launch configuration; omitted configuration uses target
host defaults. Model, reasoning, service and other launch settings come from that
configuration, not separate `create` overrides. The task/prompt limit is 64 KiB.
A create request needs two distinct fresh UUIDs: `session_id` is also its stable
start-command ID; `command_id` is the initial-submit ID and local operation ID.
All other mutations use a fresh stable `command_id` and the observed revision.
Reusing the same ID and arguments returns the retained result without dispatch;
changed arguments conflict.

Start with actual discovery rather than assumed targets:

```json
{"action":"routes"}
{"action":"capabilities","target":"local"}
{"action":"search","query":"API","limit":10}
{"action":"operations","limit":20}
```

## Coordination semantics

Inspect the target before choosing how to communicate:

- **Steering** supplements an active model run at its next supported safe boundary.
  A steering receipt records admission, not proof of model consumption or task
  completion. Operator runs do not accept ordinary model steering.
- **Submission** starts a new turn in an existing idle or suspended voyage. Existing
  cleanup/admission rules remain in effect.
- **Creation** creates an independent session and submits its initial task. These
  are separate protocol effects, not a distributed atomic transaction. A confirmed
  creation with a refused first turn is still an existing voyage to inspect/reuse.
- **Follow-up observation** reads bounded state/history or waits for a bounded
  interval. Ending a wait never implies that work stopped.
- **Lifecycle operations** use the existing cancel, rename, archive and restore
  protocols, including their revision/incarnation checks and idle requirements.

Use a subagent for bounded work owned by the current run. An independent voyage is
not added to the parent's subagent tree or completion accounting; it may continue
beyond the originating turn. The parent should report a handoff honestly instead
of claiming that accepted work is complete. Shared workspaces do not isolate file
edits: coordinate ownership or use worktrees when concurrent Git changes overlap.

## Reliability and authority

Mutations retain an immutable intent before dispatch. Reusing an operation identity
must not send changed arguments or blindly replay uncertain work. Inspect retained
receipts after cancellation, timeout, restart, or a lost response. A created voyage
is not duplicated just because the first response was lost. Reads do not wake an
executor merely to generate a model response.

Intents, exact per-step wire commands and results live under the caller's private
`resources/vessel-coordination` directory. They survive runtime suspension and
supervisor restart and are removed with that voyage's resources on deletion.
Incomplete writes remain uncertain and cannot trigger replay. No agent-owned
terminal or background polling process is created.

Existing execution access applies: inspection remains available in read-only mode;
mutations are refused there, use the existing decision interface in approval mode,
and proceed without an extra confirmation ceremony in unrestricted mode. A skill
is guidance, not a source of authority. Where an inherited runtime policy ceiling
cannot be propagated through the independent-execution API, create/submit/steer
are refused rather than bypassing that ceiling; policy-bounded subagents remain
the delegation path in that case. Unknown cleanup is not a permission to
recover, attest, or replay work.

## Sender attribution and navigation

New model-originated `create` initial tasks, `submit` messages and `steer` messages
retain structured coordination metadata alongside canonical text: the sending
Vessel/session UUIDs, the session name observed at send time, the command UUID and
the actual assistant tool-call ID. The runtime supplies these values, not model
arguments. Admission and steering application preserve them, including after
suspension. Exact retries do not rewrite the original sender. This is presentation
provenance, **not execution authority or a cryptographic identity assertion across
Vessels**; existing route authentication, rights and target policy still apply.

Helm labels these messages **Sent by _session name_** instead of **You**. Single-click
the underlined name to open the sender's conversation at the exact sending `vessel`
call, with its activity group and tool details expanded. Navigation matches stable
identities, the command and tool-call ID—not names or message
text. Renaming a session does not change old attribution or redirect the link.
Copied canonical messages retain their original source link when a voyage is branched.
Ordinary human submissions and legacy messages without provenance retain their
existing labels; Helm does not guess senders from authored text. Operator-issued
tool operations without an assistant call are not relabeled as model messages.

Navigation only reads through connected, authorized Helm routes. It does not enroll
a connection, restore an archive, restart a voyage or resubmit anything. An unavailable
Vessel/session, denied history, archived/deleted sender, changed revision, missing or
ambiguous call produces an explicit notice. Multiple connections to one Vessel use
the receiving connection when possible; otherwise ambiguity is reported rather than
silently choosing credentials. History is revision-fenced and paged, with a 30-second,
16,384-message / 64 MiB reading budget. Calls absent from the owner's public canonical
history (including subordinate-only activity) cannot be opened by this link.

## Platform and validation limits

The transport uses bounded native HTTP requests to private local loopback discovery
or configured HTTPS grant endpoints (literal-loopback HTTP is allowed for local
remote-gateway development). It does not invoke SSH, Helm, or a shell to route
commands, start an absent supervisor, enroll machines, or copy credentials.
Private local process supervision is currently supported on Linux; native
macOS/Windows behavior and deployed remote TLS require their own evidence.

Build/static checks establish compilation, not coordination quality with a real
model. Issue #186 isolated Linux verification exercised the actual binaries with
synthetic provider credentials: native tool/skill registration, identity and
catalogue paging, creation plus first task, exact duplicate/conflicting requests,
history and follow-up submission, rename/search/archive/restore, queued-to-applied
steering, cancellation, policy introspection, local operation records, grant-scoped
routing, lost mutation and creation responses without replay, independent child
execution after parent suspension, offline receipt reads, and
read-only/disabled-tool refusal. No paid provider or deployed TLS was used.

The steering exercise also exposed and fixed regenerated message timestamps:
durable admission now records the time once, so queued delivery and canonical
application construct identical metadata. Legacy records keep absent timestamps
rather than inventing a historical time.

Issue #222 verification used development Linux binaries and a temporary synthetic
provider fixture: native submit/create/steer attribution, ordinary-human fallback,
exact duplicate preservation, suspension and supervisor restart, and a real Helm PTY
sender click loading history beyond the recent 128-message snapshot and opening the
original expanded call and activity group rather than a later duplicate. An archived
sender was explicitly refused without restoration. The clicks issued no provider requests. All-target compilation also passed. These checks
do not establish deployed remote-grant navigation or native macOS/Windows behavior;
no general automated suite was recreated.
