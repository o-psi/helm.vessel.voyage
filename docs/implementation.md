# Implementation sequence

Status: implementation delivered for the [architecture](architecture.md) on the
supported Linux process path; verification and platform limits are recorded below. The current source
map is in [development](development.md); available behavior is in
[current state](current-state.md). Tracking spans
[#77](https://github.com/o-psi/voyage/issues/77),
[#78](https://github.com/o-psi/voyage/issues/78) and
[#14](https://github.com/o-psi/voyage/issues/14).

The delivery invariant is **one session per independent voyage process**, reached
by Helm through a local or remote Vessel. A local runtime socket directly exposed
to Helm, a TUI task per session, or agent loops inside Vessel do not satisfy it.
Uncommitted experiments and historical issue closure are not implementation evidence.

## Implementation status ledger

The six delivery steps now have implemented runtime and operator paths. The table
identifies source boundaries; the verification record below identifies what was
actually observed. Linux build/behavior evidence does not establish native
macOS/Windows support, deployed remote SSH/TLS operation or live-provider quality.
Those limits are explicit rather than alternate embedded execution fallbacks.

| Step | Implemented source and behavior |
| --- | --- |
| 1. Contracts | `crates/voyage-protocol/src/process/`: bounded versioned requests, exact session/incarnation/run identities, caller-bound immutable receipts and durable rejection, metadata event cursors/replay gaps, lifecycle, scoped grants and participant/transfer contracts. |
| 2. Independent owner | `voyage/`: independent lifetime owner, canonical journals/checkpoints/decisions, original-UUID JSON and managed migration, run/session cleanup and host resource accounting. Ordinary/managed/workflow/outbound Helm entrypoints now use Vessel; the embedded TUI and executor wrappers are removed. |
| 3. Supervision | `vessel/src/process/`: serialized independent launch, startup preflight, authenticated health, positive stop/restart/recovery, capacity and Linux service provisioning. Unavailable owners remain fenced. Recovery exposes pending run/resource IDs and distinguishes operator attestation. |
| 4. Multiplexing Helm | `helm/src/process_client/`: local/SSH/grant routes, per-voyage drafts/history navigation/cursors/scroll/pending identities, plain and one-shot clients, durable decisions, runtime controls, real operator tools/workflows/GitHub operations, private PTY attachment and revision-bound exports. |
| 5. Remote lifecycle | Host-private scoped grants and HTTP gateway; current rights/workspace/epoch/revocation checks; branch, archive/restore, confirmed clear/delete, compaction and next-turn model/configuration. Credentials remain on the execution host. |
| 6. Participants and owner moves | Explicit receiver bindings, bounded disclosed context, distinct subordinate sessions, immutable assignment/result/cleanup obligations, cancellation tombstones and idle reconciliation. Pinned signing identities, destination readiness, permanent source fencing and verified checkpoint courier implement explicit owner movement without timeout takeover. |

The Linux installer now has real review/apply, upgrade and rollback paths; the
browser execution console remains deferred. Native process-service
paths outside Linux fail explicitly. The outbound enrollment compatibility route
retains its narrower grant/lease semantics inside a supervised voyage.

## Code disposition and runtime ownership

**Keep means preserve useful behavior and reuse its implementation; it does not
mean keep execution code in Helm.** The existing agent loop belongs in the voyage
runtime. Extraction must move its dependencies and authority with it, not leave
Helm constructing an agent behind a new connection abstraction.

The table records the pre-extraction source paths and required final destinations;
its Helm execution paths are historical baseline references, not the current source
map. Runtime modules live in the `voyage/` workspace package. The
[current development map](development.md) lists their actual locations. Helm links
shared configuration/data types but its executable clients no longer construct the
session executor. Shared code must not become competing session ownership.

| Component and current source | Disposition and target source home | Target process responsibility |
| --- | --- | --- |
| Agent construction in `helm/src/main.rs` and `helm/src/tui_runtime.rs`; `helm/src/agent.rs`, `helm/src/agent/`, `helm/src/context.rs` | **Move and adapt** into `voyage/`, retaining agent semantics, cancellation and context handling. Replace workspace runtime construction with per-session startup. | Voyage owns the agent loop and each run. |
| `helm/src/provider/`, `helm/src/local_provider.rs`, `helm/src/inference/`; execution configuration in `helm/src/config.rs` | **Move** provider execution and configuration resolution into `voyage/`; split interface preferences from execution settings. Preserve native providers and the distinct optional compatibility bridge. | Voyage loads provider credentials on the executing machine and owns requests and continuation state. Helm displays permitted metadata and submits configuration requests. |
| `helm/src/tools/`, `helm/src/policy.rs`, `helm/src/policy_profile/`, `helm/src/runtime_policy.rs`, `helm/src/workspace_instructions.rs` | **Move and adapt** tools, registry construction, policy enforcement and instruction loading into `voyage/`. Keep policy controls in Helm as protocol clients. | Voyage enforces roots, denials, approvals and limits; neither Helm nor Vessel can broaden execution authority. |
| `helm/src/session.rs`, `helm/src/session/`, `helm/src/attachment/journal/`, `helm/src/attachment/runtime.rs`; checkpoint integration in `helm/src/tui/checkpoint.rs` | **Consolidate and move** canonical persistence, admission, fences, steering and checkpoints into `voyage/`. Reuse existing primitives with an explicit storage migration. | Voyage is the exclusive session writer. Helm holds presentation state; Vessel holds supervision metadata. |
| `helm/src/terminal.rs`, `helm/src/subagent/`, `helm/src/supervision.rs`, `helm/src/todo.rs`, `helm/src/completion/`; resource tracking in `helm/src/managed.rs` | **Move and adapt** resource ownership and reconciliation into `voyage/`, retaining cleanup obligations and workspace arbitration. Agent supervision here is distinct from Vessel's new process supervision. | Voyage owns tools, subordinate agents, terminals and cleanup for its session. Vessel manages the voyage process lifecycle. |
| Decision bridges and execution callbacks in `helm/src/tui.rs` and `helm/src/tui_runtime.rs` | **Split and replace** in-process channels with durable runtime decisions and protocol commands/events. Move decision authority into `voyage/`; retain prompts and response controls in Helm. | Voyage validates and resolves decisions; Helm presents them and sends precisely targeted responses through Vessel. |
| `helm/src/workflow/`, `helm/src/github/`, `helm/src/extensions/` | **Split by responsibility**: move execution services and their policy/resource dependencies into `voyage/`; retain operator views and commands in Helm. | Voyage executes supported operations; Helm requests and renders them. Preserve existing workflows through the new boundary. |
| `helm/src/tui/`, rendering in `helm/src/tui.rs`, `helm/src/markdown.rs`, `helm/src/onboarding/`, `helm/src/voyage.rs`, `helm/src/voyage_client.rs` | **Keep and adapt** interface code in `helm/`: navigation, drafts, rendering, connection setup and Vessel clients. Replace embedded runtime access and old Helm-participant assumptions. The current `voyage.rs` contains configuration drafts, not the execution runtime. | Helm owns operator interaction and local view state, never canonical session execution. |
| `helm/src/managed.rs`, `helm/src/remote_worker.rs`, execution entrypoints in `helm/src/main.rs`; relay paths in `vessel/src/attachment_transport.rs` | **Reuse internals, replace entrypoints and topology** with the voyage executable and Vessel routing. Retire the dedicated Helm worker and direct in-process chat execution once equivalent supported workflows are connected. | All session execution runs in voyage processes; Helm clients reach them through Vessel. |
| `vessel/src/` management, enrollment and transport code | **Keep and adapt** applicable authentication, grants and transport machinery; **build** process launch, discovery, incarnation tracking, health, stop and recovery in `vessel/`. | Vessel supervises and exposes independent voyage processes without hosting their agent loops or canonical transcripts. |
| `crates/voyage-protocol/`, `crates/voyage-storage/` | **Keep shared primitives** and evolve contracts at both ends. Share wire types and private-storage mechanisms, not live executors or competing session writers. | Each process uses primitives within its authority; canonical checkpoint writes remain voyage-owned. |
| Workspace manifests, `installer/` and release scripts | **Extend** build, packaging and service setup for the voyage binary and Vessel supervision. Linux installation, upgrades, rollback and service provisioning are implemented; reboot/logout and non-Linux deployment evidence remain separate. | Provision distinct executables and supported service lifetimes; starting Helm must not become the voyage lifetime boundary. |

This is an ownership migration, not a blanket directory rename or a second copy of
the executor. Mixed modules must be split along these boundaries. Retire replaced
paths only after their supported workflows, authority checks and failure handling
are delivered through the new contracts; do not retain an alternate Helm-owned
execution path in the finished architecture.

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
uncertain tool replay. Inspect workspace dependencies and all executable entrypoints
to show that agent/provider/tool construction and canonical writes have moved to
the voyage runtime, with no embedded executor left in Helm or Vessel at final
cutover. File relocation alone is insufficient evidence. Relevant work: #5, #78, #71, #82 and #83.

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
recreate them during documentation work or interpret the seven non-test
[quality gates](quality.md) as behavioral coverage. A separately requested targeted
concurrent-voyage regression now adds an eighth gate. The scenarios above guide
delivery verification. Temporary offline manual probes
exercise actual processes without recreating a repository test suite. Broader
automated coverage, approved live calls and native deployment evidence remain
separate work.

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

## Observed Linux verification

Temporary offline probes exercised real Helm, Vessel and voyage binaries and
private SQLite stores. Native-provider responses came only from local HTTP fixtures;
compatibility/MCP processes were local fixtures. These are manual observations,
not a recreated regression suite or a claim about live provider quality.

| Area | Observed behavior |
| --- | --- |
| Admission and resources | Durable streamed output, exact steering retry, one approved shell effect, question response, exact cancellation with no late shell effect, compatibility and effectful MCP child-session cleanup. |
| Interface | Two independent workspaces execute concurrently while switching; separate drafts, Unicode/paste and narrow resize; background approval targeting rejects another voyage's decision; approval executes once, denial executes nothing; detach/reconnect does not replay; terminal modes restore. |
| Observation and lifecycle | 2,048-event retention, explicit replay gap and snapshot cursor recovery; archive/restore, branch retry, history denial/revocation, deletion isolation; full revision-bound export, clear/compact exact retry. |
| Migration and configuration | Two-session legacy managed source extracts only the selected UUID/history/receipts and retires its source; ordinary JSON source fencing, retained backup deletion and deleted restart; configured branch survives source deletion and removal of original configuration files; managed/outbound owners restart after the entire retired source installation is removed. |
| Process failure | Vessel shutdown/restart preserves an active owner's incarnation and output; killing one voyage leaves another available; restart refuses absent cleanup, recovery exposes pending resources, explicit attestation permits a new incarnation without replay. |
| Operator workflows | Real operator tool registry; retained root PTY across runs, stale terminal run rejection and private input absent from durable files; ordinary/resumed/no-save runs preserve parent history; actual private workflow shell binding remains absent from provider requests/history; temporary model discovery cleanup. |
| Scoped access and participants | HTTP observe-only history refusal and revocation; a real participant child executes once under its accepted binding; parent attribution/cleanup once; exact retry, pre-admission cancellation fence, active cancellation and binding removal prevent late effects; dropped admission response retains one assignment and attributes late terminal cleanup. |
| Outbound compatibility and movement | Actual local enrollment and outbound remote admission after Helm exits; exact activation does not replay, and logical relay outage/withdrawal retains the local owner for observation. Full Helm signed courier preserves UUID/history, exact transfer retry retains destination incarnation and source restart stays fenced; historical command receipts survive with original-principal/payload checks and no provider replay. |

Final source revisions and clean-checkout quality-gate results are recorded on
[#78](https://github.com/o-psi/voyage/issues/78), with access/coordination results on
[#79](https://github.com/o-psi/voyage/issues/79) and
[#77](https://github.com/o-psi/voyage/issues/77). No macOS/Windows native service,
real remote SSH/TLS deployment, reboot deployment, live paid-provider or browser
console validation is claimed. Linux supports the implementation; these additional
deployment/provider certifications remain separate and must not be inferred from
the seven formatting, analysis, build and packaging gates.
