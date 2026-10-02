# Ordinary signed courier failure and recovery cohort (#353)

## Verified local result

Clean measured source `e10603830e17d3428f8154140ff0b983f20fd337` passed
formatting, strict workspace Clippy and the full workspace run: **2936 passed,
0 failed, 8 ignored** across 15 Cargo targets. The combined cohort has 73 new
semantic parents (18 Helm courier, 21 Supervisor status/context, 18 extension/
UserFile, 16 ordinary assessment), plus three inert default child entries.
Nested child summaries are retained separately and are not added to parent totals.

The audited 18 current objects retain all prior production mappings. Corrected
production coverage is **108676/135727 lines (80.069551%)**, +1453 covered lines
and +752 measured lines versus `1d49636`. All 235 foreign profiles are unchanged;
511 current profiles, summary/detail/HTML and full JSON/LCOV accounting agree.
The full reachable-production target remains unmet.

The earlier `43901f9` run also passed 2936/0/8, but its report counted two new
test-only files whose names lacked the standard `_tests.rs` suffix. That report
is preserved and rejected. The filenames and path declarations were corrected,
and the complete measurement repeated using unchanged standard exclusions.
No production source was excluded and no totals were manually subtracted.

Pre-test failures and their logs are retained: error-propagation syntax, an
unavailable `tempfile` reference replaced by the maintained installer fixture,
and an inventory type alias required by Clippy. Assertions and production
refusal behavior were preserved.

Normal Main publication and exact hosted build/archive verification subsequently
passed, as recorded below. The locally measured source and published build source
remain distinct. This delivery establishes none of
the separate browser, native platform, Root installation or stable release gates.

At the original source checkpoint, the 18 Helm whole-loop and 21 actual Supervisor
parents were uncompiled/unrun; the verified local result above supersedes that status. This independent work starts from
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
  Old cached courier results are not completion proof: they are reconciled through
  status too, because an older duplicate activation could return an unavailable
  process projection without a creation receipt. Pending/refusal preserves that
  original result; a missing retained manifest refuses before contact.
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
Legacy cached unavailable projections now explicitly exercise Pending, missing
capability, lost/refused status and contradictory Receiving without changing the
old result/bytes/IDs. Only Complete replaces the projection with the bound
historical receipt; a missing manifest refuses before any endpoint contact.

The 21 Supervisor parents invoke the actual dispatcher with real local signing
keys and private SQLite metadata. They compare the entire bounded private graph
(bytes/inodes/modes/times) across readonly/error calls; exercise pre-Accept,
partial/full artifact observations, prior namespace/intent/admission fences,
historical exact completion versus later incarnation, signature/key/public/schema/
database/unsafe artifact refusal, owner/scoped capability isolation and held
transfer/registration locks. Historical receipt rows are explicit metadata
fixtures. A fixed missing runtime image demonstrates actual admitted Unknown
before a process can launch; this is not native owner or cleanup acceptance.
The five pinned-context cases additionally exercise root/namespace nesting,
cancellation and concurrent unpinned operations, changed namespace before actual
admission and after actual admission before the production prelaunch check,
original request/intent retention, and own admission/receipt writes within the
same namespace. The prelaunch case qualifies the actual boundary function with
no launch call; it does not claim a raced live process fixture.

Failed fixture journals remain private. Requests, buffers, peers, metadata,
graphs and waits have explicit bounds. Cancellation is not called observed
cleanup. No executing Voyage, provider, human private input, real grant/credential,
host service or native Root/other-platform operation is created by preparation.

## Delivery obligations

Independent source review, the pinned-context cases and the complete local gate
are finished as recorded above. Normal Main publication and exact hosted archive
follow-through are also verified below; remaining #353/release acceptance stays open.

## Published source and verified hosted artifact

Normal Main **`036ea6e487114dd2ae34ed30fce0f53ab1dd6919`** was published and
verified clean against remote Main. It includes the corrected standard-scope
measurement of clean **`e10603830e17d3428f8154140ff0b983f20fd337`** and outcome
records. The earlier report which counted two new test files by their filenames
was rejected; `_tests.rs` names and a fresh full run restored the existing standard
scope. No counts were manually subtracted and no production source was excluded.

Actual [run 37012904856](https://github.com/o-psi/helm.vessel.voyage/actions/runs/37012904856)
terminated **SUCCESS**, performed a new build (not a skip), and identifies exact
publication source `036ea6e` in checkout/source logs, archive `BUILD.txt` and public
release target. Version **`1.0.3-nightly.20261002.37012904856.1`**. The hosted
workflow is build-only; the passing **2,936/0/8** test and **80.069551%** line
coverage results above are local. There are 73 semantic new parents plus three
inert default child entries; nested child results are not extra parent counts.

The public [development prerelease](https://github.com/o-psi/helm.vessel.voyage/releases/tag/nightly-1.0.3-nightly.20261002.37012904856.1)
contains the [archive](https://github.com/o-psi/helm.vessel.voyage/releases/download/nightly-1.0.3-nightly.20261002.37012904856.1/voyage-1.0.3-nightly.20261002.37012904856.1-x86_64-unknown-linux-gnu.tar.gz)
and [checksum](https://github.com/o-psi/helm.vessel.voyage/releases/download/nightly-1.0.3-nightly.20261002.37012904856.1/voyage-1.0.3-nightly.20261002.37012904856.1-x86_64-unknown-linux-gnu.tar.gz.sha256).
Anonymous guarded download and independent verification both exited zero under
1 GiB with zero swap. The **45,933,005-byte** archive SHA-256 is
**`9aa418c94b0977a8ccf278b2489b16d31f9b4efc4cdca115c18d61a2b106f432`**;
public checksum text and GitHub archive asset digest agree. The checksum asset's
GitHub digest matches its bytes. Safe bounded unique relative paths, no links or
path escape, exact four-binary/123-browser-asset membership and every manifest
hash passed, including worker and guardian. Manifest SHA-256:
`a0f532fcf4d2c80fe4e2d1d62594df16b5e140a8fb652e9f74b5816bd03e2201`.

Durable [#353 checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5953597060)
and [#375 checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/375#issuecomment-5953596259)
retain source, quality, archive and limitation identities. No native installation
of this artifact was performed or inferred; CT106 and other native evidence remain
bound to their separately qualified source. This development prerelease is not a
stable release. #333, #353 and #375 remain open; full 100% reachable-production
coverage, measured scope and explicit deferred authority/platform limits are
unchanged.
