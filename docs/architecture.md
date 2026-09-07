# Architecture

Status: canonical component model. The Linux implementation uses these process
boundaries; see [current state](current-state.md) and [delivery evidence](implementation.md)
for supported paths, verification and deployment limits.

## Three programs

**Helm is the TUI.** It owns presentation, per-voyage drafts, navigation and client
connections. Helm connects to Vessels, whether local or remote. It does not own an
agent loop, execute tools, acknowledge canonical checkpoints or directly supervise
voyage processes.

**Vessel is the supervisor and access service.** It authenticates clients,
discovers and starts voyage processes, exposes their authorized state, routes
commands and observations, and reports process health and resource capacity.
A local Vessel and a remote Vessel have the same role. Local versus remote is
relative to the Helm connection, not a different product or session type.

**Voyage is the execution runtime. One session has one independent `voyage`
process.** That process owns the conversation, agent loop, provider transport,
tool registry, decisions, run admission, canonical persistence and cleanup. It
is separate from Helm, Vessel and every other session's runtime process. Vessel
must not implement sessions as agent tasks inside its own process.

```text
                         one session per execution process
Helm ── local Vessel ──── voyage A
                  └───── voyage B
     └─ remote Vessel ─── voyage C
                  └───── voyage D
```

The lines are authenticated control and observation connections. They do not
represent shared transcript ownership or permission to transfer provider secrets.
Vessel owns supervision metadata; the voyage owns authoritative session state.
A shared storage engine is possible only if it preserves these ownership fences.

## Terminology and identities

| Term | Meaning |
| --- | --- |
| Voyage / session | The same ongoing unit of work. A new session or branch creates a new voyage. |
| Conversation | The voyage's canonical interaction history. |
| Run | One execution within a voyage; ending a run does not end the voyage. |
| Voyage process | The live runtime for one session; its process incarnation changes after restart. |
| Vessel | A service supervising voyage processes and participating through explicit local grants. |
| Machine | The operating-system host. It is not a session, runtime or permission grant. |
| Participant | A Vessel accepted into a voyage's scope; membership alone cannot dispatch work. |
| Subagent | Work delegated within a voyage, not another independently created user-facing session. |
| Configuration draft | Saved proposed settings without execution ownership. |

A voyage keeps its session UUID across resume and runtime restart. Every runtime
start has a fresh incarnation identity. A process ID alone cannot establish
identity or liveness. Run IDs, command IDs, session revisions, event cursors,
connection generations and membership revisions have different meanings and must
not be interchangeable.

A voyage needs no repository, project map or mandatory named purpose. A workspace
is a local resource binding, not the identity of the voyage. Several voyages may
refer to the same workspace, subject to explicit writer arbitration.

## Local use

The target local path is `Helm → local Vessel → voyage`. Local operation requires
a local Vessel, but no remote account, remote enrollment or machine-selection
wizard. Normal startup should discover the user's local Vessel and start it when
necessary, visibly reporting its independent background lifetime and failures.
Service installation and automatic startup at login are explicit operations.

Helm presents one ordinary new-voyage action and one list containing permitted
local and remote voyages. Choosing another Vessel changes where a new runtime is
hosted. Selecting a voyage connects to its existing owner rather than launching a
second session executor. Concurrent create/open requests must converge on the same
selected identity or return an explicit conflict.

Before first send, a new-voyage proposal is a Helm-local configuration/composer
draft with no execution ownership. Opening Helm alone does not allocate a voyage
process. First send durably records its proposed identities and hands creation and
turn admission to Vessel and the independent voyage runtime. An uncertain handoff
retains those identities and text for explicit recovery. Existing-session resume
and branches with inherited conversation are separate intentional operations.

## Vessels participating in a voyage

Vessels may join a voyage under user-controlled scope and locally accepted grants.
The Vessel supervising its voyage process provides the route to its canonical
owner. Additional membership must not spawn a competing voyage runtime or copy
its canonical conversation into an independently writable session.

Multiplexing many voyages across Vessels and delegating work across several
Vessels inside one voyage are different capabilities. The former needs one runtime
process per session; the latter additionally needs durable assignment, disclosure
and cancellation contracts. Any participant-side worker is subordinate to its
assignment and cannot become a second canonical session owner.

The initial implementation must establish the single-owner process boundary and
mixed local/remote management. Participant execution and moving a voyage to another
Vessel require additional explicit contracts and evidence. Exact inter-Vessel
routing, discovery and transport negotiation are implementation decisions; the
current outbound Helm attachment protocol does not implement this target topology.
Do not silently invent a central mandatory Vessel or automatic owner failover.

## Lifecycle and failure isolation

Switching the selected voyage only changes the view and input target. Voyages
continue concurrently under per-voyage and machine-wide limits. Helm disconnect or
crash detaches the interface; it does not cancel accepted work. A completed run
leaves the voyage available for another turn.

Vessel shutdown is an explicit service-lifecycle operation. It must enumerate owned
voyage processes and apply a visible drain or stop policy. A Vessel crash creates
uncertainty that must be reconciled with live runtime owners; it is not evidence
that processes stopped, survived or completed cleanup. Runtime or machine failure
can interrupt execution. Restart restores durable history and records a new
incarnation; it never claims old tools or PTYs survived without observation.

Each voyage serializes mutating runs and owns its subordinate work. Crashing one
voyage must not terminate another voyage or Helm. Resource limits and shared-file
arbitration are still required: a process boundary is not an OS sandbox.

See the [runtime contract](runtime-contract.md) for required state transitions,
[security](security.md) for authority and [implementation](implementation.md) for
delivery order. No new `voyage` or Vessel-supervision CLI syntax is established by
this design document.
