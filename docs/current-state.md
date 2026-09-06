# Current implementation

This page inventories code on `main`, not the target architecture or a readiness
claim. All automated tests and evaluations were removed in
[#140](https://github.com/o-psi/voyage/issues/140). Their former descriptions and
results do not establish current coverage. Documentation replacement is tracked in
[#141](https://github.com/o-psi/voyage/issues/141); see [quality](quality.md) for the
current verification boundary.

## Process boundary

The workspace now includes a standalone `voyage` executable and shared runtime
library. `helm connect` uses the new **Helm → Vessel → voyage process** path.
A Linux Vessel launches one independent process per session and authenticates its
private runtime connection. The connected interface can list, create, observe,
submit, steer, rename, change model and cancel exact runs through Vessel. Closing
that interface does not request cancellation. This is an implemented path, not a
claim that the architecture migration or its acceptance scenarios are complete.

Legacy `helm chat`, `run`, `managed` and `remote-worker` still instantiate runtime
code in the Helm process through the shared voyage library. Their full-screen UI
retains its old workspace runtime lifetime and switching restrictions. They have
not been replaced with the connected multiplexer. No ordinary JSON-to-process
journal migration is automatically performed.

## Connected voyages

`helm connect` discovers an owned private local Vessel directory and can start an
absent supervisor using companion binaries. Linux local access is authorized by
OS account ownership and Unix peer credentials. `--ssh ACCOUNT --remote-directory
ABSOLUTE_PATH` uses that SSH account's local Vessel authority, without forwarding
provider credentials. `--include-local` adds the local catalogue to that remote
connection. The SSH route is separate from enrolled machine/session grants.

The TUI keeps per-voyage drafts, pending command identities, scroll and observation
state, marks unread activity and shows connection errors. Snapshots are polled and explicitly mark bounded/truncated projections; paginated
history and full message/run-output chunks are available through typed requests.
This is not a cursor-based event subscription implementation. Actions capture
session, incarnation and run identity, and uncertain submissions retain their
pending identity for receipt inspection. Reconnect observes without automatically
resubmitting input. The frontend accepts up to 16 paired `--ssh` and
`--remote-directory` destinations, optionally alongside local voyages. Full legacy
panel parity remains unfinished.

The independent runtime reuses managed journal admission, streaming checkpoints,
execution fences, steering and cleanup obligations. Its resource stores are scoped
to the session process directory while existing workspace coordination remains.
It refuses compatibility-bridge execution and effectful MCP configurations whose
cleanup is not supported by the managed execution adapter. Provider selection and
policy are loaded locally on the executing host. Canonical history is written only
by that runtime owner for the new process route.

Runtime approvals and model questions have durable pending requests. Responses bind
an exact run, incarnation, decision and revision; receipts support exact retry and
single-response semantics. Waiting is bounded by the smaller of configured timeout
and 120 seconds. Expiry, cancellation and disconnect do not silently authorize an
action. These runtime decisions do not establish remote enrolled responder grants.

Vessel stores supervision registrations, not canonical transcripts. It serializes
starts, checks capacity, verifies live session/incarnation responses and refuses
stale endpoints. An unavailable runtime is not silently replaced. Explicit restart
requires matching clean-stop evidence; a dead or unreachable process does not
establish successful cleanup. Runtime stop requests cancellation and records clean
stop only after observed cleanup. Supervisor stop leaves independent runtimes alone.
The Linux installer provides explicit service installation, activation, inspection,
stop and uninstall; its wizard remains simulated. See [installer operations](../installer/README.md).

Branch/archive/delete, direct private PTY attachment, full workflow/task/subagent
controls, enrolled per-session grants for this new route, participant-Vessel
execution and owner migration remain incomplete. The roadmap's full exit evidence,
including native service deployment and approved live-provider validation, is not
established by the code inventory here.

The remaining sections inventory shared execution behavior and retained legacy
frontends. A feature in those sections is not automatically exposed by `helm connect`.

## Shared execution and providers

- Native OpenAI Chat, OpenAI Responses and Anthropic transports support streamed
  model output and repeated model/tool turns. Native API providers do not need
  Codex. Provider failures, transient retries and cancellation have runtime paths.
- Native ChatGPT OAuth has login, device login, status, logout and explicit
  one-time Codex credential import. Its subscription backend uses an experimental
  internal product endpoint. API billing and subscription access are distinct.
  The separate optional `codex-compatibility` provider executes Codex app-server
  and retains its own capability restrictions.
- Provider/model discovery, a model picker, next-turn model selection, automatic
  session titles and local-compatible endpoint setup are present. Config inspection,
  dependency diagnostics, shell completions and manpage generation are available.
- Tool registration is local and authoritative. Built-ins cover file read/write,
  directory listing, search, guarded atomic patches, one-shot shell commands,
  persistent PTYs, questions, todos, subagents and completion accounting.
  Optional MCP stdio tools are discovered through configured bounded transports.
- Filesystem roots, command policy, approval modes, timeouts, output limits and
  administrator policy ceilings constrain tools and delegated work. Named profiles
  have explicit selection, preview and freshness checks. These checks are
  application policy; they are not an OS sandbox.

See [configuration](configuration.md) and [security](security.md) for credential,
policy and tool boundaries.

## Conversation and interface

The full-screen TUI has a composer, Markdown conversation rendering, activity and
argument/result detail views, session navigation, model/settings controls, agent
supervision and terminal attachment. Plain chat and one-shot run are separate
frontends. New, resume, rename, branch, explicit context compaction, Markdown export
and confirmed clear operations exist. Branches have new session identities.

Ordinary sessions use atomic JSON persistence, including canonical history, model,
composer draft and recovery metadata. Saved preferences are separate from history.
Unsent drafts are restored during navigation; they are not model messages or
Markdown exports. Session ownership prevents cooperating writers from executing
the same saved session concurrently. Switching sessions does not establish the
target independent runtime boundary.

Native full-screen runs accept bounded steering while working. Accepted input is
saved and queued for the next safe model boundary; it does not modify a request
already in flight. Receipts distinguish queued, applied and unapplied delivery;
restart can leave delivery unknown. Work is not automatically replayed. Plain,
unattended and compatibility-provider modes have narrower steering capabilities.
Approvals and questions use frontend response channels, not durable multi-interface
decision ownership. Questions are distinct from authorization; plain/unattended
questions return unavailable. Unattended approval follows configured policy without
waiting for an interactive prompt.

Context and output limits are opt-in (`context_window` and `max_tokens` default to
zero); provider capacity still applies. An enabled local context check can omit old
whole turns from the request copy while retaining canonical saved history. Manual
compaction is separate. Automatic semantic recovery from provider context rejection
is not implemented. Explicit session/project inference allowances, durable attempt
accounting, usage history and reasoned operator overrides are available; an
allowance is not a guarantee about provider billing or available context.

## Tasks, subagents and terminals

Todos are durable **workspace-scoped** records with dependencies, status, blockers,
assignment, notes, progress, evidence and archival. Assignment targets local
subagents; it does not dispatch to remote machines. Subagents support bounded
concurrency and queuing, messaging, follow-up, wait/cancel, durable outcomes and
archival. Optional Git worktrees support isolated work and guarded integration and
cleanup. Active subagents do not survive process restart merely because their
records persist.

Completion ledgers bind obligations to trusted session/run identities, track
adoption and evidence, and distinguish completed work from blocked/deferred or
unresolved work. Final-response gating and durable completion dispositions are implemented.
Accounting does not prove the semantic truth of submitted evidence. Cooperating
workspace writers use shared coordination and ownership locks; a second subagent
runtime for the same workspace can be refused. These workspace stores are not yet
per-Voyage process resource ownership.

PTYs support start, incremental read/write, resize, interrupt, selection, naming and
termination within the Helm process. Count and unread-output limits apply. Saved
terminal metadata does not restore a process: loaded entries are stale after
restart. Direct human attached-terminal input stays outside model-visible history;
model-directed terminal I/O has the normal tool-output boundary. Questions and
ordinary chat are model-visible and unsuitable for secrets. Terminal control text
is sanitized for display; diagnostics go to a private log rather than the TUI.

## Managed and remote surfaces

`helm managed` uses an explicitly selected private SQLite journal for create/list,
revision-bound submit, exact-run cancellation and local recovery. Managed admission,
immutable command receipts, canonical checkpoints and cleanup blockers exist.
Ordinary JSON chat/run sessions are not automatically admitted to this owner.
Transfer, catalogue, identity and sharing libraries are foundations; their presence
does not make ordinary TUI navigation a managed or remote execution client.

`helm remote-worker` exposes one dedicated session through an explicit foreground
outbound connection to an enrolled Vessel. Providers, tools, roots and credentials
stay on the executing Helm. Vessel's opt-in HTTP relay supports bounded listing,
inspection, submission, observation and exact-run cancellation. It is not a general
remote create/open/history/model/approval interface. Existing private sessions are
not shared by enrollment. The worker retains native-provider/resource restrictions
and conservatively cancels on transport loss or authority withdrawal. Durable grant
withdrawal and local crash recovery exist; interrupted work and unconfirmed cleanup
must not be reported as successful or automatically replayed. Local cleanup
attestation is distinct from observed cleanup.

Enrollment supports status, rotation, revocation and local detach. Ordinary
`helm attachment connect` provides presence; discovery in the voyage setup UI is
read-only and saves configuration drafts. Sharing consent, actor identity,
coordination metadata, nomination leases and immutable control receipts do not
start coordinators, grant tool execution or implement distributed work. These
legacy enrollment surfaces do not supply the new local/SSH multiplexer with
per-session grants. Coordinator handoff and a unified enrolled remote operator
interface remain incomplete. The Vessel web page is a status page, not an execution
console.

## Other product surfaces

Declarative extensions have explicit installation/activation and resource/skill
selection. Repository onboarding produces reviewable guidance. Saved workflows
provide typed public inputs, preview, digest-bound trust and explicit transient
private shell bindings. GitHub tooling separates inspection from attended review
of publication. These features remain subject to local authority and provider/tool
availability. The installer wizard is a setup preview; its separate CLI implements
explicit Linux service installation and lifecycle operations.

See [operations](operations.md) for existing commands and recovery, and
[implementation](implementation.md) for the work required to establish the target
process architecture. Source inventory alone does not establish live-provider
compatibility, native-platform security, service behavior or release readiness.
