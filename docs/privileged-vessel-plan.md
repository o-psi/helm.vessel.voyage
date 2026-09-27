# Privileged Vessel and per-voyage execution identities

Status: implementation plan, not current runtime behavior. Approved direction:
**a privileged Vessel supervisor launches independent Voyage processes under an
explicitly configured execution identity** (option 2).

Tracking: [#344](https://github.com/o-psi/helm.vessel.voyage/issues/344).
Related: [filesystem consent #327](https://github.com/o-psi/helm.vessel.voyage/issues/327),
[remote updates #343](https://github.com/o-psi/helm.vessel.voyage/issues/343),
[browser ownership #333](https://github.com/o-psi/helm.vessel.voyage/issues/333),
[accounts #213](https://github.com/o-psi/helm.vessel.voyage/issues/213), and
[recovery #238](https://github.com/o-psi/helm.vessel.voyage/issues/238).

Baseline inspected: `470eb82a030357537200883620659a2249302ffa`. No host privilege,
service, credential, or filesystem ownership changes are part of this planning
delivery. Release milestone and implementation schedule remain unassigned.

## 1. Outcome and boundaries

The installer can provision a privileged system Vessel. The administrator selects
an ordinary execution user and explicitly enables any administrator execution.
Each voyage identifies who executes its tools. A human owner can approve an
administrator run through either Helm client, including on a remote machine.

A normal filesystem approval must accurately describe access available to that
execution identity. When operating-system restrictions prevent the requested
operation, Helm offers a distinct administrator-execution review when supported.
A successful administrator launch still respects the host's actual capabilities,
mounts, mandatory access controls, and policy. Never promise universal access.

Preserve the architecture in [architecture.md](architecture.md): Helm presents,
Vessel authenticates and supervises, and one independent Voyage process owns each
session's conversation, execution, decisions, journal, and cleanup. No embedded
agent loop, client-side executor, or generic remote privileged shell endpoint.

The design makes no assumption that the host is dedicated, disposable, a
container, or a VM. Detect capabilities on the executing host. Root inside a
namespace is not evidence of authority outside it. The first implementation is
Linux on the currently supported architecture with systemd because that is the
existing installer/runtime backend. macOS, Windows, and alternate service managers
need explicit backends and native verification; report unavailable support.

User installations remain supported. Existing installations are never converted
by an ordinary upgrade. System installation, administrator enablement, and legacy
adoption are distinct reviewed operations.

## 2. Current implementation and required changes

| Surface | Current behavior | Required work |
| --- | --- | --- |
| Installer | User-owned layout and `systemctl --user` | Explicit installation scope, protected system layout, system lifecycle adapter |
| Process launch | Child inherits supervisor identity | Pinned execution identity, complete privilege setup/drop, startup attestation |
| Registration | No execution-principal binding | Supervisor-owned identity, policy revision and authorization binding |
| IPC | Supervisor checks peer UID equals its own UID | Verify the registered runtime UID and exact process incarnation |
| Private files | Current-user ownership checks | Separate supervisor control and per-identity runtime ownership rules |
| Provider accounts | Host registry discovered from process environment | Explicit account namespace and authorized identity association |
| Filesystem grants | Runtime policy overlay | Actual-identity access preflight and truthful failure classification |
| Recovery and updater | Same-user paths and user services | Identity-preserving recovery, system update/readiness/rollback |
| Helm | Provider profile, location and access-mode controls | Separate execution-user/admin review and observed execution status |

Primary code entry points:

- Installer: `installer/src/cli.rs`, `install/layout.rs`, `install/transaction.rs`,
  `service/unit.rs`, `service/command.rs`, `service/systemd.rs`, `remote/linux.rs`.
- Supervisor: `vessel/src/process/launch.rs`, `start.rs`, `accounts.rs`, `registry.rs`,
  `database.rs`, `routing.rs`, `recovery.rs`, `recover_command.rs`, `suspension.rs`,
  `access/`, `transfer/`, `participant/`, `updates.rs`.
- Runtime: `voyage/src/server.rs`, `server/transport.rs`, `server/bootstrap/`,
  `runtime_policy.rs`, `tools/roots.rs`, `accounts.rs`, `accounts/`, `subagent/`.
- Contracts: `crates/voyage-protocol/src/process/`, account/decision contracts and
  `crates/voyage-storage/src/` migrations and ownership boundaries.
- Clients: `helm/src/process_client/`, `web/resources/js/voyage-settings.js`,
  `web/resources/js/vessel-update.js`, Web decision rendering, gateway schemas,
  Flux Blade components, and the alternate React client's supported contracts.

Audit every launch path: fresh start, configured/account/settings start, resume,
automatic suspension recovery, branch, transfer/import, managed execution,
workflow, participant, subagent and updater. A fix only in new-voyage setup is
insufficient. Update all applicable public allowlists and both protocol ends.

## 3. Authority model

Keep these independent:

1. Human connection rights: who may observe, create, approve, or administer.
2. Execution identity: which OS user/groups actually run a Voyage.
3. Runtime policy: tools, allowed roots, network, sandbox and resource limits.
4. Provider account: which device-local account supplies inference and billing.

An administrator OS identity does not implicitly grant provider accounts, other
Vessel connections, or permission to accept its own elevation. An ordinary
connection with lifecycle/Decide permission cannot authorize administrator startup.
Existing full-access connections must not silently acquire a new machine-admin
right during migration; explicit bootstrap binds the initial administrative owner.

### Proposed records

Names below describe new contracts, not existing callable commands:

- **ExecutionIdentity:** opaque host-local ID; revision; label; OS backend; pinned
  UID/GID/group policy or native equivalent; execution home; account namespace;
  enabled state; administrator classification; applicable policy ceiling.
- **ExecutionBinding:** Vessel ID, voyage ID, identity ID/revision, resolved OS
  identity, launch/incarnation ID, policy revision and authorization reference.
- **ExecutionReview:** operation ID; requester/owner identity; target voyage and
  expected revision; requested identity; workspace; policy/isolation description;
  account binding; transition type; expiry; integrity digest of the reviewed facts.
- **ExecutionReceipt:** exact command and review IDs, admission state, observed
  launch identity, completion/failure/uncertainty and cleanup obligations.

Clients select opaque configured identities; they cannot submit arbitrary UID,
group, executable, environment, home, unit content, or credential-store paths.
Re-resolve the configured OS account before launch. Account deletion, UID reuse,
group changes, policy edits and identity revision changes invalidate stale review.
Numeric UID alone is not a durable account identity. External account-directory
changes that cannot be verified must block the affected launch.

### Authorization lifetime

Persist the desired execution identity as session metadata. Treat authorization
to start an administrator process separately. Default proposal: explicit owner
approval for each administrator run; a process replacement requires fresh review
before privileged execution resumes. Old approval receipts establish what happened,
not permission to launch another privileged incarnation. Ordinary automatic
resume retains its configured identity and still rechecks current host policy.

No silent fallback from an unavailable identity to root, the supervisor user, or
another user. Retain the voyage and explain the required action. Durable standing
administrator policies are separate future scope unless explicitly requested.

### Trust boundary

An unrestricted host-root voyage has machine-administrator authority. It can alter
host files, processes, credentials and potentially the supervisor itself. Local
receipts cannot be called tamper-proof against that administrator. Approval is
meaningful authorization, not a claim of confinement from a root adversary.

When offering a restricted administrator mode, require a verified OS isolation
boundary and report its exact limits. The current default, sandbox off, cannot
support a promise that arbitrary root commands are confined to approved paths.
Never advertise isolation merely because the host is virtualized. A shared OS
identity also does not provide isolation between mutually hostile voyages. An
account with existing sudo, privileged groups, service-control rights or equivalent
host grants may already be administrative. Inventory and report that fact; do not
claim the account is unprivileged from a nonzero UID or remove its rights silently.

## 4. System installer and host discovery

Introduce an explicit installation-scope abstraction consumed by every layout,
service, status, update, rollback and uninstall operation. Proposed CLI shape:
`install --scope system --execution-user USER`, with administrator enablement as a
separate explicit choice. These flags do not exist yet. Never infer the ordinary
user from a remote login, directory owner, browser user, or `SUDO_USER` alone.

Before mutation, produce a reviewable plan that probes:

- OS/architecture and service-manager support; real effective privileges.
- Namespace/user mappings, available capabilities, mount restrictions and required
  isolation mechanisms. Do not require or alter container/hypervisor settings.
- Configured user/group resolution, execution home, account storage and workspace
  accessibility under the intended identity.
- Existing installation, Vessel identity, state schema, services, endpoints,
  occupied ports, live voyages, gateway dependencies and available rollback.
- Root ownership and non-writable ancestors for the executable and control paths.

Recommended Linux system layout, subject to packaging conventions:

| Path | Ownership / purpose |
| --- | --- |
| `/opt/voyage/releases/<id>` | Root-owned immutable release including browser worker assets |
| `/opt/voyage/current` | Transactionally selected release, root-controlled |
| `/etc/voyage/` | Root-owned execution identities and policy ceilings |
| `/var/lib/voyage/` | Root-owned supervisor catalogue, receipts and installer state |
| `/run/voyage/` | Protected discovery and authenticated local control endpoints |
| Explicit per-identity runtime roots | Private journals, artifacts and runtime IPC; accessible only as designed |

Do not execute a user-writable binary as root, including an existing per-user
`current` link. Validate parents, symlinks, hardlinks, ownership and descriptor
identity before publishing privileged paths. The systemd supervisor must retain
independent Voyage lifetimes; supervise child resources without killing voyages
on a routine supervisor restart. Test the effective unit, cgroups and dependencies,
not just unit-file text. Do not harden the supervisor with settings that prevent
its required identity changes; apply child restrictions at launch.

Keep the network gateway unprivileged. Give it only a bounded authenticated local
route; no catalogue key, raw root token, arbitrary filesystem access or automatic
administrator right. End-user authenticated authority must survive forwarding.

## 5. Safe launch and cross-identity IPC

A protected launch specification comes from authoritative supervisor metadata,
not from a runtime-writable registration, workspace config or request-supplied
executable. Pin release, identity, workspace, runtime root, account scope, policy
and expected incarnation before effects. Reserve the admission identity durably.

Use a narrowly implemented launch path that safely sets supplementary groups,
GID and UID, capabilities, environment, descriptors, working directory, resource
limits and optional namespaces. Avoid arbitrary work in a multithreaded post-fork
hook; use a reviewed launcher design with an explicit readiness/error channel.
Ordinary children must lose inherited administrative capabilities and unexpected
privileged descriptors. Administrator children receive only the approved execution
mode. All failures occur before agent execution or remain an uncertain admission
with a resolvable receipt.

Set HOME/XDG/config discovery explicitly for the execution identity. Do not inherit
root's shell initialization, SSH agent, proxy secrets, cloud credentials or an
unreviewed PATH. Authenticate runtime startup and verify actual UID/GID/groups,
capabilities, release and incarnation before claiming readiness. Use stable
process handles/start identity for cleanup; never signal a reused numeric PID.

Audit managed browser and extension subprocesses separately. Administrator Voyage
execution must not silently disable a browser sandbox or expose private input.
Use a verified supported launch/isolation arrangement, with an explicitly scoped
subprocess identity if required, or report browser capability unavailable. Preserve
Voyage ownership of that browser and cleanup; never fall back to a root browser
with disabled security merely to make the journey pass.

Replace same-UID IPC assumptions with exact expected peer identity and existing
session/incarnation authentication. Validate both directions. Keep the control
record and launch token outside runtime-writable authority. A stale socket, copied
registration, forged runtime report or token for another session cannot authenticate.

## 6. State, accounts and untrusted runtime data

Separate root-owned supervisor control from runtime-owned canonical conversation
state. Define directory traversal and per-identity ownership deliberately; do not
solve access by making directories world/group writable or recursively changing
ownership of the current tree.

The runtime owns its journal. A root supervisor must treat that journal, runtime
reports, attachment paths and other child-writable files as untrusted. Suspended
observation currently reads runtime state without a live owner: introduce a bounded
observer under the recorded identity or another reviewed read-only boundary.
Validate the projected result before privileged decisions. Do not parse arbitrary
SQLite files or follow child-controlled paths as root merely for convenience.

Account storage must have an explicit namespace and identity association. For the
first implementation, each configured execution identity uses its own authorized
device-local account context. Existing ordinary identity retains its registry and
account bindings. Administrator account availability is established during setup;
missing accounts are reported before first send. Do not silently borrow root's
login, copy user credentials, or turn a provider profile into an execution identity.
Any same-host account migration/sharing requires a separate explicit reviewed
mapping; cross-Vessel credential copying remains prohibited.

Test identity changes through OAuth refresh, API-key access, model discovery,
profile selection and billing attribution. Account helpers execute under the
appropriate identity without loading user plugins/configuration into the root
supervisor. Secret input remains private and never enters model-visible receipts.

## 7. Filesystem consent and active identity transitions

Before showing a root-access review, check actual OS access under the proposed
execution identity and relevant namespace/sandbox. Distinguish policy refusal,
Unix/ACL denial, read-only filesystem, mandatory control restrictions, unavailable
elevation and unsupported isolation. A directory-level check does not prove that
every existing descendant is writable; recheck exact targets when operating.
Avoid destructive permission probes. Include target identity and limitations in
the review, then revalidate directory identity and access after approval.

Only publish a temporary root grant when policy and tested access permit it.
Permissions can change afterwards: report later operation failure accurately,
retain the draft/task and do not replay an uncertain write. Do not chmod, chown,
edit sudoers, or grant machine-admin authority as a hidden consequence of approving
a directory. Preflight may determine that administrator mode also cannot help.

If a user chooses administrator execution for an existing ordinary voyage:

1. Prepare a typed transition review including identity, account, workspace, policy,
   conversation revision and pending work. Explain the administrator scope.
2. Obtain explicit authenticated administrative-owner approval.
3. Stop admitting new work; cancel/drain the current run and observe cleanup of
   owned shells, browser, PTYs and workers. Any unresolved effects block transition.
4. Checkpoint canonical history and fence the old owner. Perform a journaled state
   ownership migration under installer/supervisor control, never a blind recursive
   chown. Retain rollback information and validate all file types and paths.
5. Launch a new incarnation of the same voyage with the approved identity; verify
   readiness before publishing the new owner. Never have two canonical owners.
6. Resume only explicitly reviewed pending work. Prior queued tool calls and
   uncertain external effects are not automatically replayed with greater authority.

On failure preserve inspectable state and a receipt. Restore the old owner only
when the transition state proves doing so is safe. Root-to-ordinary transitions
need the same ownership, authority, secret-handling and cleanup review; history
can contain data the target identity is not permitted to receive. Do not silently
copy inaccessible artifacts or relabel a still-running root process as ordinary.

Subagents and tool children cannot request a more privileged identity than their
parent's authorized ceiling. Participant, branch and transfer destinations must
resolve identity and approval on their own host; a source UID or root approval is
not transferable authority. Unattended administrator work without a valid explicit
policy must reach a bounded pending/refused state, not an indefinite prompt.

## 8. Helm interaction and compatibility

Both Helm clients expose an **Execution user** setting independent of provider
profiles and read-only/approval/unrestricted tool policy. List only configured
identities authorized for the connection. Show a persistent administrator indicator
based on observed runtime identity; distinguish desired identity from running one.

Web uses existing Flux field/select, modal, callout and button components; no
bespoke controls without a documented reason. TUI offers equivalent setup,
review, cancellation, receipt recovery and status. On older Vessels, identify the
identity as legacy/current service user when known, otherwise unknown. Never
assume ordinary execution merely because the capability is absent.

Capability negotiation includes installation scope, supported identity selection,
available administrative review, migration/transition support and isolation facts.
Strict older decoders must never receive unsupported fields or commands. Existing
clients may continue supported ordinary operations under explicit compatibility
rules; they cannot create or silently resume administrator work without a review
surface. Add typed safe error codes plus actionable messages, without private
paths/secrets or raw subprocess diagnostics.

The path-denied journey is: request path access → preflight → explain cause →
review administrator transition if available → verified new runtime → retry only
an explicitly known-safe operation. Cancellation at every stage preserves the
conversation and records whether any effect occurred.

## 9. Adoption, update and rollback

Initial system-service provisioning requires actual administrator authority on
the executing machine. An unprivileged installed Vessel cannot grant itself that
authority. Support existing remote administration for bootstrap; do not assume
physical access or a local desktop prompt. If unavailable, report the prerequisite.

Adoption procedure:

1. Inventory and review exact source installation, identities, accounts, grants,
   services, endpoints, runtime roots and running processes. Verify a private backup
   using online database snapshots where needed.
2. Stage root-owned binaries and control state. Preserve device identity, session
   IDs and exact receipts; migrate schema transactionally and validate ownership.
3. Drain/fence the old supervisor for cutover. First implementation requires no
   active voyages during adoption; do not assume a new service can safely adopt
   old children. The user can defer until work ends. Retain suspended histories.
4. Install the system supervisor and unprivileged gateway; map old voyages to the
   explicitly reviewed ordinary identity. Enable administrative owners separately.
5. Verify exact executable/identity, authenticated readiness, account discovery,
   ordinary resume, gateway reconnection and expected OS access.
6. Commit adoption only after those checks. On failure restore the original
   installation and services when its schema/state are still compatible. Retain
   unresolved migration obligations otherwise; never launch competing supervisors.

Extend [remote-updates.md](remote-updates.md) for scope-aware system/user service
operations. Preserve pinned artifact review, exact approval, independent updater,
observed readiness, rollback and browser-worker retention. Scope and effective
service configuration are immutable facts of the approved operation. A routine
nightly update cannot switch installation mode or enable administrator voyages.
Root acquisition must use explicit download authorization and protected environment;
never inherit an ordinary user's GitHub credential implicitly.

A root updater must consume a trusted artifact produced by the publication flow,
not a local workspace build chosen by an ordinary voyage. Verify artifact identity,
manifest and all executable/browser hashes. State-schema migration declares its
rollback compatibility before activation. Uninstall disables only this installation,
retains user data by default and reports still-running owned resources.

## 10. Delivery sequence and gates

Each row is a reviewable implementation work package; open linked child issues
when implementation begins. Do not create competing project boards or assign an
unapproved release milestone. Merge normal completed increments to main with the
new capability disabled until its prerequisite gates pass.

| Phase | Work | Depends on | Exit gate |
| --- | --- | --- | --- |
| P0 | Finalize authority/lifetime/account and storage contracts; threat review; host probe specification | This plan | No implicit privilege, identity, credential or rollback assumptions |
| P1 | Versioned identity/review/receipt types, capability negotiation, catalogue migrations | P0 | New/old peers tested; no privilege fallback; legacy identity explicit |
| P2 | Cross-identity storage, IPC, observer and account namespace | P1 | Ordinary child cannot forge root control; secrets and suspended reads remain scoped |
| P3 | Launch with identity setup/drop, attestation and lifecycle coverage | P2 | Real child evidence for UID/GID/groups/caps/FDs and exact recovery |
| P4 | System installer, protected layout, gateway separation and scope-aware updater | P1–P3 | Fresh install/upgrade/failure/rollback/reboot tested in isolated Linux fixtures |
| P5 | Owner-only admin review, run authorization, filesystem preflight and identity transition | P1–P4 | Cancellation, revocation, stale/duplicate approvals and cleanup failures verified |
| P6 | Flux Web and TUI parity; supported alternate client/gateway contracts | P1, P5 | Same operation scope/status shown; stale/offline/legacy flows safe |
| P7 | Transactional legacy adoption and downgrade/uninstall handling | P2–P6 | IDs/history/accounts preserved; rollback proven; no double owner |
| P8 | Full verification, publication, hosted artifact and opt-in live activation | P1–P7 | All acceptance criteria below met with durable evidence |

Early P1/P2 work can remain unreachable behind capability gating. Do not ship a
partially exposed root installation whose launch, IPC or update path still assumes
same-user operation. Administrative execution and transition are required scope,
not items to quietly defer after adding the installer flag.

## 11. Verification matrix

Use existing focused fixtures and add tests for new boundaries; no reinstatement
of retired broad suites or hosted test/coverage jobs.

| Area | Required evidence |
| --- | --- |
| Identity launch | UID/GID/groups/capability/env/FD observations; unavailable/deleted/reused user; attempted privilege regain; ordinary child cannot write protected fixture |
| Authority | Owner vs ordinary/scoped connection; forged identity/review; wrong Vessel/session; expiry/revocation; repeated and concurrent approvals; unattended refusal |
| IPC/storage | Wrong peer UID/incarnation; stale/replaced socket; symlink/hardlink/path races; tampered registration; hostile SQLite/attachment paths; per-identity observations |
| Lifecycle | New, configured, resumed, suspended, branched, imported, transferred, participant and workflow launches; restart and reboot; crash between each admission/transition step |
| Transition | Root and ordinary directions; no simultaneous owners; live browser/PTY/shell cleanup; unknown effects block replay; inaccessible artifacts/history refusal |
| Accounts | Existing bindings remain stable; per-identity enrollment/refresh/model discovery; missing admin account preflight; no credential leakage or copying |
| Filesystem | Policy vs OS denial; ACLs, unwritable descendant, read-only mount, replaced directory, unsupported isolation; no false grant or silent permission edit |
| Installer/update | Root-owned paths; user-writable ancestor refusal; scope confusion; changed unit/release review; downloader failure; readiness failure; rollback and browser assets |
| Clients | Real TUI journey and shared-browser Flux journey; retained draft; admin indicator from observed identity; disconnect during approval/transition/update; exact receipt recovery |
| Host diversity | Disposable Linux VM with native root, unprivileged user, multiple users; namespace/container limits as a separate case; unsupported host refuses clearly |

Root/system-service tests run only in explicitly designated disposable fixtures,
never by changing the developer workstation's users/services or using production
credentials. Namespace-only success does not establish native machine-root
behavior. Linux results do not establish macOS/Windows support.

For each Rust/Cargo/test delivery, run final workspace coverage per AGENTS.md,
check the test exit status, select all current workspace objects and publish
`coverage/latest.json` with source identity and counts. Run focused package checks,
formatting, Clippy and client/gateway checks appropriate to each change. Validate
actual changed behavior; do not count a compile as installation or runtime proof.

After authorized main publication, inspect existing nightly runs, dispatch the
existing build-only workflow if needed, follow it to completion and inspect actual
BUILD.txt source, archive checksum, binaries and browser-asset inventory. No stable
release is implied. Record exact commits, run URLs and artifacts in #344.

Live acceptance uses an opted-in host after inventory and review. Through the
shared browser: create ordinary voyage → observe controlled OS denial → approve
administrator run/transition → verify actual UID and write a designated protected
test file → verify ordinary voyage remains unable to write it → reconnect/restart
and verify authorization/receipt behavior → verify update and rollback. Use fixture
providers for automated runs; obtain a new provider/budget approval for any further
live inference. The earlier two-prompt authorization is consumed.

## 12. Completion criteria and rollout

The feature is complete only when:

- A reviewed system installation starts the privileged supervisor and launches
  ordinary voyages under the configured unprivileged identity.
- Explicit owner review launches an administrator voyage with accurately reported
  authority; ordinary connections and tools cannot self-elevate.
- Filesystem review reports real access and limitations; the motivating approved
  write works on a suitable fixture without hidden permission changes.
- Every launch/recovery/transition route preserves identity, single ownership,
  exact receipts and uncertain-effect fencing.
- Existing installation adoption retains accounts, histories and device identity,
  with verified failure recovery and rollback boundaries.
- Both clients and the installer/updater pass their actual supported journeys.
- Required coverage, published source, hosted artifact and native/browser evidence
  are linked in the delivery record. Remaining unsupported platforms and host
  prerequisites are explicit.

Roll out first in disposable native Linux fixtures, then an explicitly selected
real host. Select hosts by observed capabilities and owner intent, never names or
assumed topology. Do not silently migrate the other connected Vessels. Tax-Axis's
previously deferred access remains deferred. A final operational guide must cover
installation, administrative owner enrollment/revocation, identity management,
account setup, transition recovery, update/rollback and uninstall.

Before implementation leaves P0, confirm the proposed per-run administrator
consent lifetime and whether account namespaces must be shared on the same host.
Until changed explicitly, use fresh administrator review and separate authorized
account contexts. These are recorded defaults, not assumptions about any host.
