# Helm session management through Vessel

**Agreed design with partially implemented foundations.**
Tracking: [#77](https://github.com/o-psi/voyage/issues/77).
The [voyage product model](voyages.md) defines the operator experience: an open-ended
session on one or more user-scoped Helms. Every session, including local chat, is
a voyage; Helm is the program. Interface, coordinator and execution roles may
overlap. This document describes supporting management and lifecycle boundaries;
remote coordination is planned, while local voyages already work.

## Current implementation

Helm supports local chat/run sessions, private managed sessions, enrollment and
outbound authenticated presence. With explicit local configuration,
`helm remote-worker` creates one new dedicated managed session and connects it to
Vessel. Vessel with `--remote-execution` exposes authenticated list, inspect, submit,
replay and cancel operations for that session.

Use the [enrollment guide](attachment-cli.md), [presence guide](attachment-presence.md)
and [remote-session guide](remote-sessions.md) for working commands, HTTP shapes,
privacy limits and recovery. Presence alone cannot execute work or expose sessions.
Existing private sessions are not implicitly imported or shared.

Integration of voyage scope with execution, the unified remote Helm interface,
a remote coordinating agent,
cross-Helm delegation, coordinator handoff and full remote session lifecycle remain
planned. `helm attach` is not an implemented command. The existing Vessel `/ui`
page is an authenticated static status page; browser console work is deferred.

## Ownership and terminology

| Term | Meaning |
| --- | --- |
| Enrollment | Association of a Helm installation with Vessel using scoped credentials. |
| Connection | Renewable authenticated outbound channel, independent of conversation identity. |
| Voyage / session | The same ongoing unit of work, including an ordinary local chat; cross-Helm coordination is planned. |
| Interface Helm | Authorized operator view and steering surface for the voyage. |
| Coordinating Helm | Agent runtime that plans and delegates within voyage scope; may be remote. |
| Participant Helm | Runtime executing assigned work under its local authority. |
| Local session | A local voyage with Helm-owned canonical history, persistence and visibility. |
| Run | One execution within a session, with its own completion and cleanup state. |
| Command | Identified request with separate delivery, acceptance and outcome states. |

A Helm can hold multiple roles. In ordinary local chat, the local Helm supplies
the interface and execution. For planned distributed voyages, opening an interface
does not by itself move coordination to that workstation. A voyage need not have a repository or predefined component map,
and choosing a Helm for one task does not lock later tasks to that machine.
A local voyage already has its session identity and persistence; it is not a
separate entity above a local chat. Distribution of its history and attribution of
participant execution records need an explicit cross-Helm persistence/context
contract. This terminology leaves session/run IDs and wire schemas unchanged; it
does not establish a distributed identifier mapping.

Helm owns tool execution, canonical local state, provider credentials, opaque
provider continuation, model configuration, local policy and approvals. Vessel owns
authentication, permitted machine presence/routing and authorized control-plane
metadata. Executing Helms initiate outbound connections; no inbound Helm task port
is required. Delegation cannot expand roots, policy, grants or resource limits.
Application policy is not an OS sandbox.

## Enrollment and machine scope

Current enrollment uses Vessel-issued one-use invitations and separate persistent
credentials. Follow the current CLI guide for secure terminal/stdin input, expiry,
response-loss recovery, revocation, private storage and explicit re-enrollment.
Changing Vessel identity must not silently transfer authority.

Enrollment is not voyage membership, session sharing or approval delegation. Planned
Helm management must let the user select permitted Helms when starting a voyage
or extend an existing voyage's scope, and inspect scope changes. A user can name a permitted execution target or let the coordinator
choose within scope. The proposed [scope-edit contract](voyage-coordination.md#5-scope-edits-while-work-exists)
requires explicit drain/cancel disposition and separate enforcement/cleanup evidence;
the interface must not report a removed machine's work as cancelled without evidence.

Any convenience `helm attach` entry point or alias remains a CLI design question.
Setup must distinguish interface use, coordinating/executing work and hosting Vessel,
allow overlapping roles, and make service installation explicit.

## Runtime ownership and continuity

The [multiplexed interface design](voyages.md#one-tui-multiple-live-voyages-planned)
requires one TUI to observe and control multiple live local and Vessel-routed
voyages concurrently. Each voyage's runtime process is separate from the TUI;
local supervision outlives interface exit, while remote execution remains on its
Helms. A shared voyage-connection abstraction must preserve host-specific authority,
explicit action identities and reconnect semantics without assuming a local child
process. This requirement is planned; retained in-process workspace runtimes and
the dedicated foreground worker do not fulfill it.

A local managed-session owner already serializes execution and journal mutation on
its host. That local owner is not the planned coordinating agent across machines.
The distributed design must preserve local execution fencing while defining which
Helm coordinates the voyage and how that role changes without conflicting dispatch.

Closing or reconnecting an interface must be distinct from stopping its coordinator
or participants. Another authorized interface should be able to inspect and steer
the same voyage. Coordinator selection, allowed handoff points, durable handoff
state and recovery are specified for review in the
[coordination contract](voyage-coordination.md#7-explicit-quiescent-coordinator-handoff);
implementation remains planned and automatic failover is not assumed.
A handoff must not replay uncertain tool effects or claim that dead processes survived.

Current dedicated-worker behavior is narrower: losing an operator HTTP client does
not itself cancel a run; losing the worker transport or exiting the foreground
worker cancels owned active work and attempts bounded cleanup. Restart recovery
records interruption and retains unresolved cleanup/tool outcomes. It does not
resume an interrupted process or automatically repeat effects.

Full remote lifecycle still requires create/open/resume, metadata operations,
branching, archival and deliberately authorized deletion, with truthful conflicts
and canonical history. Model selection applies to the selected executing runtime;
it is not a global policy change across all participants.

## Commands, events and observations

The current outbound WebSocket transport and shared protocol provide bounded,
versioned commands, negotiated capabilities, sequencing and replay. The dedicated
worker implements a restricted set. A protocol variant or library helper does not
prove an operator workflow is exposed; consult the remote guide for actual endpoints.

Voyage orchestration requires additional identity/routing and authorization design:
which voyage, coordinator and participant a request concerns; which scope and
ownership revision authorizes it; and how results are attributed after changes.
The [coordination contract](voyage-coordination.md) supplies proposed identity,
assignment, fencing and disclosure semantics. Its domain names are not existing
wire fields; #9 must agree concrete encoding and test both peers before exposure.

Required semantics:

- Persist accepted input and immutable command identity before dispatch; exact retries
  observe original outcomes without creating another execution.
- Keep delivery, admission, run completion and observed cleanup distinct. No guarantee
  of exactly-once external effects across crashes is implied.
- Reject stale authority and conflicting revisions. Handle cancellation and late
  results against the intended run, without attributing them to newer assignments.
- Bound queues, frames, retries, deadlines and replay. Expired replay reports an
  explicit gap/snapshot requirement; snapshots obey current disclosure policy.
- Reauthorize after reconnect. Show interface, coordinator and participant connection
  state separately from queued, running, blocked, cancelled or interrupted work.
- Account for delegated results before claiming an outcome complete. The existing
  per-run completion gate does not prove voyage-level semantic correctness.

## Sharing, retention and approvals

Default exposure remains new explicitly managed sessions only. Existing local
histories require explicit sharing consent and disclosure review. Voyage membership
must not bypass this boundary, including when a participant joins late or a
coordinator changes. Define which context/results are needed and authorized rather
than copying every participant's raw history.

Visibility and retention are separate decisions. Metadata, live events with bounded
replay and durable transcript copies require explicit documented policies. Titles,
paths, usage and tool status can be sensitive. Provider credentials/continuation and
direct human terminal input do not become shared voyage context. Already delivered
content cannot be guaranteed retractable; unshare, archive, local deletion and
Vessel cache deletion remain distinct operations.

The selected trust model is one owner controlling their machines. View, submit,
cancel, approve and enrollment administration remain separate capabilities.
Interface or coordinating roles do not confer approval authority. Current unattended
remote execution refuses human-required actions explicitly. Full scoped remote
approval handling remains required work, with local opt-in, exact immutable action
binding, expiry, attribution, safe cancellation/revocation races and no bypass of
mandatory local restrictions. Notifications convey availability; they do not grant
authority to decide. Separate mobile/browser interfaces are deferred.

## Delivery and verification

| Issue | Remaining outcome |
| --- | --- |
| [#77](https://github.com/o-psi/voyage/issues/77) | Complete Helm-facing voyage workflow and integration |
| [#9](https://github.com/o-psi/voyage/issues/9) | Full interactive and voyage-aware outbound contract |
| [#10](https://github.com/o-psi/voyage/issues/10) | Complete identity/authorization and usable enrollment lifecycle |
| [#78](https://github.com/o-psi/voyage/issues/78) | Full session runtime and separate coordinating-Helm lifecycle |
| [#79](https://github.com/o-psi/voyage/issues/79) | Context/session sharing, retention and per-operation authority |
| [#14](https://github.com/o-psi/voyage/issues/14) | Helm operator interface for local/remote management |
| [#18](https://github.com/o-psi/voyage/issues/18) | Explicit service lifecycle and operational verification |
| [#21](https://github.com/o-psi/voyage/issues/21) | Scoped attended/unattended and remote approval decisions |
| [#72](https://github.com/o-psi/voyage/issues/72) | Secure Vessel notification and authorized handoff |
| [#82](https://github.com/o-psi/voyage/issues/82), [#83](https://github.com/o-psi/voyage/issues/83) | Completion behavior and end-to-end evidence |

The [production contract](attachment-production-contract.md) retains full workflow
acceptance. The [dependency-ordered implementation map](voyage-coordination.md#10-dependency-ordered-implementation-and-acceptance-map)
assigns concrete outcomes and acceptance tests to the related issues. Dependency
order does not make unfinished required behavior optional.
Test actual Helm/Vessel boundaries, scope/authority matrices, duplicate delivery,
local/remote contention, multi-interface reconnect, coordinator handoff, cancellation,
crashes, partial output, storage failure, backpressure, revocation and privacy.
Include TUI input routing, stale/offline presentation, Unicode, resize and recovery.

Linux is the required development quality gate. Report other-platform limitations
and separately observed platform/deployment/provider results. The
[cutover runbook](cutover.md) requires real dogfood evidence; a docs change or a
successful dedicated-worker fixture does not prove the full voyage experience.
