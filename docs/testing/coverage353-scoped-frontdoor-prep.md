# Scoped frontdoor and saved-authority cohort (#353)

SOURCE preparation only. No tests, Cargo/builds, measurement, native effects,
provider calls or human credentials were used. The isolated branch is
`coverage353-scoped-frontdoor`, based on committeddb3d358. The prior measurement,
profiles, Main changes and current hosted/native qualification remain untouched.

## Actual defect and narrower admission

[Source defect record](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5946588908):
Capabilities omitted saved account/enrollment vectors from the socket's authority
snapshot. Fresh grant authentication alone could accept a changed scope with the
same revision; its comparison was against a fresh grant, not the socket's original
context. Connection token rotation also was omitted from the final current-grant
comparison.

An opaque domain-separated SHA256 of the **complete typed saved** ConnectionGrant
or ProcessGrant now accompanies authenticated Capabilities. It includes every
nested parent/connection/participant field, token hash (never raw token), identity,
revision, expiry/revocation, rights and all saved scopes. Stable typed serialization
is unambiguous. Owner workspace discovery is derived metadata outside this hash;
the saved owner vectors and frozen v1.0.2 twelve-right bytes are unchanged.
A fingerprint is metadata, not an access credential or permission.

Current servers advertise `saved_authority_pin`. Their frozen socket adapter
refuses a missing/malformed fingerprint; initial bootstrap and explicitly older
unadvertised capability documents keep the existing None compatibility. Native
and browser sockets use the same adapter. The optional expectation is carried
only through the private Granted envelope/GrantAuth pipe and omitted when None,
preserving legacy envelope bytes. Public clients cannot override that envelope.
SystemOpenSocket uses the same frozen auth/pin clone as subsequent SocketCommand;
its exact immutable credential/identity/context equality is unchanged.

The core compares that expectation with the **same authenticated current grant**
before dispatch, not merely a prior cached Capabilities check. Existing command
rights, current_scope, revocation and intent binding remain authoritative. Final
connection rechecks include token identity; session/account/epoch checks compare
the full saved authority so token/scope changes cannot reuse an earlier context.
No grant is rewritten, no new right is inferred and no unknown effect is replayed.

Both wire consumers were inspected: protocol reply results are JSON Value,
Helm access Metadata accepts added fields, and Web PublicVesselHttp/VesselGateway
validate required protocol/identity/features without a closed capability-key list.
No public command schema or raw credential field was added. Private expectation
None roundtrip and exact SystemOpenSocket auth equality have prepared regressions.


## Late authority changes after successful dispatch

[Related outcome defect](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5947018021):
the final saved-connection check runs after dispatch has succeeded. A late grant
rotation/revocation previously produced a generic error and a false definite
`outcome_unknown=false` reply despite an already accepted effect. The factored
production reply boundary now marks that postdispatch failure with the existing
`OutcomeUnknown` type. The session boundary reauthenticates the same held grant and compares the complete
typed saved fingerprint, including nested bindings, accounts, enrollment, token
hash and expiry. All preflight authentication/fingerprint/rights refusals
remain definite; no grant fence is relaxed and no operation is replayed.

Vessel commands have no exhaustive effect classifier, so the failed late check is
conservatively unknown for any successful dispatch, including a read. The payload
is withheld after authority loss. Successful reads retain normal definite replies
and the existing history redaction. This is an observation of the original exact
operation, never permission to automatically retry or disclose a revoked reply.

Four additional source-prepared parents cover connection and session private SQLite acceptance
settlements followed by token/rights/revision/revocation drift and complete session account/
enrollment/parent/participant authority changes, exact command and
accepted destination state retention, normal read/history redaction, and frozen
pin refusal before a Start intent/session is created. The accepted-effect tests
invoke the actual notification branch, then the same factored production late
checkers; it is a **boundary-level** deterministic test, not an end-to-end scheduler
race, native owner or provider claim. Tests remain unexecuted pending the parent gate.

## Prepared assertion families

- Private connection current-principal/revision/schema/right/account/enrollment/
  workspace/expiry/revocation/authidentity drift preserves derived grant bytes.
