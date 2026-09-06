# Implementation sequence

Status: planned work for the [architecture](architecture.md). The current source
map is in [development](development.md); available behavior is in
[current state](current-state.md). Tracking spans
[#77](https://github.com/o-psi/voyage/issues/77),
[#78](https://github.com/o-psi/voyage/issues/78) and
[#14](https://github.com/o-psi/voyage/issues/14).

The delivery invariant is **one session per independent voyage process**, reached
by Helm through a local or remote Vessel. A local runtime socket directly exposed
to Helm, a TUI task per session, or agent loops inside Vessel do not satisfy it.
Uncommitted experiments and historical issue closure are not implementation evidence.

## 1. Define component and connection contracts

Define versioned Helm–Vessel and Vessel–voyage contracts before moving frontends.
They cover capability discovery, catalogue, create/open, snapshots/events, command
receipts, submit/steer, decisions, cancellation, lifecycle controls and runtime
stop. Keep local and remote Vessel clients behind the same interface.

Specify session identity, process incarnation, run identity, command deduplication,
expected revisions, current authority, event cursors and deadlines separately.
Bound frames, queues, subscriptions and retention. Specify replay-gap recovery,
unknown command outcomes and unsupported capabilities. A decoded frame is not an
authenticated or authorized command.

Resolve concrete endpoint authentication, service configuration and credential
ownership here. The existing outbound Helm worker protocol can supply reusable
code, but is not the new topology. Check both ends of any wire change.

Exit evidence: a reviewed contract and cross-component examples for normal requests,
stale/duplicate requests, denial, disconnect, replay gap and ambiguous delivery.
Relevant work: #9, #10, #78 and #79.

## 2. Extract the voyage executable and authoritative owner

Add a real `voyage` binary and reusable runtime modules. Move agent construction,
run admission, checkpoints, steering, decisions, resource ownership and cleanup
out of the TUI's lifetime. Start exactly one session owner in each voyage process.
The voyage remains available across individual turns.

Reuse managed journal admission, execution fences and cleanup obligations rather
than adding a second executor. Integrate ordinary chat, managed storage and future
plain/one-shot clients with one authoritative owner. Preserve exact history and
session UUIDs; do not keep writable JSON and journal copies of a session. Existing
storage-transfer primitives require explicit frontend integration and recovery.

Audit workspace-keyed terminal, subagent, todo and completion ownership. Distinct
voyage processes need distinct inventories while retaining shared workspace writer
arbitration and machine-wide budget/resource enforcement. Provider credentials,
continuation state and local tool authority remain with the executing runtime.

Exit evidence: runtime execution without a TUI; exclusive ownership under competing
clients; durable streamed output; interrupted admission/checkpoint/cleanup with no
uncertain tool replay. Relevant work: #5, #78, #71, #82 and #83.

## 3. Make Vessel supervise voyage processes

Add explicit process start, discover, inspect, stop and recovery operations to
Vessel. Give each session a separate voyage PID/incarnation and private runtime
connection. Serialize competing starts, authenticate the owner and reject stale or
forged endpoint registrations. Stored PIDs never establish survival.

Vessel holds supervision metadata, not the agent loop or canonical transcript.
Protect local runtime configuration and credential access without depending on
Helm's environment or terminal descriptors. Enforce host capacity before launch.
Define graceful drain/stop policy, startup failure and unconfirmed cleanup.

Vessel restart must reconcile surviving owners and abandoned obligations without
duplicate starts or implicit tool replay. Closing Helm must leave the Vessel and
voyages alive. Package secure local-service operation on supported platforms;
report unsupported native paths instead of providing insecure fallbacks.

Exit evidence: two independent voyage processes, one runtime crash isolated from
the other, Helm exit/reconnect, duplicate starts, stale registrations, exhausted
capacity, and Vessel shutdown/restart under active work. Relevant work: #18 and #78.

## 4. Turn Helm into the multiplexing TUI

Replace in-process agent ownership with Vessel clients. Connect to the local
Vessel automatically for normal local use, without remote enrollment. Combine
permitted local and remote catalogues in one voyage list, with origin, runtime
state, connection freshness, unread activity and pending decisions.

Maintain separate drafts, scroll positions, event cursors and modal state per
voyage. Switching a view cannot cancel work or redirect a pending action. Route
input, steering, decisions, model/policy changes and exact-run cancellation using
the identity captured when the action was prepared. Preserve drafts on refusal or
unknown delivery. PTY attachment is a separate private channel.

Retain useful existing workflow, tool, task, subagent, terminal and model controls
through authorized runtime operations. Do not replace them with nonfunctional
panels. Keep the UI responsive under slow connections and flooded background output.

Exit evidence: concurrent voyages remain active while switching; late replies do
not cross targets; background decisions are actionable; Unicode, paste and narrow
viewports work; reconnect restores observations without resubmitting input.
Relevant work: #14, #21, #32 and #72.

## 5. Complete remote access and session lifecycle

Use the same Vessel management surface locally and remotely with explicit client
identity and session grants. A remote Vessel supervises its own voyage processes;
Helm does not create a proxy agent on the interface machine. Discovery and connection
must not implicitly share existing sessions or expand local roots.

Complete authorized create/open/history, rename, branch, archive, confirmed delete
and next-turn model changes. Durable decisions require current responder authority,
exact targeting, deadlines and single-response semantics across interfaces.
Separate interface loss from runtime authority loss; preserve fail-closed dispatch
and truthful enforcement/cleanup states through revocation and partition.

Exit evidence: one Helm manages a local voyage and multiple remote voyages through
Vessels, including simultaneous progress, reconnect, exact cancellation, denied
history, revoked grants, expired decisions and capability/version differences.
Relevant work: #9, #10, #14, #21, #72, #78 and #79.

## 6. Add participant-Vessel execution deliberately

Vessels joining a voyage must not create another canonical session owner. Define
locally accepted execution bindings, durable assignment admission/deduplication,
context disclosure, result attribution and subordinate cleanup. Unknown assignment
acceptance blocks reassignment; membership removal has an explicit drain/cancel
disposition. Participant workers remain subordinate to the owning voyage process.

Moving that owner to another Vessel is separate from participation. Require positive
source fencing, durable relinquishment and verified destination readiness before a
new incarnation becomes authoritative. Do not implement timeout-driven takeover.
The exact inter-Vessel transport and portable context format must be specified
before these operations can be enabled.

Exit evidence: authorized participation, denied grants, partitioned enforcement,
late results, ambiguous admission, membership removal and owner-move failures without
a duplicate session owner. Relevant work: #77, #78 and #79.

## Verification and delivery

The previous automated tests and evaluation suite were removed by request. Do not
recreate them during documentation work or interpret the remaining seven non-test
[quality gates](quality.md) as behavioral coverage. The scenarios above are delivery
requirements for implementation; replacement automated coverage and approved live
or native deployment checks need to be addressed when validation work resumes.

For every delivered slice, record the source revision, observed behavior, failure
handling, supported platforms and unresolved requirements. Run applicable existing
checks and the required clean-checkout quality gates. Publish only the completed
scope, keep broader epics open, and preserve other work and release artifacts.

The core acceptance workflow is: launch two local voyages and a remote voyage from
one Helm; verify one separate process per session; switch and steer while all work;
resolve a background decision; close/reopen Helm; recover output without replay;
cancel one run and observe cleanup while the others continue. Then exercise a
runtime crash, a Vessel outage and revoked access. Multi-Vessel participation needs
its additional evidence before being advertised.
