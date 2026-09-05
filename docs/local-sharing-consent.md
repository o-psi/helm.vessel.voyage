# Durable local sharing declarations

`attachment::sharing::consent::ConsentStore` records explicit local sharing
choices in a dedicated private directory. It is an administrative library:
opening a store, enrollment, and reading a snapshot do not authorize disclosure,
execution, approval, or restoration of a cached `SharingRegistry`. No frontend or
remote authorization adapter is connected to this store.

A trusted local caller obtains its installation/principal from `LocalActorStore`
and derives destination origin, machine, owner and epoch from verified enrollment.
The store checks those bindings; it does not authenticate caller-supplied identities
or contact Vessel. Enrollment alone never supplies consent.

## Explicit changes and observations

The caller constructs a `ConsentChange` containing a fresh operation UUID, actor,
session UUID, expected revision, destination and `SharingSettings`. It calls
`preview_local`, presents the previous and proposed settings to the local operator,
and passes the confirmed digest to `commit_local`. The digest binds the complete
preview; it is integrity evidence, not a credential or proof of human interaction.
The caller must implement the actual confirmation interaction outside model input.

An absent session has no declaration and revision zero. Every new change requires
the current revision and increments it once. Changing destination or enrollment
epoch requires a new confirmed change. Unsharing records a persistent `None`
tombstone with no destination and both approval opt-ins disabled. Records do not
contain conversation text, credentials, provider state, terminal input or proof
secrets. Their actor, destination and session metadata is still private.

Exact retries retain the original operation UUID, request and digest. They return
the immutable historical receipt even if later declarations advanced the session.
A receipt therefore does not describe current policy. `inspect_local`, `list_local`
and `audit_local` reread validated evidence under a private lock; their results are
inert metadata too. A future authority adapter must recheck current enrollment,
policy and local execution authority at the relevant operation boundary.

Disclosure settings describe intent. This store neither transmits content nor
implements transcript retention or deletion. Live sharing does not promise history
retention, and unsharing cannot retract bytes already disclosed elsewhere.

## Storage, retries and failures

The store uses the local actor's private directory primitives: pinned Unix file
access and the existing native Windows private-storage implementation. It validates
single-component filenames, bounded reads, ownership/link constraints and immutable
publication. Each operation takes a nonblocking directory lock and releases it
before returning. Resolve enrollment and actor state before calling it; do not hold
other Journal, network or execution locks across these calls.

The immutable header binds the actor. Numbered records bind previous snapshots and
a hash chain. Each published record also requires a matching immutable publication
witness. A staged candidate alone is never effective consent. While a valid candidate
is pending, snapshot/list/audit reads fail closed; only the exact matching operation
can finish publication. Witness-only state cannot restore an older grant. A witness
may be completed only with its exact valid staged candidate. Missing, conflicting,
partial or malformed evidence fails closed and is not overwritten automatically.
Preserve such evidence for explicit operator repair; there is no destructive repair
or compaction command in this library.

Publication syncs the candidate, witness and directory before publishing the record,
then syncs the directory again before acknowledgement. A lost acknowledgement can
be recovered through the original exact retry. The existing filesystem helper's
platform guarantees apply; Linux failure injection does not establish Windows
power-loss durability. Hashes and witnesses detect inconsistent evidence, not a
storage owner's deliberate rollback: deleting/restoring both record and witness,
or restoring a complete older backup, is outside this local trust boundary.

A store holds at most 4,096 records of 16 KiB each. Pages contain at most 100 entries.
At capacity, existing receipts remain observable but new changes are refused.
There is no silent pruning of revocation tombstones or audit history.

## Verification

`cargo test -p helm --lib attachment::sharing --all-features` exercises explicit
consent, destination/actor/CAS binding, immutable retry, tombstones, pagination,
actual capacity, corrupt evidence, link attacks, competing handles and independent
process locking. Real subprocess exits and injected publication failures cover
staging, publication and lost acknowledgement. A regression proves that removing
the final record cannot restore the preceding grant. The existing local-actor tests
cover its unchanged identity and lock behavior. Platform-specific native runs are
not claimed by the Linux checks.
