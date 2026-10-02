# Ordinary signed courier failure and recovery cohort (#353)

Source checkpoint only: the 18 Helm whole-loop and 16 actual Supervisor parents
are uncompiled/unrun. Additional pinned-context boundary cases are being prepared
before the coordinator's source-final gate. This independent work starts from
qualified published `c3f997f`; it changes no Main, target, profile, running host,
provider or real credential. Scope and production findings precede implementation:
[courier admission](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951104943),
[activation uncertainty contract](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951433174),
[pre-Accept recovery](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951550965),
[chunk admission/ACK](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951864159).
The cohort belongs to [release #375](https://github.com/o-psi/helm.vessel.voyage/issues/375).

The current map's 181 zero addresses in Helm's `admin/transfer.rs` are input
candidates, not estimated gains or a reachable-total shortcut. The full measured
denominator remains. This ordinary local account operation is distinct from
notification courier, scoped account/gateway, Root/system/adoption/migration,
other platforms and provider acceptance. Helm transports a signed artifact; it
does not become a conversation executor or manufacture another canonical owner.

## Correctness changes

- Existing dangling journal links now refuse before endpoint contact rather than
  looking absent and being replaced by a new operation.
- New private journals record Receiving and ActivationPending. The courier
  uploads first, saves Pending before sending one original activation, and stops
  on loss/unknown/refusal. The speculative pre-upload activation is removed.
- Saved unresolved manifests use owner-local readonly `TransferStatus`, gated
  by `signed_transfer_status_v1`. Request/reply bind the exact transfer/activation
  IDs, retained signed manifest/digest, pinned source/destination/session and
  signed artifact identity. Pending never authorizes effects. Complete contains
  a sanitized exact historical creation receipt, never current process liveness.
- Supplying the retained manifest permits readonly validation even when the
  old journal checkpoint preceded Accept. Status does not accept, save, pin or
  reserve that manifest or any activation ID. Only a verified Receiving result
  permits the subsequent normal idempotent Accept and exact chunks.
- Receiving requires a prior private catalogue namespace witness captured before
  Prepare effects, plus consistent absence of lifecycle admission/receipt, any
  current or historical target registration, activation intent and completion.
  A false `activated` bool or two reads of a newly empty database are insufficient.
- Status requires existing coherent identity/key/public files and an existing
  readonly compatible SQLite namespace. Missing/corrupt/replaced/unsafe files,
  schema or namespace drift refuse; no create, migration, key mint or repair is
  permitted. Private metadata/descriptor/path identity and SQLite HAS_MOVED are
  checked, including artifact time/leaf stability and dangling-link refusal.
- Per-transfer ordering precedes registration serialization. Activation saves
  its original intent before entering startup; uploads refuse pending/admitted
  targets before writing. The existing startup future runs in a private pinned
  namespace context: every DB blocking call captures it before changing threads,
  opens only existing RW/no-create/no-migration state and checks namespace before
  and after. A prelaunch check retains uncertainty on later drift. Unrelated
  ordinary Starts keep their existing behavior; no environment/tool switch exists.
- Source chunk data is bounded, valid STANDARD base64 and exactly matches the
  claimed byte boundary before Upload. The one acknowledgement must match its
  transfer/next/bounded stored bytes before progress. A bad post-dispatch ACK
  remains uncertain and cannot trigger subsequent effects automatically.

## Genuine historical proof limit

Older private Prepared records did not retain the original catalogue device/
inode. Neither a current startup observation, a valid empty replacement database,
nor absence in a rebuilt map can prove that an earlier activation never happened.
These witnesses are never reconstructed. Such old unresolved records remain
Pending and cannot authorize upload/activation; an exact bound historical
creation receipt can still return Complete without effects. An unsupported older
backend retains saved ambiguity with an upgrade requirement. New anchored records
support full Receiving recovery, including old-format courier journals against a
current backend. This does not claim safe upload resumption for every historical
unanchored preparation; the missing original proof is real and remains explicit.

## Prepared behavior and evidence boundaries

Helm's 18 parents use actual authenticated local public duplex command/reply
correlation, strict pre-effect journal observations and frozen args/UUIDs/keys/
IDs/deadline. They cover private leaf/parent/lock/retired route refusal, new flow,
terminal reopen, lost preparation/export/upload/activation, explicit original
recovery, Receiving/Pending/Complete and unsupported legacy status, malformed
chunk and status/ACK boundaries, awaited cancellation and observed socket/task/
listener/lock retirement. Signature envelopes are typed opaque fixtures: no
cryptographic trust, native ownership or real conversation is inferred.

The 16 Supervisor parents invoke the actual dispatcher with real local signing
keys and private SQLite metadata. They compare the entire bounded private graph
(bytes/inodes/modes/times) across readonly/error calls; exercise pre-Accept,
partial/full artifact observations, prior namespace/intent/admission fences,
historical exact completion versus later incarnation, signature/key/public/schema/
database/unsafe artifact refusal, owner/scoped capability isolation and held
transfer/registration locks. Historical receipt rows are explicit metadata
fixtures. A fixed missing runtime image demonstrates actual admitted Unknown
before a process can launch; this is not native owner or cleanup acceptance.

Failed fixture journals remain private. Requests, buffers, peers, metadata,
graphs and waits have explicit bounds. Cancellation is not called observed
cleanup. No executing Voyage, provider, human private input, real grant/credential,
host service or native Root/other-platform operation is created by preparation.

## Pending gate

Rustfmt/diff checks are source checks. Independent production/test review,
pinned-context cases and all combined source edits must finish before the root
coordinator's one applicable workspace gate/coverage/current-object audit. Exact
new parent counts, outcomes, compact coverage, normal Main publication and hosted
build follow-through remain pending. No skipped test or local branch is delivery.
