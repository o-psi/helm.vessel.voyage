# Full attachment delivery contract

Tracking: [#77](https://github.com/o-psi/voyage/issues/77#issuecomment-5548053996),
[#9](https://github.com/o-psi/voyage/issues/9), [#10](https://github.com/o-psi/voyage/issues/10),
[#78](https://github.com/o-psi/voyage/issues/78), [#79](https://github.com/o-psi/voyage/issues/79).

**Status: implementation contract, not a production-readiness claim.** The operator
explicitly requested the full system, not an MVP. Earlier foreground-only and
post-MVP scope exclusions are superseded. Dependency ordering is an engineering
sequence, not permission to omit service management, full session lifecycle,
remote approvals, or notification/handoff from delivery.

## Authority and trust

**Selected deployment: single owner controlling their own Helm machines, with no
untrusted remote operators.** This was explicitly selected through the clarification
tool and confirmed in [#77](https://github.com/o-psi/voyage/issues/77#issuecomment-5548263061).
Multi-tenant OS isolation is outside this delivery scope; all authentication, local
policy, consent, revocation, audit and credential protections below remain required.

Helm is the sole execution and canonical-session authority. Vessel authenticates
operators and routes commands; it never receives provider credentials or opaque
provider continuation. Every effective permission is the intersection of current
local policy, installation delegation, principal capability and session sharing.
Approval can satisfy an approval requirement, never override a hard denial.

A Vessel-authenticated principal assertion trusts that Vessel to authenticate its
operators within the installation's locally delegated scope. This does not defend
against a compromised Vessel impersonating one of those operators. End-to-end
principal-signed approvals require a separately verified key-registration contract;
unsigned UI assertions must never be advertised as end-to-end authentication.

Sharing ACLs are not process isolation. Shell/tools running as one OS account can
access that account's resources. Mutually untrusted operators require separately
isolated execution identities/workspaces and verified OS controls; application
policy must not be advertised as that isolation. [#62](https://github.com/o-psi/voyage/issues/62)
tracks native sandbox adapters. Remote PTY keystroke transport is not implicitly
authorized by session attachment or approval delegation.

## Storage and command admission

New attachment storage is separately versioned. Legacy `voyage.json` and Vessel
`control_plane` snapshots remain inert; no automatic credential or task migration.

Canonical session state, accepted command identity/digest, run state and event
sequence must commit in one transactional boundary before acceptance is published
or provider/tools execute. A failed or uncertain commit never permits dispatch.
A repeated command with the same machine/principal/payload returns its original
outcome. Reusing its ID with different arguments, principal or target is a conflict.
Deduplication must outlive bounded replay; expiry cannot make a previously accepted
command executable again. Capacity exhaustion rejects new work, not old evidence.

One fenced owner may execute a session. Short storage transactions are distinct
from run-long execution ownership. A `save()` lock alone is insufficient: two
processes could already have executed effects from stale snapshots. All local
CLI/TUI mutations and remote operations must use the same coordinator. Arbitrary
local `load_reference(PATH)` is not a remotely callable session lookup.

On restart, abandon neither accepted turns nor partial output. Runs with uncertain
outcomes become interrupted; reconnect replays observations, not external effects.
A cancellation acknowledgment is not confirmation that the process has stopped.
No success response may substitute for unknown/failed persistence or cleanup.

## Enrollment and credential lifecycle

- Invitations are administrator-authorized, scoped, high-entropy, short-lived and
  consumed atomically; Vessel retains verifier hashes, not raw keys.
- Persist local enrollment transaction identity and a proof-of-possession key
  before redemption. Bind recovery to transaction, installation key, origin and
  enrollment epoch. Fresh challenges prove possession; consumed keys do not regain
  general enrollment authority after a lost response.
- Rotation is idempotent and proves both old authorization and new-key possession.
  Finite recovery/overlap windows must not extend through repeated retries.
- Revocation durably advances the authorization epoch before acknowledgment and
  invalidates queued commands, pending approvals, reconnect and rotation recovery.
- Disconnected execution has a bounded authorization lease; expiry stops new effects
  and initiates scoped cancellation. Already-completed effects cannot be undone.
- Detach works locally offline, preserving sessions and distinguishing local disable
  from confirmed remote revocation. Backup restoration cannot silently resurrect
  revoked authority.
- HTTPS is required except explicit loopback development. No credential-bearing
  redirects, query-string secrets, logging of keys, or silent server-origin changes.
- Secure prompt/stdin alternatives accompany positional join keys. Service secret
  storage must have verified platform-specific ownership/ACL behavior.

## Protocol and privacy

Replacement envelopes use a separate version and typed operations; legacy v1 is
not negotiated. Reject unsupported versions, unknown fields/operations, nil IDs,
oversized frames, invalid revisions and expired commands before admission.
Workspace selection uses locally configured identities, not arbitrary remote paths.
Distinct identities represent installation, connection, principal, session, run,
command and event cursor. Delivery, acceptance, execution and terminal states are
not interchangeable.

Commands include all session lifecycle operations, not just create/submit/cancel.
Sharing covers none, metadata, live events and explicit retained transcript modes.
Existing local sessions are private by default. Branches preserve source disclosure
restrictions; enrollment administration does not grant transcript access. Authorize
lookups, subscriptions, replay, snapshots, export, branch and destructive operations
individually, including after ACL changes and reconnect. Retention uses explicit
byte/count/TTL bounds and backup/deletion disclosures. Never serialize a raw Helm
Session into an outbound frame.

## Approval decisions

The durable approval record binds installation/authorization epoch, principal scope,
session/run/tool-call IDs, local workspace/policy identity, immutable full action
arguments and relevant input hashes, with finite expiry. Redacted presentation text
is not action identity. Decisions use a single atomic state transition for local
and remote clients. Execution rechecks current policy, expiry, cancellation,
revocation and resource preconditions at dispatch commitment. A changed action needs
new approval. Crash recovery never replays an uncertain approved external effect.
Notifications carry opaque references, not approval authority; opening a link cannot
approve. Authenticated, CSRF-protected decision submission is required.

## Production acceptance map

| Delivery | Required evidence before completion |
| --- | --- |
| Enrollment #10 | Expiry/replay/concurrent redemption, lost responses at each commit, rotation overlap, stolen/revoked credentials, secure storage/URL/redirect tests |
| Transport #9 | Current/unsupported-version fixtures; duplicate/conflicting commands; deadline/sequence gaps; slow clients; bounded replay; disconnect and restart at acknowledgments |
| Coordinator #78 | Real multiprocess contention; one provider/tool dispatch per admitted run; CAS conflicts; accepted/partial output; delete/finish races; disk-full and crash injection |
| Sharing #79 | Principal/machine/session matrix; private branch ancestry; revoke while streaming; sensitive metadata/projection; cache expiry/deletion/backup tests |
| UI #14 | Browser auth/CSRF/XSS, accessibility/responsive layouts, duplicate clicks/stale revisions, offline/interrupted states, full lifecycle and destructive confirmations |
| Approvals #21 and handoff #72 | Exact-action binding, stolen/expired/replayed decisions, local/remote races, unavailable approvers, cancellation/revoke/restart, notification delivery failure |
| Services/operations #18 | Linux/macOS/Windows install/start/stop/reboot/upgrade/rollback/uninstall; private identities; no implicit elevation; backup and revocation-rollback drills |
| Release | Full workspace/system tests; offline native-provider E2E; reviewed budgeted live-provider evidence; platform CI; load/failure/security evidence; packaging, SBOM/signing and operator runbooks |

These rows stay unfinished until implementation and actual verification exist.
Foundational library tests are not end-to-end attachment evidence. Earlier cleanup
checks do not establish readiness of the replacement system.
