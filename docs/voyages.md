# Voyages: sessions in Helm

**Agreed product direction; multi-Helm voyage orchestration is planned.**
Tracking: [#77](https://github.com/o-psi/voyage/issues/77).
This document defines the product model for the related designs and delivery issues.
Current setup and execution commands are documented separately below.

**Helm is the program. Every session is a voyage.** Starting a new local chat is
starting a new voyage; it needs no Vessel, enrollment, name, purpose form or machine
selection. A voyage is an open-ended session in which the user and agents work
within a user-controlled scope of one or more Helms. It may remain entirely local.

Helm is the interface for local work and for working through Vessel with other
Helms. Vessel supplies the authenticated control plane for remote work. Adding
permitted Helms configures the scope of the voyage; it does not turn an ordinary
chat into a separate kind of session. The conversation provides continuity across
turns and, with planned cross-Helm coordination, across machines.

## Terms and everyday workflow

| Term | Meaning |
| --- | --- |
| Helm | The program providing the interface and agent runtime; an installed or running instance may fulfill several roles. |
| Voyage / session | The same ongoing unit of work. Session is the technical name retained in current CLI, API and storage contracts. |
| Conversation | The interaction history presented within a voyage, not a separate local-only product object. |
| Turn | An interaction within the voyage. |
| Run | An execution within the voyage, with its own completion and cleanup state; finishing it does not end the session. |
| Composer draft | Unsent input belonging to a voyage. |
| Configuration draft | Optional saved setup choices; it is not a session or a running voyage. |

- Start a local voyage with `helm chat`, or use **Ctrl+N** / `/new [TITLE]` in
  full-screen Helm. A fallback title is supplied automatically.
- Find saved voyages with **Ctrl+S** or `helm sessions`; current interface labels
  also call these sessions or recent conversations. Resume continues the same
  voyage; creating or branching a session creates a new voyage.
- `helm run` starts a voyage when it creates a session; `helm run --resume REF`
  continues an existing one. Managed and dedicated remote sessions are voyages too.
- Selecting other Helms is optional scope configuration. The current **Ctrl+V** /
  `/voyages` picker only saves configuration drafts and cannot attach those choices
  to an executing session. See the [current UI guide](helm-voyage-ui.md).

The intended interface has one ordinary new-voyage action and a recent-voyage list,
with optional Helm selection. A separate local-chat versus voyage workflow is not
part of the product model. Current labels and draft controls do not fully express
that model yet; their UI/runtime integration remains required work.

A voyage can involve research, files, systems administration, software, or several
kinds of work together. A repository, project, component map or fixed workflow is
optional. An application and a license server on another host are one example of
work spread across machines, not a required structure for every voyage.

## Roles

| Role | Responsibility |
| --- | --- |
| Interface Helm | Presents the voyage and lets an authorized user inspect, steer and manage it. |
| Coordinating Helm | Runs the coordinating agent: interprets the request, plans, delegates within scope and collects results. |
| Participant Helm | Executes assigned work under its own local authority and reports results. |
| Vessel | Authenticates access and provides machine presence, routing and permitted coordination metadata. |

These are roles, not three different kinds of installation. In ordinary local
chat, the local Helm supplies the interface, coordinates the voyage and executes
its work. A Helm may serve more than one role. The interface open on a workstation need not be the coordinating
Helm; the coordinator may run on an always-on remote machine. The coordinator may
also execute work itself when permitted. Vessel does not become the agent runtime
or acquire provider credentials.

For example, a user can open Helm on a laptop, interact with a voyage coordinated
by Helm on another machine, and involve two other permitted Helms as needed. The
laptop need not participate in execution. Closing that interface is a separate
action from cancelling the work. Another authorized interface can reconnect to the
same voyage; it must show actual coordination and execution state.

## One TUI, multiple live voyages (planned)

One Helm TUI must multiplex multiple simultaneously running voyages. Its unified
voyage list includes local voyages and authorized voyages reached through an
attached Vessel. Switching the selected voyage changes the view and input target,
not execution: background voyages continue without requiring the user to finish
or cancel their work. Separate terminal tabs and local subagents are not a
substitute for this workflow. Multiplexing independent voyages is distinct from
coordinating several participant Helms within a single voyage.

Each executing voyage must have its own runtime process, separate from the TUI
process and other voyages' execution runtimes. A local voyage runs on the local
Helm host without requiring Vessel. A Vessel-routed voyage executes on its
coordinating and participant Helms, not inside Vessel or a proxy agent in the
interface process. Multiple roles can share a Helm installation without sharing
the TUI's process lifetime. Participant execution remains subject to its own host's
authority; this does not require every participant to live in one process.

The interface must use a common voyage-connection abstraction over local IPC and
Vessel-routed connections, rather than treating every voyage as a local child
process or embedding interactive terminals. It must support authorized observation,
input, approval responses and exact-run cancellation with explicit capability and
availability reporting. Wire encoding and supervisor/service mechanics remain
implementation work under #9, #14, #18 and #78; no new command is implied here.

Required boundaries:

- Execution ownership, canonical persistence and checkpoint acknowledgement belong
  to the executing runtime, not the selected TUI view. One active mutating run per
  voyage remains the rule; concurrency across voyages is bounded and independent.
- Local supervision must outlive TUI detach/exit. Closing or crashing the interface
  does not cancel voyages. Explicit stop/cancel is separate and reports observed
  cleanup. A runtime or host crash can interrupt work; reconnection must not claim
  process survival or replay uncertain tool effects.
- Show voyage identity, local/Vessel origin, coordinator and participant locations,
  run state, connection freshness, unread activity and pending decisions. A voyage
  may span hosts; one location label must not imply a permanent single-host scope.
  Disconnection means execution state may be unknown, not necessarily stopped.
- Bind drafts, input, steering, approvals, questions and cancellation to explicit
  voyage/run/request identities. A view switch must never redirect a pending
  action. Viewing a voyage grants no additional execution or approval authority.
- Reauthorize on reconnect and recover persisted, sequenced events with bounded
  replay and explicit gaps. Retry commands only under their original identity and
  receipt semantics; do not blindly resubmit actions after losing a response.
- Bound queues and resource use per voyage so a slow view, disconnected Vessel or
  failed runtime does not block unrelated voyages. Process separation does not
  isolate shared files: host policy, workspace conflicts and resource limits still
  need enforcement. Listing voyages must not implicitly share private sessions.

This is the target architecture, not current TUI behavior. Today navigation is
blocked during active work, workspace runtimes live in the frontend process and
actual frontend exit shuts them down. Current dedicated-worker transport-loss and
exit semantics below also remain narrower than this target.

## Machine scope and routing

A new local voyage uses the local Helm; choosing local chat establishes that scope
without an extra picker. Extending the scope to other Helms must be explicit and
user-controlled. Enrollment makes a machine known to Vessel; it does not
automatically admit the machine to
every voyage, share its private sessions, or authorize work on it.

The user can direct a particular piece of work to a permitted Helm, or let the
coordinator choose within the current scope. Selecting a coordinator or starting
on a machine must not permanently bind every subsequent task to that machine.
Work can span several participants without a compulsory component-to-host map.

Scope changes must be visible and authorized. Adding a machine cannot silently
expand local permissions or disclose existing private history. Removing a machine
must report the disposition of its queued and active work. The detailed rules for
removal, reassignment and already-disclosed context are specified for review in the
[coordination contract](voyage-coordination.md#5-scope-edits-while-work-exists);
implementation remains planned.
Agents and delegates cannot add machines or widen authority on their own.

## Session continuity and lifecycle

The voyage represents the ongoing conversation. Individual turns, delegated tasks
and their runs have bounded lifecycle and completion states. A completed run does
not close the entire voyage or imply that every requested outcome was verified.

Interface presence, coordinator availability, participant connectivity and run
state must be shown separately. An interface disconnect must not be interpreted as
a cancellation request. Stopping an executing Helm or losing execution authority
is a different event and must preserve truthful results and cleanup status.

Coordinator selection and handoff must be explicit to the user and prevent two
coordinators from issuing conflicting work. The selection UX, allowed handoff
points and recovery protocol are proposed in the
[quiescent handoff contract](voyage-coordination.md#7-explicit-quiescent-coordinator-handoff).
They still require implementation and failure testing. No automatic failover,
transparent process survival or safe replay of uncertain effects is promised.
A new coordinator must not infer that an interrupted process is still running or
repeat a tool effect whose outcome is unknown.

Users must be able to follow assignments and results across permitted machines,
understand blockers, and distinguish provisional output from verified outcomes.
Local voyages already persist through the existing session stores. Cross-Helm
context distribution and reconciliation with participant execution records
follow the proposed [context and continuity contract](voyage-coordination.md#6-context-canonical-history-and-retention).
“Voyage” does not introduce a second local identity layered over
a session. This terminology does not rename existing IDs, change the Journal
schema or completion ledger, or establish a distributed identifier mapping.

## Authority and privacy

Every executing Helm enforces its local workspace roots, command policy, approvals,
resource limits and cancellation. Coordination is not permission to override them.
Provider credentials and opaque provider continuation state remain on the Helm
using them. A path, model choice or policy on one host is not automatically valid
on another.

The selected trust model is one owner controlling their own machines. Interface
access, coordination, execution, enrollment administration and approval decisions
still need distinct scoped authorization. Being a viewer or coordinator grants no
automatic approval authority. Required human decisions must be routed explicitly
or reported unavailable; unattended work cannot wait indefinitely or bypass them.

Share only authorized context and results, with explicit disclosure and retention
rules. Existing private sessions remain private until deliberately shared. Direct
human terminal input stays outside model-visible records. Revocation can stop
future authorized access; it cannot retract bytes already disclosed.

Helms connect outbound to Vessel. No inbound Helm task port is required.
Application policy is not an OS sandbox.

## Available now and still planned

| Workflow | Current status |
| --- | --- |
| Local voyages: Helm chat, saved sessions, tools and local subagents | Available; see [Helm](../helm/README.md). |
| Private managed local sessions and run-owned completion accounting | Available; see [managed sessions](local-managed-sessions.md) and [completion](completion-gate.md). |
| Enrollment and authenticated outbound presence | Available through [enrollment](attachment-cli.md) and [presence](attachment-presence.md). |
| Explicit metadata registration and outbound control leases | Available through [coordination control](coordination-control.md); nominations confer no execution authority. |
| One new dedicated foreground remote session | Available through `helm remote-worker` and opt-in authenticated Vessel HTTP operations; see [remote sessions](remote-sessions.md). |
| Helm voyage setup and machine/coordinator selection | Available as [saved configuration drafts](helm-voyage-ui.md); no runtime execution or sharing. |
| Start, resume and work in local voyages through Helm | Available through current chat/session controls. |
| One TUI multiplexing process-isolated local and Vessel-routed voyages | Planned; current navigation blocks during active work and frontend runtimes are in-process. |
| Unified voyage creation/scope UI and remote voyage management in Helm | Planned integration; current configuration drafts are separate from sessions. |
| Remote voyage coordinator, cross-Helm delegation and coordinator handoff | Planned. Local subagent supervision and managed-session ownership are foundations, not these features. |
| Full remote lifecycle, broader sharing, services and delegated approvals | Still require delivery and verification under the attachment issues. |

The current worker's operator HTTP client may disconnect without requesting
cancellation. Loss of the worker's own outbound transport cancels its active work;
foreground worker exit also cleans up owned work. Those current semantics must not
be described as the planned persistent multi-Helm voyage experience.

`helm attach` and a unified voyage command set are not implemented. Browser console
work is deferred; Helm is the intended operator interface. Existing Vessel HTTP
operations and its authenticated static status page remain useful current surfaces.

## Delivery and acceptance

The [session management design](vessel-session-management.md),
[coordination contract and implementation map](voyage-coordination.md), and
[production contract](attachment-production-contract.md) map the remaining work.
The full agreed workflow is the completion standard; a working transport or a
single remote task alone does not establish full multi-Helm readiness.

Acceptance must first demonstrate that an ordinary local chat creates a voyage
without Vessel or a configuration wizard, and that resume preserves its identity
while new/branch creates another. Optional scope configuration must not create a
second conversation category or silently share its history.

Multiplexing acceptance must demonstrate two local voyages running concurrently
without Vessel, then local and Vessel-routed voyages in the same TUI. Verify
separate runtime process identities, switching while both make progress, retained
per-voyage drafts, background decision notifications, exact-target responses and
cancellation, TUI exit/crash and authorized reconnect without stopping live work.
Kill one runtime and disconnect one Vessel connection: unrelated voyages must
remain usable, and unavailable state or interrupted work must be reported honestly.
Cover duplicate/stale commands, event gaps, slow-consumer backpressure, bounded
concurrency, shared-workspace conflicts and private-session/approval isolation.

Multi-Helm acceptance must demonstrate an interface on one Helm, coordination on
another and work on additional permitted Helms, as well as overlapping roles and local-only
use. Cover explicit targeting, coordinator routing, scope edits, reconnecting from
another interface, cancellation, handoff conflicts and honest interrupted recovery.
Include work without a repository and work across multiple unrelated directories.

Tests must also cover unauthorized machines/interfaces, private-context disclosure,
revocation, stale scope, duplicate commands, reordered results, partial failures,
resource cleanup and bounded replay. Define handoff and shared-context contracts
before encoding their acceptance tests. Preserve existing per-run completion tests;
add cross-machine accounting evidence rather than treating local tests as proof.
Linux is the required development gate. Report separate platform, deployment and
live-provider evidence where applicable; documentation alignment is not runtime
verification. The [cutover runbook](cutover.md) retains the dogfood evidence gate.
