# Vessel-managed Helm sessions

**Status: agreed design; not implemented by this documentation change.**
Tracking epic: [#77](https://github.com/o-psi/voyage/issues/77).
All commands, configuration names, and wire operations below are proposed unless
explicitly described as current. This is not an installation guide for a released feature.

## Full-system delivery supersedes phased scope

The operator now requires the **full production system, not an MVP**. The
[production contract](attachment-production-contract.md) supersedes the MVP,
“later” and post-MVP delivery exclusions below while preserving their safety
requirements. Full session lifecycle, supported-platform services, opt-in remote
approvals and secure notification/handoff remain acceptance requirements. The
historical phase descriptions below explain design evolution, not reduced scope.
Nothing in this scope change makes an unimplemented command available.

## Current implementation versus target

Legacy `helm --voyage`, `vessel pair`, and HTTP task polling have been removed.
There is currently no Helm-to-Vessel connection mode. Vessel retains health/readiness,
metrics and an authenticated status UI, not fleet or session management. The new
attachment commands in this document remain unimplemented.

The operator explicitly approved a clean break before replacement delivery,
superseding the original preserve-old-clients requirement in #77/#9/#10. See
[retirement and data preservation](vessel-connectivity-retirement.md). Existing local
sessions and legacy enrollment/database files are left intact but old credentials
and queued/running task snapshots are not loaded or executed. Do not reinterpret old
pairing strings as join keys.

## Ownership and terminology

- **Enrollment:** one-time association of a local Helm installation with Vessel.
- **Connection:** renewable authenticated outbound channel; not a conversation.
- **Session:** durable Helm-owned conversation, with independent visibility policy.
- **Run:** one model/tool execution within a session.
- **Command:** identified remote request with its own delivery and outcome state.

Helm owns canonical conversation history, provider continuation state, model
selection, workspace/access policy, approvals, tools, compaction and branching.
Vessel owns operator authorization, machine inventory, routing, command/audit
history, cached permitted metadata and remote UI state. Provider credentials and
opaque provider continuation state never go to Vessel. Session content can contain
sensitive data even when it contains no credentials.

Helm initiates every network connection; no inbound task/session port is required.
Local roots, deny rules, approvals and resource limits remain final. Application
policy is not an OS sandbox.

## Enrollment and CLI

The primary proposed entry point is:

```sh
helm attach https://vessel.example.com JOIN_KEY --name devbox
```

Here `attach` joins the **installation**, not the current conversation or a PTY.
A namespaced `helm vessel join` alias is a possible discoverability aid; final alias
and flag spelling belongs to the CLI delivery issue. Existing direct terminal
attachment retains its separate [input/lifecycle contract](terminal-attach.md).

Proposed invitation and secret-safe alternatives:

```sh
vessel joins create --name devbox --expires 10m
helm attach https://vessel.example.com                 # secure key prompt
helm attach https://vessel.example.com --join-key-stdin
```

Positional secrets can leak through shell history and process listings; prefer the
prompt or stdin. A scripted invitation producer must output only the key on stdout
before it can safely be piped into stdin; never mix explanatory output with secrets.

Enrollment must:

1. Require authenticated, authorized invitation creation in Vessel.
2. Use a high-entropy, short-lived, one-use key scoped only to enrollment, with a
   verifier hash stored by Vessel and atomic redemption.
3. Require HTTPS except explicit loopback development; do not forward credentials
   to another origin through redirects.
4. Load/create persistent local identity and exchange the join key for a separate,
   scoped worker credential. Store secrets in an OS credential store or owner-only
   local storage; keep them out of logs, prompts, transcripts and diagnostics.
5. Handle a lost redemption response without blindly reusing a consumed key or
   creating duplicate machines; define a bounded authenticated recovery path.
6. Advertise name, version, capabilities and non-secret effective policy, subject
   to disclosure controls, then start an outbound connection.

Rotation, revoke, enrollment expiry and explicit re-enrollment must be supported.
A saved credential reconnects without another join key. Changing Vessel identity or
URL must not silently transfer authority. Initial scope is one enrolled Vessel per
local identity; multi-Vessel ownership is deferred pending conflict semantics.

## Lifecycle

V1 `attach` runs in the foreground; no implicit daemon, service installation or
privilege escalation. Later proposed operations include:

```sh
helm attach https://vessel.example.com --install-service
helm vessel status
helm vessel reconnect
helm vessel detach
```

These examples are not implemented commands. Service installation is explicit and
must define OS-user identity, config and credential ownership on each platform.

- Browser closure or network loss does not itself cancel an accepted run. Helm
  continues within existing budgets, retaining bounded replay state.
- New offline commands must have explicit queue/deadline semantics; the UI cannot
  report execution merely because Vessel accepted a command.
- Foreground Helm exit cancels/reaps its owned live work and persists recoverable
  history. A service can outlive the TUI but cannot resurrect dead PTYs.
- After process/host restart, unrecoverable runs become interrupted and terminal
  metadata becomes stale; never automatically rerun uncertain tool effects.
- Detach/revoke stops new remote authority and active remote work safely, invalidates
  the credential/channel, and preserves local sessions. Offline revocation cannot
  instantly stop a disconnected process; document this limitation and bounded
  authorization/connection checks rather than promising remote erasure.

## Session operations and concurrency

MVP: list visible metadata, create a managed session, submit turns, stream assistant
text and structured tool status, inspect usage/status, and cancel an active run.
Next: open/resume, rename, branch, archive and explicitly confirmed deletion.
Model selection affects subsequent turns and cannot expand local policy.

A Helm-owned coordinator must arbitrate local TUI/CLI and remote clients. Atomic
file replacement alone does not prevent two processes from losing each other's
session edits. Use a single owning runtime or equivalent cross-process locking and
revision enforcement; the exact IPC mechanism remains an implementation choice.
Only one active mutating run may own a session, while separate sessions may execute
concurrently within installation-wide limits. Busy/conflict responses are explicit.

Persist accepted user turns and partial assistant output on failure/cancellation.
Show connection health separately from session/run status: idle, busy, blocked,
completed, cancelled, failed or interrupted must not be confused with offline.

## Outbound command/event contract

Target transport: outbound WebSocket for interactive latency, with semantics
independent of transport. Legacy HTTP polling has been removed; no replacement
transport is available yet. Session commands are distinct from retired queued
one-shot tasks.
Conceptual operations include `session.list`, `session.create`, `turn.submit`,
`turn.cancel`, `output.delta`, `tool.started`, `tool.completed` and `turn.completed`;
these are not existing API endpoints.

Versioned envelopes carry connection, command, session and run IDs as applicable,
expected session revision and per-session event sequence. Negotiate supported
versions/capabilities and fail unsupported operations clearly.

- Persist command deduplication across reconnect/restart; duplicate submission must
  not create two turns. Do not promise exactly-once external side effects across crashes.
- Reject stale mutating revisions without overwriting newer history.
- Acknowledgment of delivery, acceptance and confirmed completion are distinct.
- Cancellation is scoped to a run and handles late events/results deterministically.
- Resume from a cursor; if bounded replay has expired, return snapshot-required.
  Snapshots obey the same sharing policy and exclude provider continuation state.
- Bound frames, queues, output, replay retention, retries and timeouts; slow clients
  must not consume unbounded memory or silently lose authoritative state.
- Reauthorize after reconnect and reject expired commands/revoked credentials.

## Sharing and retention

**Default: new Vessel-managed sessions only.** Enrollment does not list, upload or
control pre-existing local sessions. Existing sessions need explicit per-session or
installation-level opt-in, with a disclosure preview and revocation path. Helm
checks visibility on every operation, not just on inventory listings. Branches of
private histories cannot become a backdoor for disclosure.

Visibility and retention are separate decisions. Proposed `session_sync` modes:

| Mode | Intended disclosure/retention |
| --- | --- |
| `none` | No remote session content or metadata access |
| `metadata` | Only explicitly allowed metadata; no conversation content |
| `metadata-and-live-events` | Default for managed sessions: metadata and live content with bounded delivery replay, not indefinite transcript caching |
| `full-transcript` | Explicit consent to durable permitted transcript copies in Vessel |

Define replay/cache TTLs, storage bounds, backup handling and UI disclosure before
implementation ships. A newly connecting viewer cannot demand old content forbidden
by the selected policy. Titles, paths and tool status can also be sensitive; minimize
and redact them. Direct human PTY input is never synchronized into model-visible or
session records. Remote PTY control is outside this session MVP.

Unshare, archive, local deletion and deletion of Vessel caches are different actions.
Require confirmation for destructive operations, retain non-secret audit attribution,
and explain backup retention. Already delivered data cannot be guaranteed retractable
from an untrusted recipient. Detach does not mean delete.

## Approvals and operator authorization

V1 clearly denies approval-required actions rather than waiting on an invisible
prompt. Enrollment, sharing or sending a prompt never promotes access to unrestricted.
Existing explicitly configured unattended grants remain a local policy decision;
attachment must not silently change them.

Post-MVP remote approvals require local opt-in, authenticated scoped principals,
finite expiry and single-use decisions bound to the exact immutable action, workspace,
session and run. Changed actions require new approval. Local/remote decision races,
cancellation, revocation and unavailable approvers resolve safely with attribution.
Capabilities distinguish view, submit, cancel, approve writes, approve commands and
administer enrollment. No grant bypasses mandatory local restrictions. Possible
`remote_approvals`, `approval_principal` and `approval_timeout_seconds` configuration
names are proposals, not supported settings. Notification/mobile handoff is separate
from authority to decide an approval.

## Delivery map and verification

| Issue | Outcome | Dependencies / phase |
| --- | --- | --- |
| [#77](https://github.com/o-psi/voyage/issues/77) | Overall managed-session delivery | Epic |
| [#10](https://github.com/o-psi/voyage/issues/10) | Join-key enrollment and credentials | First; coordinate handshake with #9 |
| [#9](https://github.com/o-psi/voyage/issues/9) | Versioned interactive command/events | Contract before runtime/UI integration |
| [#78](https://github.com/o-psi/voyage/issues/78) | Helm session execution coordinator | #9; integrate #79 before exposure |
| [#79](https://github.com/o-psi/voyage/issues/79) | Sharing, retention, authorization | Coordinate #9/#10; required before UI exposure |
| [#14](https://github.com/o-psi/voyage/issues/14) | Vessel session UI | #9/#10/#78/#79 |
| [#18](https://github.com/o-psi/voyage/issues/18) | Explicit service lifecycle | After foreground MVP |
| [#21](https://github.com/o-psi/voyage/issues/21) | Delegated remote approvals | Post-MVP; #9/#10/#78/#79 |
| [#72](https://github.com/o-psi/voyage/issues/72) | Notifications/mobile handoff | Coordinate #21; not MVP |

Existing focused issues are reopened with follow-up acceptance criteria; completed
broad foundation epics remain closed. Every delivery issue includes test expectations.
Unit and integration fixtures must cover enrollment expiry/replay/races, rejection of retired clients and new-protocol
version compatibility, local/remote contention, duplicate commands, disconnect at
acknowledgment boundaries, cancellation, crashes, partial output, storage failures,
backpressure, revocation and policy/sharing isolation. Browser tests cover auth,
CSRF, escaping, stale/offline states, retention disclosures and confirmations.
CLI/storage/service behavior needs Linux/macOS/Windows evidence and explicit platform
limitations. End-to-end fixtures must exercise real Helm/Vessel boundaries with a
controlled provider, including no inbound Helm port and no implicit existing-session
exposure. Run repository quality gates for implementation; record actual results,
not intention. Documentation-only planning does not claim runtime verification.
