# Voyages across Helms

**Agreed product direction; multi-Helm voyage orchestration is planned.**
Tracking: [#77](https://github.com/o-psi/voyage/issues/77).
This document defines the product model for the related designs and delivery issues.
Current setup and execution commands are documented separately below.

A **voyage** is an open-ended session in which the user and agents work across a
user-scoped set of Helms. Helm is the interface for local work and for working
through Vessel with other Helms. Vessel supplies the authenticated control plane.
The conversation and its purpose provide continuity as work moves between machines.

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

These are roles, not three different kinds of installation. A Helm may serve more
than one role. The interface open on a workstation need not be the coordinating
Helm; the coordinator may run on an always-on remote machine. The coordinator may
also execute work itself when permitted. Vessel does not become the agent runtime
or acquire provider credentials.

For example, a user can open Helm on a laptop, interact with a voyage coordinated
by Helm on another machine, and involve two other permitted Helms as needed. The
laptop need not participate in execution. Closing that interface is a separate
action from cancelling the work. Another authorized interface can reconnect to the
same voyage; it must show actual coordination and execution state.

## Machine scope and routing

Each voyage has an explicit user-controlled scope of permitted Helms. Enrollment
makes a machine known to Vessel; it does not automatically admit the machine to
every voyage, share its private sessions, or authorize work on it.

The user can direct a particular piece of work to a permitted Helm, or let the
coordinator choose within the current scope. Selecting a coordinator or starting
on a machine must not permanently bind every subsequent task to that machine.
Work can span several participants without a compulsory component-to-host map.

Scope changes must be visible and authorized. Adding a machine cannot silently
expand local permissions or disclose existing private history. Removing a machine
must report the disposition of its queued and active work. The detailed rules for
removal, reassignment and already-disclosed context still need design decisions.
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
points and recovery protocol remain open design questions. No automatic failover,
transparent process survival or safe replay of uncertain effects is promised.
A new coordinator must not infer that an interrupted process is still running or
repeat a tool effect whose outcome is unknown.

Users must be able to follow assignments and results across permitted machines,
understand blockers, and distinguish provisional output from verified outcomes.
The mechanics of shared context, voyage persistence and reconciliation with each
Helm's canonical local session/run history are still to be designed. This product
model does not redefine the existing local Journal schema or completion ledger.

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
| Local Helm chat, saved sessions, tools and local subagents | Available; see [Helm](../helm/README.md). |
| Private managed local sessions and run-owned completion accounting | Available; see [managed sessions](local-managed-sessions.md) and [completion](completion-gate.md). |
| Enrollment and authenticated outbound presence | Available through [enrollment](attachment-cli.md) and [presence](attachment-presence.md). |
| One new dedicated foreground remote session | Available through `helm remote-worker` and opt-in authenticated Vessel HTTP operations; see [remote sessions](remote-sessions.md). |
| Helm interface for managing local/remote voyages, scope and participants | Planned. Current Helm chat does not provide that operator workflow. |
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

The [session management design](vessel-session-management.md) and
[production contract](attachment-production-contract.md) map the remaining work.
The full agreed workflow is the completion standard; a working transport or a
single remote task alone does not establish voyage readiness.

Acceptance must demonstrate an interface on one Helm, coordination on another and
work on additional permitted Helms, as well as overlapping roles and local-only
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
