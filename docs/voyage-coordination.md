# Cross-Helm coordination contract

**Design revision 1; proposed implementation contract for review. No distributed
runtime or wire feature is enabled by this document.** The accepted
[voyage model](voyages.md) governs terminology and product scope. Tracking:
[#77](https://github.com/o-psi/voyage/issues/77). The dependency and test map below
specifies remaining work; it does not mark those issues complete.

The selected deployment model is [one owner controlling their Helms](https://github.com/o-psi/voyage/issues/77#issuecomment-5548263061).
This scopes authentication, not the distinct interface/coordinator/participant or
approver roles; it does not prescribe a future multi-operator deployment. A voyage needs no
repository, project map, or named purpose. Ordinary local chat keeps its current
session UUID, local privacy, and local execution without contacting Vessel.
Configuring other Helms extends that same session. The interface may remain on a
laptop while another Helm coordinates and other participants execute. These roles
can also all belong to one Helm.

## 1. Identity and ownership

Names in this document are domain concepts, **not existing JSON fields, commands,
capabilities or schema versions**. A negotiated implementation must encode and test
them explicitly; existing strict v2 peers must refuse unsupported coordination.

| Identity | Meaning and lifetime |
| --- | --- |
| Session UUID | Existing voyage identity. Resume, scope edits and handoff preserve it; new/branch creates another. |
| Session address | Origin installation ID plus session UUID, bound during explicit first registration. The origin component disambiguates independently created/copied stores; it is routing provenance, not a second voyage object. It stays stable after coordinator handoff. |
| Interface instance | Authenticated client connection and principal. Reconnection changes connection identity without creating another session or run. Multiple interfaces may observe the same voyage. |
| Coordinator epoch | Monotonic generation of the selected coordinating Helm for this session address. Distinct from credential epoch, socket generation and session revision. |
| Scope revision | Monotonic revision of user-selected execution participants and their accepted bindings. Separate from canonical conversation revision and per-host policy revisions. |
| Coordinating run | One admitted operator turn and its owned delegated work. At most one mutating coordinating run per session; steering joins its durable queue. |
| Assignment ID | Durable identity of one delegated obligation within that run. It does not name a new user-facing voyage. Retransmission never creates a new assignment. |
| Participant run | Host-local execution record bound to exactly one assignment attempt, with its own checkpoints, effect intents and cleanup evidence. |
| Command ID / event cursor | Immutable request identity / observation position. Neither is execution authority. Cursors are scoped to their projection and authorization generation. |

A configuration draft has none of these execution powers. Saved picker choices
must be reviewed against current enrollment, scope and policy when applied to an
existing session. Copying an installation directory is not a supported way to
create a second concurrently active installation; identity collision fails closed.

The coordinator owns the canonical conversation and coordination ledger. A
participant owns its local execution history, provider state, resources and effect
records. Participant execution records are subordinate records, not independently
editable copies of the voyage. Vessel owns authenticated routing and the durable
control record identifying the coordinator and current scope. It does not run the
agent, become the canonical transcript store, or replace a Helm execution fence.

All local and remote writers of a managed voyage use its same authoritative local
owner. Explicit preparation of a current JSON session for coordination must retain
its UUID, fence the source, preserve a private original and reject ambiguous
publication. It must never maintain two writable JSON/Journal histories. Ordinary
local use does not require this preparation or registration. Storage format work
and its confirmation UI belong to #78/#14; a configuration draft cannot invoke it
implicitly.

## 2. Authority and local acceptance

Each operation needs the intersection of authenticated principal capability,
installation delegation, current voyage scope, disclosure consent, executing-host
policy and any required exact-action approval. Feature negotiation is only codec
compatibility. A title, advertised machine capability, stored binding, consent
snapshot, or Vessel compare-and-swap result is never an execution grant.

The coordinator belongs to the voyage's selected scope and explicitly accepts the
coordinating role. It may run its model there; ordinary tool execution on that host
still needs a locally accepted execution binding. Interface-only Helms need not
belong to execution scope. A participant accepts a locally configured workspace
identity, provider/model availability, policy ceiling and resource limits; remote
paths, credentials or a coordinator's more permissive profile cannot replace them.
A workspace may be any permitted directory and need not contain a repository.

Enrollment proves a machine's relationship with Vessel. Selecting it in the
interface records a proposal, not acceptance by its runtime. Additions become
eligible only after the selected Helm authenticates the owner/coordinator,
validates current local delegation and durably accepts the exact binding. Rejected,
stale, offline or unavailable bindings remain visible and ineligible. A local
owner may configure bounded unattended acceptance within explicit grants; the
coordinator cannot create those grants. Rotation/re-enrollment requires fresh
current binding validation, never silent reuse of a prior epoch's consent.

The executing Helm checks authority inside admission and again before provider,
tool, subagent and disclosure boundaries. Control changes and dispatch must have a
local ordering point: either dispatch was committed under the still-valid grant,
or the change prevents it. Reading ConsentStore, releasing its lock and later
admitting from that snapshot does not establish this ordering. #79/#78 must build
a current local authority owner with durable revisions, cross-process exclusion,
and a transaction-compatible guard. Never hold database/storage locks across
network I/O or provider execution. An external effect already dispatched cannot
be recalled by changing a policy record.

Cancellation and truthful authorized observation remain available when a selected
execution profile becomes stale. They still require current identity and access;
a failed dispatch check must not obstruct cleanup or grant private observation.

## 3. Durable control changes and coordinator fencing

There is one active coordinator epoch per registered session address. Vessel
persists control changes with an expected control revision and immutable operation
ID/digest. Exact retries observe the original transition; changed retries conflict.
The current coordinator also holds a local session execution fence and a finite,
current remote coordination lease. Participants verify the authoritative epoch,
current scope and their own local grants before accepting an assignment.

Vessel's control record contains identifiers, bindings, revisions, transition state
and bounded digests/receipts, not conversation text, provider state or secret input.
It is an additional routing fence. A valid control record cannot cause a Helm to
execute without its own accepted local authority and journal admission.

A control mutation uses explicit `proposed`, `fencing`, `committed`, or `blocked`
observations. These are proposed domain states. The response names which transition
is durable, which Helms acknowledged enforcement, and which remain unreachable.
Vessel and Helm stores do not share an atomic transaction; an outbox/inbox and
idempotent acknowledgments must bridge them. Failed or uncertain writes preserve
the operation identity and a non-executable pending state. Recovery reads receipts
before advancing; it does not infer a completed transition from elapsed time.

For removal/revocation, Vessel stops new routing as soon as its fence commits. A
Helm already holding a grant may have an in-flight effect: local enforcement is
observed separately or bounded by lease expiry. Do not report instantaneous global
revocation. A disconnected participant accepts no new remotely delegated work;
transport/authority loss initiates conservative cancellation and bounded cleanup.
It cannot renew authority from cached metadata. The planned path preserves this
fail-closed policy rather than promising autonomous partitioned execution.

Reconnecting uses fresh authentication and reconciles retained obligations before
new work. Replayed frames cannot renew an obsolete coordinator epoch. A service
restart obtains a new live owner; pending/unknown execution remains blocked. Restoring
old backups must not resurrect old control or consent generations: reconciliation
with current authority is required, and disagreement fails closed.

## 4. Assignment admission, results and retries

An assignment's immutable request binds session address, coordinating run,
assignment/attempt ID, coordinator epoch, scope revision, authenticated requesting
principal, target Helm and enrollment epoch, local execution binding revision,
context digest, task text, constraints, result-disclosure policy and admission
expiry. No provider credential, remote-selected filesystem root or opaque provider
continuation is included. The participant verifies the coordinator on its current
outbound Vessel connection; unauthenticated claimed IDs cannot name a trusted sender.

The coordinator transaction records the assignment obligation and send intent
before it publishes dispatch. The participant atomically records deduplication,
accepted task/context, local run identity, revision and initial cleanup obligation
before acknowledgment or effectful runtime construction. The coordinator records
that receipt before showing acceptance. Lost acknowledgments are recovered with
the same immutable request, including its original expiry and digest.

Participant deduplication outlives event replay and binds the complete request,
including coordinator epoch and target. An exact accepted retry is observation
even after its admission deadline, subject to current observation authorization.
It never receives another execution token. A changed target, context or payload
under the same identity conflicts. Receipt capacity exhaustion rejects new work;
it cannot discard history and permit re-execution. A receipt query after handoff
uses separately authenticated observation, not a forged retry with a new epoch.

For an **explicit target**, refusal/offline/busy leaves a visible blocker and never
falls back to another host. For **delegated placement**, the coordinator may choose
among currently accepted in-scope bindings. Before any participant admission, a
provably rejected attempt may be replaced with a new attributed placement attempt.
Unknown delivery/admission is not rejection: query/recover that exact attempt and
block reassignment until its disposition is established. After admission, changing
target requires cancellation/cleanup accounting and an explicit new attempt; it
is not an automatic transport retry. No task is permanently tied to its first host.

Results bind assignment, attempt, participant run and execution host, with terminal
outcome, cleanup state, bounded summary, authorized artifact references, reported
usage and evidence limitations. They are untrusted task data, not instructions or
authority grants. A coordinator records a result once, preserves its attribution,
and records incorporation, failure impact, or no-longer-needed reason. Late or
duplicate results cannot satisfy another assignment or overwrite a newer result.
Terminal failure/cancellation and observed cleanup do not imply task success.

The parent completion gate accounts for all owned assignments, descendants, results
and todos using a fresh revisioned snapshot. Removing a participant, archiving an
assignment or switching interface cannot erase those obligations. Unresolved remote
state yields an incomplete/interrupted outcome with explicit IDs. Mechanical
accounting cannot prove semantic correctness; useful synthesis still needs #83/#22
evidence. Finishing a run leaves the voyage available for another turn.

## 5. Scope edits while work exists

An authorized user previews a scope edit against the current revision. Two
interfaces racing edits get one commit and one conflict; the loser retains its
proposed edit for review, without silent merge or automatic resubmission.

| Edit | Required outcome |
| --- | --- |
| Add a participant | Show proposed local binding and disclosure. It becomes eligible after local acceptance and control commit; it receives no earlier history automatically. Existing assignments keep their identities. |
| Remove an idle participant | Fence new assignments, revoke future disclosure, obtain local acknowledgment or show enforcement pending. Retain prior result/receipt provenance. |
| Remove with queued or active work | Require a choice between **drain existing work** and **cancel owned work**. Neither choice authorizes reassignment or erases unknown outcomes. |
| Drain | Stop new assignments immediately at the routing fence. Permit only explicitly named already-admitted assignments to continue under current local authority and finite leases, with the minimal result/cleanup channel disclosed in the preview. Unadmitted queued work stays blocked for explicit replanning. |
| Cancel | Record durable cancellation intents for the exact assignment runs. Stop new effects at each participant's observed fence; report requested, terminal outcome and cleanup separately. No new assignment inherits a cancelled attempt's ID. |
| Remove the coordinator | Refuse the direct edit; perform the explicit handoff below, then remove its execution membership if requested. |
| Narrow disclosure or revoke a local grant | Stop future unauthorized publication/admission. If required context/result exchange is no longer permitted, cancel affected work and retain a truthful blocker; do not silently keep sharing under drain. |

The no-argument/default removal path does not cancel active work; it asks for a
disposition. Unattended APIs require that disposition explicitly. Drain can remain
pending indefinitely if the user keeps necessary work running; it is not an
unbounded hidden stdin wait. Cancellation and transition observation have bounded
requests, and the user may later choose cancel. A disconnected host is displayed
as enforcement/cleanup unknown, not removed-and-stopped. Re-adding it uses a new
binding revision and cannot reactivate earlier assignments or consent.

Unaffected accepted assignments may continue after a scope revision changes only
when current authoritative scope still admits their exact binding. New assignments
must name the current scope revision. A drain exception permits only its enumerated
old assignment IDs, never new descendants on another Helm or expanded authority.
Local descendant work remains within the already-admitted assignment's limits.

## 6. Context, canonical history and retention

Scope selects possible execution hosts. Disclosure separately selects the data a
recipient may receive and retain. The default is no pre-existing private history.
A participant receives a bounded typed context packet: task text, selected public
conversation excerpts or an explicitly labelled summary, relevant authorized
results/artifact references, source revisions and provenance. It never receives a
raw serialized Session, another host's provider state, credentials, private secret
bindings or direct human PTY input/output. Artifact references confer no filesystem
access and require a separately authorized bounded fetch; URLs are not credentials.

The user previews recipients, content categories, selected history boundary,
retention and onward disclosure before applying scope/context changes. A locally
configured per-session grant may permit repeated packets within those bounds;
each send still rechecks it. Delegates cannot forward a packet to another Helm
merely because both belong to the voyage. Late joiners get no automatic replay of
earlier context. Branching preserves private ancestry restrictions and resets
approval delegation; a branch is not a way to export its parent's private history.

Result return is also disclosure. Assignment acceptance must establish permission
to return the requested summary/evidence to the coordinator and authorized
interfaces, and state whether that content may become canonical conversation.
If permission is absent or revoked, show a bounded unavailable-result status rather
than copying local tool records. Do not report success merely because private local
work finished. Known-secret redaction is an additional output defense, not consent
or a promise to discover all sensitive information.

Visibility and retention remain independent. Proposed implementations support
none, metadata, live events with bounded replay, and explicit retained transcript
access. Metadata includes sensitive names, paths and usage; redact or omit fields
outside the exact grant. Vessel's default content relay is ephemeral; a durable
coordination registry does not authorize transcript caching. Retained participant
context/results need explicit byte/count/TTL limits and deletion behavior under
#79. Expiry stops access and scheduled retention; backup copies and already-delivered
bytes cannot be guaranteed retractable. Immutable deduplication receipts may retain
bounded identifiers/digests after content expiry, with that retention disclosed.
No content retention limit may erase an unresolved effect/cleanup obligation.

For coordinator handoff, this revision deliberately requires authorization for the
**complete portable canonical conversation and coordination ledger needed to
continue the same voyage**, not just a model-generated summary. Preview exactly
which historical fields move. Encode an explicit, bounded provider-neutral snapshot
with original message ordering/text and attribution, validated tool-call/result
structure, receipts, revisions and obligation dispositions. Never serialize Session
wholesale. Exclude provider continuation, runtime instructions, credential/secret bindings
and raw/private terminal streams that are outside canonical history. Already
canonical, authorized model-visible tool text must retain its exact text and order;
it cannot be silently stripped because it originated at a terminal. If such a
canonical field is prohibited from disclosure, block handoff as below. Host-specific
runtime resources remain on their host.

If some required canonical record cannot be disclosed or represented safely, handoff
is blocked with the excluded category identified without leaking its contents.
Do not silently truncate/rewrite the old conversation or pretend a summary is its
canonical replacement. Scope delegation can still operate with selected packets
while the original coordinator remains. A future partial-history handoff would
need a separately reviewed continuity contract; it is not implicit in this design.

## 7. Explicit quiescent coordinator handoff

Handoff is user-requested, to an in-scope Helm that has accepted coordination under
its own policy. No automatic failover, active process migration or forced takeover
is defined. The destination's provider/model must be explicitly available; a
provider change needs review and cannot transfer the old provider's credentials or
continuation state. Unsupported canonical tool history fails before activation.

1. Persist an immutable handoff request bound to source/destination, current epoch,
   control/conversation/scope revisions and transfer consent. Enter fencing; stop
   new turns, steering and assignments. Interfaces retain rejected drafts. Observe
   accepted work until it drains, or perform separately confirmed cancellation.
2. Acquire/retain the source execution owner. Establish positive quiescence: no
   active coordinating run, pending steering, undelivered admission ambiguity,
   active assignment, unaccounted result or unresolved effect/cleanup obligation.
   Named local recovery/attestation remains explicit and distinguishes observed
   cleanup from operator testimony. Timeout/offline state is never quiescence.
3. Freeze the canonical revision and create the authorized portable snapshot with
   its digest. Durably mark the source relinquished/pending handoff so restart or
   loss of Vessel cannot make it writable. Preserve its private local data.
4. The destination acquires its local session fence and stages the exact snapshot
   under the same session UUID/address, with verified source provenance and no
   execution grant. Reject a conflicting existing session or changed snapshot.
   Confirm durable readiness only after storage, consent, policy and capability
   validation. A staging acknowledgment does not select a coordinator.
5. Vessel commits one expected-revision transition to the next coordinator epoch,
   bound to both durable source relinquishment and destination readiness. It never
   chooses another target on timeout. The destination rereads that committed
   transition and current local authority before activation. A lost response is
   resolved by exact operation lookup, not a second handoff or provider dispatch.
6. Source remains read-only for that epoch and points authorized clients to the
   new coordinator. Participants learn the new epoch before accepting new work;
   old frames remain fenced. Old receipts/results remain inspectable only under
   current consent, with original host/epoch attribution intact.

There is no instant distributed commit. Crash after any step leaves a durable
pending or committed transition that recovery can inspect. Before source
relinquishment, an authenticated abort may restore admission on the unchanged
source epoch. After relinquishment, resume only the exact handoff or perform a new
explicit recovery transition after all possible owners are positively fenced.
Deleting a pending marker, expiring a lease, or restoring an old backup cannot
implicitly roll ownership back. If the source and its canonical data are lost,
this design cannot reconstruct them from Vessel's metadata-only registry.

Foreground coordinator exit is execution-host shutdown: cancel/reap its owned
work, retain unconfirmed obligations and stop renewal. Closing an independent
interface only detaches that observer. When one process holds both roles, a close
that would also terminate the coordinator must explain that consequence; continued
background execution requires an explicitly installed/started runtime host (#18),
not a hidden daemon or a claimed surviving process.

## 8. Operator input, cancellation and notifications

All interfaces read the same authorized projection and submit revision-bound
commands to the coordinator. A submitted ordinary turn uses compare-and-swap;
competing submissions do not run twice. Active steering has durable ordered
queued/applied/not-applied receipts and a bounded queue, with drafts retained on
refusal. Reconnect observes receipts; it never resends composer text automatically.
An unauthorized or malformed observation cannot cancel unrelated work.

Cancellation names the exact coordinating run or assignment run. A whole-run
cancel durably closes new assignment admission before fanning out exact child
cancellation intents. A racing late child admission remains owned and must be
queried/cancelled; it cannot escape the parent obligation ledger. Requested means
intent persisted, terminal means execution outcome known, cleanup observed means
owned-resource observation completed. Operator attestation remains separately
labelled. A later turn cannot proceed through unresolved consequential effects.

Questions and approvals have separate destinations and records. Model questions
are model-visible input, never a secret-entry surface. Remote approval is disabled
unless the executing Helm opts into the exact capability and authenticated
principal. Bind a decision to action digest, local workspace/policy, session,
coordinating and participant run, tool call, executing Helm, epoch and finite
expiry. Recheck all current authority at dispatch; changed arguments need a new
request. Local/remote decisions compete for one atomic terminal transition.
Unavailable/expired/revoked requests deny without indefinite stdin waits.

Vessel notifications carry bounded authorized references and minimal content for
attention, failure, completion or budget events. They confer no decision authority.
Reconnect may deliver the same notification; deduplication must not execute a
response twice. Resolved/expired references remain non-actionable. Delivery failure
is visible without promoting permissions. The authorized Helm interface opens the
current request; browser/mobile destinations remain deferred by user direction.

## 9. Compatibility, bounds and verification obligations

Coordination requires explicit feature/version negotiation on both Helm and Vessel.
The current v2 command domain has no coordinator epoch, scope revision or assignment
binding; do not reuse a connection ID for them. Unsupported peers continue only
within their explicitly selected existing functionality, with no silent protocol
downgrade or accidental coordination admission. Final wire shapes belong to #9
with fixtures for both readers and writers and malformed nested values.

Retain finite existing frame/command/queue limits; new context packets, participant
sets, control transitions, receipts, replay pages, notification queues and retained
snapshots need explicit byte/count bounds. Capacity rejects new admissions without
losing accepted evidence. Do not add a general model-turn deadline or discard active
obligations to make storage fit. Trusted time is sampled at commit boundaries;
clock failure refuses new time-sensitive authority while permitting authorized
receipt observation. Lease expiry uses monotonic runtime time and cannot be extended
by reconnecting with stale persisted timestamps.

Transport retries are bounded and reuse immutable IDs. Public replay has a separate
contiguous cursor per permitted projection. When authorization changes, invalidate
that projection generation; do not advance a cursor across silently filtered private
records. Snapshot-required recovery returns only currently authorized state.
Canonical and durable-result retention are not equivalent to a live replay window.

## 10. Dependency-ordered implementation and acceptance map

Each row is a release-quality work package, not permission to stop after a type,
helper or happy-path fixture. Tests ship with the implementation they exercise.
Packages may develop against reviewed contracts in parallel; exposure waits for
all stated dependencies. Existing issue scope remains broader where noted.

| Order / issue | Concrete deliverable and dependency | Acceptance mapped to verification |
| --- | --- | --- |
| A — #9 / #10 | Agree strict negotiated coordination envelopes and authenticate interface, coordinator and participant roles against current installation/principal/credential epochs. Add the control-record CAS, immutable control receipts, leases and restart fencing. No effectful route until B/C. | Wire goldens on both peers; unsupported/unknown/duplicate fields; forged sender/role, enrolled-but-out-of-scope, rotation/revoke, stale generation, expiry during lock wait, concurrent control mutation and killed-writer recovery. Assert zero provider/tool dispatch. |
| B — #79 | Join durable consent to current cross-process admission/disclosure authority; implement recipient-specific context/result projections, preview/confirmation, late join/branch restrictions and bounded retention/deletion. Uses A identities; C uses its transaction-compatible guard. | Scope × principal × action × disclosure matrix at actual dispatch/publication; revoke between check/commit and during stream; private ancestry, changed destination, stale snapshot, content expiry versus retained receipt, backup limitations, symlink/storage failures and no secret/provider/PTY leakage. |
| C — #78 | One authoritative session backend for local and remote interfaces; explicit existing-session preparation preserving UUID; durable per-assignment outbox/inbox/dedup, participant run mapping and owned cleanup. Integrate normal agent/subagent/policy builders. Requires A/B before remote admission. | Two local processes plus two interfaces racing revision; loss before/after both admission commits; exact retry across restart; changed payload/target; capacity and disk-full; native-provider tool effect counted once; unknown outcome blocks reassignment; local-only chat still works without Vessel. |
| D — #78 / #9 | Actual coordinator delegation and attributed result collection across separately running Helms, scope add/drain/cancel, and the quiescent handoff state machine above. Requires A–C. | Interface A, coordinator B, participants C/D plus overlapping roles; explicit target no fallback; delegated routing without repository; delayed/duplicate/reordered results; scope edits while running; partition/revoke; crash after every handoff step; stale source cannot restart; no false stopped/complete outcome. |
| E — #14 | Integrate optional scope selection into ordinary new/resume voyage controls, current coordinator/participant health, steering, assignment/result inspection, full session lifecycle and confirmations. Use actual A–D APIs; configuration drafts remain inert until applied. | Real PTY plus real Helm/Vessel boundaries: local new/resume UUID, remote coordinator independent of interface, multi-interface reconnect/CAS conflict, source draft preservation, Unicode/paste/narrow/resize, exclusive modal/terminal input, sanitized host text, offline states and exact destructive-action confirmation. |
| F — #21 | Durable exact-action approvals routed to authorized local/remote Helm interfaces, disabled by default, with local ceiling checks and single-use decisions. Requires A–C and E decision surface. | No approver/expiry/deny/cancel; local-vs-remote race; action/host/epoch mismatch; stale policy; stolen or replayed decision; restart after decision and before effect; no blind execution replay, no secret in question/approval/session/log. |
| G — #72 | Upstream Vessel notification outbox/inbox, authorized Helm attention and question/approval reference handling, destination preferences, expiry, dedup and visible delivery failures. Requires A/B; F for approval references. | Disconnected interfaces, duplicates, overflow, expired/resolved requests, revocation/scope change, missing recipients, no forged decision from opening a reference. Preference/quiet-hour tests where applicable; no browser/mobile implementation implied. |
| H — #18 | Explicit supported runtime/service hosting separate from interface lifetime; install/start/stop/reboot/upgrade/rollback/uninstall, private identity/backups, signed artifacts/SBOM and operator recovery. Can develop alongside A–G; full lifecycle drills require them. | Actual service foreground/background lifetime, no implicit elevation, host restart with truthful interruption/cleanup, upgrade while owned, preserved identity/session data, uninstall without silent data loss, rollback cannot restore revoked authority, deployed TLS/proxy drills. Archive build alone is not service evidence. |
| I — #82 / #83 / #22 / #19 | Cross-Helm completion accounting and useful result synthesis, failure/security/load drills and recorded representative dogfood. Local completion and current dedicated-worker tests remain valid only for their actual scope. Requires C/D and complete A–H workflow for epic exit. | False-final/ignored result, failed/cancelled child impact, late result, blocked todo and unrelated work isolation; all native offline adapters; bounded approved live general-purpose workloads and measured outcomes; record failed/unrun evidence rather than checklist-only completion. |

Issue #10 also retains credential storage/encryption and setup obligations beyond
role binding; #79 retains full consent administration and retention, #14 full
session lifecycle, and #18 all supported operational artifacts. Dependency order
does not waive these or promote earlier “later” exclusions into completion rules.
The next useful implementation is A's current authenticated control/identity
contract joined with B/C authority and admission, not another uncalled data type.

Required integration scenario: start a local voyage without Vessel, explicitly
configure accepted remote scope and consent, use a distinct remote coordinator,
delegate useful non-repository work to two Helms, close/reopen an interface,
review attributable results, exercise an approval, remove a participant with work,
and perform a positively quiescent handoff. Repeat with crashes, revoked authority,
unknown effects and unavailable consent; every refusal must preserve recoverable
state. Full lifecycle/service operation and semantic evidence remain epic gates.

Linux quality/system/release/packaging checks are required. Native platform,
deployment and live-provider evidence are separate observations; no Linux fixture
proves native Windows/macOS service or storage behavior. No live calls are required
to review this design. Runtime tests are not evidence for a prose-only change.

## 11. Source boundaries and remaining decisions

The existing implementations below are useful foundations, not distributed
coordination. Source links target the repository; packaged guides remain readable
without source files.

| Current source | Verified boundary |
| --- | --- |
| [remote_worker.rs](https://github.com/o-psi/voyage/blob/main/helm/src/remote_worker.rs) | One dedicated session with fixed workspace/model and enrollment binding; list/inspect/submit/cancel plus replay. Active work is cancelled on worker transport loss. |
| [journal/remote.rs](https://github.com/o-psi/voyage/blob/main/helm/src/attachment/journal/remote.rs) | Singleton remote binding (`slot=1`), transactional public projection/receipts and cleanup evidence; no participant-assignment ledger. |
| [attachment/runtime.rs](https://github.com/o-psi/voyage/blob/main/helm/src/attachment/runtime.rs) | ManagedSessionOwner retains host-local execution ownership; admission checks current authority inside the owned transaction. It is not a distributed coordinating agent. |
| [sharing/consent.rs](https://github.com/o-psi/voyage/blob/main/helm/src/attachment/sharing/consent.rs) | Durable preview/CAS consent declarations and audit receipts; no production authorization adapter. Its storage lock must not be held across network/provider work. |
| [attachment.rs](https://github.com/o-psi/voyage/blob/main/crates/voyage-protocol/src/attachment.rs) | Strict v2 command vocabulary and capability names. Full-lifecycle variants alone do not expose handlers or coordinator authority. |
| [remote_http.rs](https://github.com/o-psi/voyage/blob/main/vessel/src/remote_http.rs) | Authenticated bounded dedicated-worker relay, current connection checks and immutable mutation fingerprints; no durable distributed control registry. |
| [voyage.rs](https://github.com/o-psi/voyage/blob/main/helm/src/voyage.rs), [voyage_setup.rs](https://github.com/o-psi/voyage/blob/main/helm/src/tui/voyage_setup.rs) | StoredDraft and discovery/picker state, not session creation, scope admission or execution. |

Revision 1 resolves the default placement/removal/handoff/continuity rules for
review. Remaining implementation decisions must be recorded before their code:
exact wire feature/version and field schemas; control-registry persistence and
local authority transaction integration; numeric bounds and retention defaults;
secure interface credential setup; concrete service/credential-store adapters.
They cannot weaken the invariants above. Active handoff, automatic failover,
partitioned autonomous work and partial-history handoff are not promises of this
revision; adding any requires explicit design and new failure/authority evidence.