- Exact durable command intent rejects changed payload, principal or revision;
  no runtime/registration is created by the intent-only fixture.
- Start nil IDs, noncanonical/unapproved paths, configuration and existing-session
  conflicts refuse before intent/owner creation. WorkspaceRead, cancellation,
  terminal and lifecycle rights remain separate from an owner flag.
- Frozen account/enrollment changes and loaded token rotation refuse at core
  admission before any lifecycle intent/session mutation. Complete saved process
  fingerprints include nested scopes; unchanged legacy owner and newly discovered
  workspaces keep their authority fingerprint unchanged.
- Ordinary scope checks/private guardian framing retain grant/parent/revision
  checks and fixed null refusals. No protected Root lease is successfully minted.
- Nineteen actual owned ordinary binary-test child journeys run production
  `process::serve`, public native header authentication/SocketBackend, real loopback
  WebSocket transport, authority changes, correlation replay/nil/protocol/binary/
  oversized/private-envelope refusals, subscription isolation/unsubscribe and
  application liveness expiry. They assert zero Voyage/session creation and
  positive tracked child SIGTERM/exit/discovery removal, with private bounded logs.

The child runs in its private cwd/HOME/data/state with cleared environment; only
its instrumentation profile setting is inherited. It uses the current compiled
binary-test executable and requires a current audited Voyage entrypoint (no Voyage
execution is requested). No normal new binary/object or dependency is added.
Loopback transport is not deployed publicTLS/Web, native Root or provider proof.
Owned resources are tracked by UID/executable/start before termination; failed
cleanup retains uncertainty/evidence and cannot be called passing.

## Gate after every current source cohort is final

Do not compile/run during preparation. Root owns the combined window. Include
these focused filters with the workspace instrumentation and private bounded unit:

- `scoped_frontdoor_tests`
- `session_late_authority_tests`
- `scope_authority::ordinary_family_tests`
- `process_http::owned_frontdoor_tests`
- `saved_authority_wire_tests`

Use the existing retained target, one compiler job, bounded memory/no swap and
`RUST_TEST_THREADS=2`; do not combine incompatible `--no-clean --no-report` flags.
Current runnable Voyage must be built/audited by the gate owner, including source,
version/depfile/ELF/hash witness and all current object/profile entries. Child BIN
results are nested under the parent Cargo target, not additional parent tests.
Resolve concrete failures before strict formatting/Clippy and the independent full
workspace measurement. Preserve previous raw/object evidence, audit complete current
objects/files, commit the compact summary and own main/actual hosted archive
follow-through. No test-prep or percentage improvement closes353 or defers its
full reachable-production objective; Root/system/adoption/platform/browser/provider
obligations remain separately accounted for without denominator pruning.

## First coordinated gate fixture corrections

The full gate on frozen `78ff047` retained three library failures and one owned
BIN failure. Terminal's synthetic JSON omitted its mandatory run/terminal IDs
(Cancel already supplied its run ID). The two older account attenuation fixtures
had no saved authoritative child record, so the full saved-scope guard correctly
refused before their intended parent/participant checks. They now persist each
explicit synthetic child variant and assert the intended attenuation/refusal label.
No production scope or parent guard is weakened.

The BIN failure was a write-side `ConnectionReset`; the retained original fixture
has no case ID, so its exact loop iteration is not independently established.
Source identifies the deliberate `MAX_FRAME_BYTES+1` frame as a path whose production
frame guard can close from its length header before the payload flush. The correction
is restricted to that premise; the parent's rerun must confirm it resolves the
actual failure, and any other failing iteration remains a failure. Only that
oversized refusal case accepts
write-side reset/broken-pipe; all other frame writes remain strict. It still must
observe closed transport, no command reply/session creation, unchanged grant and
positive owned service retirement. Private case metadata now identifies any
retained failed child fixture. The original failure directory/logs are preserved.
These isolated corrections passed rustfmt/diff checks only; the parent owns the
relevant rerun and fresh final measurement after the initial gate terminates.
