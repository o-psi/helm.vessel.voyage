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
| `controls` | `session_id`, `section` (`models` or `policy`) | `run_id` |
| `history` | `session_id` | `offset`, `limit`, `expected_revision` |
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

Page sizes are 1–128 (default 50). Search is case-insensitive catalogue-metadata
search, not full-text search across every conversation. History is the public
revision-bound projection; oversized messages can be marked truncated. Event
follow/wait uses a durable cursor and a 0–30,000 ms wait (default 0), bounded further
by the executing tool timeout. Event replay gaps require a fresh inspection.
Use `snapshot.revision` from `inspect` for mutations, the registration's
`incarnation` for live-resource operations, and `snapshot.run.run_id` for the run.
Do not infer these identities from names. Archived catalogue metadata carries the
revision needed for restore.

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
