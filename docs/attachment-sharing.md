# Current sharing authority — partial #79

Tracking: [#79](https://github.com/o-psi/voyage/issues/79#issuecomment-5552642329),
under the [full attachment contract](attachment-production-contract.md).

`helm::attachment::sharing::SharingRegistry` is the sole owner of current sharing
policies in one process. It owns authorization checks over its current in-memory values.
`PolicySnapshot` contains local observations only; it has no authorization,
projection or branch methods. General sharing adapters remain unwired; the current
[dedicated remote worker](remote-sessions.md) uses a separate fixed-session binding with durable local withdrawal. Its Journal
transaction fences admission and public projection after withdrawal; this does not
activate general sharing policies or confer positive authority from snapshots.

Each operation reads current consent from the registry and requires matching
installation/owner/epoch, the installation's delegated capability, the authenticated
principal's capability, and the disclosure/archive/approval policy. Grants have no
request deserializer: callers must construct them from verified authentication,
not copy claimed identities or capabilities from a command. Enrollment administration
and session creation capabilities do not grant existing-session visibility or control.
Capabilities do not imply each other. In particular, `CancelAny` does not synthesize
a missing `CancelOwn` in an otherwise disjoint capability intersection.

Cancellation requires trusted attribution for the requested run from the canonical
journal. The session must match; `CancelOwn` additionally matches the authenticated
principal to the recorded run owner. `CancelAny` permits another principal's run only
when both delegation and the principal grant contain it. Neither permits cancellation
without a known run target. Do not use request-provided ownership claims or authorize
one run and then dispatch cancellation against a different request ID.

Disclosure rules use this matrix: none permits nothing; metadata permits
metadata observation only; live permits live observation and separately authorized
mutations; transcript also permits history. Archived sessions cannot run, branch or
approve tools. Separate local opt-ins and both capability grants are required for
write and command approval. Local roots, hard denials, resource limits and approval
decision verification remain separate mandatory checks; this library does not execute
or approve an action.

Trusted-local `register_local` creates a policy once. `update_local` uses an expected
revision and replaces settings atomically in memory; stale or overflowing updates
leave current consent unchanged. Unshare, archive and approval-disable updates affect
all later checks through that owner. Branching reads and authorizes the current source
under the same write lock as child-policy creation, requires both history and branch
capabilities, rejects stale source revisions/private or archived sources, cannot widen
disclosure, and resets approval opt-ins. These local control APIs must not be exposed
as unvalidated remote handlers: remote consent/destructive actions still need explicit
confirmation, authorization, audit and a storage transaction.

Installation epoch changes must increase monotonically. They invalidate old grants
and policy consent; explicit local updates re-consent each policy at the new epoch.
The valid epoch range matches enrollment: 1 through `i64::MAX - 1`. The registry holds
at most 4,096 policies and retains at most the distinct typed capabilities per grant.
Poisoned registry locks fail closed. No sessions or credentials are deleted here.

**An authorization boolean or returned projection is not a reusable permit.** Recheck
current authority when committing dispatch or publishing queued output, together with
current local policy, expiry, cancellation and resource checks. The registry lock does
not cover a caller's later effect or network send. This implementation provides no
cross-process, restart or disk-transaction fence. The session coordinator must persist
sharing generations and join authorization checks to its actual commit boundary before
remote exposure. A process-local `Arc` is not a substitute for that integration.

Per-session sharing and installation delegation are not durable yet. Retention/cache
expiry, full lifecycle confirmations/audit, stream lease expiry, cross-process revocation,
private ancestry semantics after later source changes, and network/UI verification remain
open. Already disclosed content cannot be promised retractable. No enrollment, transport,
service or provider path is enabled by this change.

`cargo test -p helm --lib attachment::sharing` covers the complete prior disclosure,
archive and approval matrix; capability intersection on both sides; enrollment-only and
view-only denial; own/foreign/session-mismatched cancellation; stale consent and epoch
changes; branch authority; concurrent CAS; fail-closed poisoned locks; capacity and
revision boundaries. Tests use synthetic identities and in-memory state. They do not
establish durability, remote security or platform deployment readiness.
