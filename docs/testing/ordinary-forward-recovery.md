# Owner-reviewed forward recovery of a proved mixed user installation

Scope: [#401](https://github.com/o-psi/helm.vessel.voyage/issues/401). These are
prepared source boundaries; full coverage/build/native qualification is pending.
This is ordinary Linux user installation only. System/Root scope is separate.

## Why this entry exists

An old updater may leave managed metadata pointing at v1.0.2 while an approved
nightly supervisor/gateway run and catalogue has already migrated to schema2.
Without an original schema1 snapshot, ordinary legacy rollback cannot recover
that database. Never edit the schema version, fabricate a backup, discard Voyages
or replay the uncertain old apply. The new `recover-user` entry independently
reviews and repairs forward identities, retaining the original receipt byte for
byte. It does not convert that uncertain receipt into a claimed successful update.

## Supported source boundary

Prepare requires one original `unconfirmed` receipt whose original metadata is the
actual managed v1.0.2 identity and whose reviewed candidate is the exact running
release. Both complete installed archives are verified. The current supervisor
must expose an authenticated ready endpoint, exact owned unit, directory, actual
kernel executable and account namespace. The one explicit ordinary gateway must
be the same running release, exact owned fragment with no behavior overrides, unchanged
enablement, canonical approved HTTPS origin and literal loopback bind. Recovery
supports only the temporary immutable gateway command (quoted or plain executable,
optional systemd `:` prefix), with either the known positional origin at the end
or the canonical `--public-origin` flag. Other escaping/unknown command forms
refuse rather than guess. Curl is needed for bounded local gateway health reads.

Every retained ordinary owner must have observed cleanup, idle ownership locks,
no active run, matching complete registration projection and journal≤20. Catalogue
must already be schema2; execution identity/binding/admin authority tables must be
empty. Private account and canonical data are pinned. Active owners, peer scope,
unknown cleanup and independently changed context refuse without cancellation.
The target is a separately qualified full archive with pinned manifest/declaration
and formats readable by the actual running source. Nothing trusts a latest label.

## Exact local owner review

Invoke the **qualified current** standalone installer, not the old installed
updater, against the same ordinary HOME/XDG namespace as its live services:

```sh
/path/to/qualified/bin/voyage-installer recover-user prepare NEW_RECOVERY_UUID --original-operation ORIGINAL_UUID --running-release EXACT_RUNNING_RELEASE_SHA256 --bin-dir /path/to/qualified/bin --gateway-unit EXACT_GATEWAY.service --public-origin https://approved-origin.example
/path/to/qualified/bin/voyage-installer recover-user apply NEW_RECOVERY_UUID --review EXACT_PREPARE_REVIEW_SHA256
/path/to/qualified/bin/voyage-installer recover-user status NEW_RECOVERY_UUID
```

Prepare stages/verifies the target privately and returns the operation, original
receipt, original metadata, actual running source, target and gateway identities
plus a review hash. Inspect the private `recoveries/NEW_RECOVERY_UUID.json` for the
complete reviewed units, origin, source/account/directory/evidence before apply.
Apply requires that exact hash and permits only the `reviewed` phase. The new UUID
must differ from the original. Repeated apply never performs an effect, even with
the same hash. No new provider credential, authority right or budget is inferred.

Apply holds ordinary installation/coordinator/worker locks, persists a separate
operation and quarantine, stops only reviewed services, holds every session's
startup/guardian/execution leases and pins an existing-schema2 snapshot/proof.
This snapshot is evidence only: forward code cannot restore it. It first aligns
managed metadata with the **already approved actual running source**, then
publishes the separately approved target; legacy binaries never become rollback
readers. It starts the quarantined target and atomically changes only the reviewed
gateway executable to the managed persistent `current/bin/vessel` path, restoring
`--public-origin` with the exact original HTTPS value. All other unit bytes and
credentials are retained. Enablement is observed/preserved, not changed.

Completion requires actual target pointer/archive, authenticated supervisor,
exact target unit/namespace/account, exact repaired gateway/argv/enablement,
loopback gateway health/source version and unchanged canonical/private proof.
All session leases remain held through final observations and quarantine removal.
The original receipt is hash checked again. Only then is the new recovery complete.
A completed recovery with pinned original identities, retained snapshot and target
allows a future independent update without changing/replaying the old receipt.
Missing or tampered recovery proof cannot bypass the old pending-operation gate.

## Interruption and limits

Failure retains both releases, snapshot, quarantine and separate unconfirmed
recovery. It does not attempt an automatic rollback or restore the already
unreadable old metadata. `status` performs bounded observation only. A drained
worker and reacquired exact session leases may complete observation of an already
published/ready approved target; it never installs, starts, restores or replays.
A live helper retaining leases prevents reconciliation. A previous/source pointer,
missing proof or changed unit/account/canonical data remains unconfirmed/fenced.
Such states need another specifically reviewed recovery mechanism; this entry
must not be presented as universal corruption repair.

Native CT106 forward qualification remains required before issue closure. Its
original operation `2a84c5a0-bf77-4fa0-9fe5-89a8c5207463` stays unknown; retained
schema2 and both completed Voyages must survive, repaired managed identities and
actual services must match the new qualified archive, and the unchanged approved
public origin must expose healthy authenticated Helm connection behavior. Do not
publish private local paths, tokens, receipt payloads or terminal input in evidence.

## Idle notification layout between review and quiescence

The live source courier opens its schema1 notification store even with no
subscriptions. Its idempotent SQLite transaction can change the main-file header
and cold PERSIST-journal bytes without changing any notification record. The
prepared review retains the original raw `state_sha256` and adds an explicit
`review_state_sha256`; the approval hash and receipt are never rewritten.

Before quiescence, only this known notification pair can use semantic equality.
The read-only projection validates both private owned regular files, the supported
schema1 metadata and full SQL table/index/trigger definitions, full column metadata,
every typed row/row identity, the exact clock row and SQLite sequence. Unknown
schema objects, changed logical data or clock, WAL/SHM, hot/nonzero journal header,
missing pair, symlinks/hardlinks/unowned/nonprivate files and bound overflow refuse.
Every other file still contributes its raw bytes; both notification files still
contribute their names, existence and privacy/size accounting. An inactive PERSIST
journal tail has no replay authority; its raw bytes remain in the original review.

After the source stops and ownership locks are held, the installer requires the
same complete semantic review and every other source/account/registration claim,
then pins a new **raw** snapshot. Target activation, final proof and reconciliation
require every raw byte of that held snapshot to remain unchanged. Even an idle
notification header write after quiescence fails the held proof. This supports
known live-source physical drift; it does not relax the pending activation proof
or silently discard notification state. Old reviews lacking full semantic evidence
refuse and need a separate fresh reviewed operation, never an altered approval or
replayed unknown mutation. Native qualification of this boundary remains pending.

## Approved running source without an older format declaration

A known mixed installation can have an approved, authenticated schema2 running
candidate published before `update_compatibility` was added. Actual CT106 source76
has this shape: old v1.0.2 managed metadata, actual catalogue2 and two observed
journal12 conversations. Its immutable manifest remains unchanged. Ordinary
update/rollback still refuses absent previous-reader declarations.

The separate forward-only path records the actual catalogue schema, every unique
journal schema found across the complete session inventory and registration process
protocols in its review and held evidence. For an undeclared running source it
requires the target installer hash to equal the executing recovery binary and the
target's entire declared format vectors to equal this recovery binary's embedded
source-derived current catalogue/journal/protocol contract. Every observed schema
must be admitted by those target readers. Unknown, changed, duplicate, missing or
malformed formats, different installer bytes or a changed target contract refuse.
The installer derives these values from its compiled source inputs; it does not
invent a prior release's readers or modify the old manifest.

Actual ordinary schema2 and raw canonical/account/state/session ownership proof
remain required. The global current catalogue declaration also includes separately
gated bound-layout formats; this ordinary operation does not admit that layout.
Poststart verification requires the same ordinary schema2 and every held raw claim.
Recovery never calls journal upgrade or database restore and never promises binary
rollback to the undeclared source. Future explicit runtime work remains subject to
its own normal format/authority checks. Declared sources retain the existing
rollback-format guard without this exception.

This is prepared source, not a successful native recovery. The refusal from operation
`6183c0a9-97d9-48c7-baa6-e538029ccda7` cannot be turned into an approved review by
editing its files. After focused/strict/full source qualification and an actual new
hosted artifact, use a new distinct owner review from current facts, preserving the
original uncertain operation and all prior private evidence.

## Existing credential-only gateway override

Actual fresh prepare `51f287d7-01c7-439d-a45e-02f1d6c951af` refused the existing
credential drop-in before returning a review hash; no apply occurred. Do not
remove or rewrite that private credential configuration to make recovery pass.

The prepared fix accepts zero overrides or one private owned regular credential
file in the exact gateway unit's drop-in directory. It reuses the maintained
supervisor allowlist: a single `[Service]` section and a single literal
`VOYAGE_CREDENTIAL_KEY_FILE` environment assignment with a normalized absolute path.
No service behavior directive, environment-file expansion, additional assignment,
multiple override, symlink, hardlink, unowned or nonprivate file is admitted.
The review pins path, device/inode, owner/mode, bytes hash and effective environment;
live gateway and recovery installer credential paths must match that assignment.
Apply and poststart observation require the same pinned override and context.
Replacing a file with identical bytes changes its inode and refuses the review.
The credential bytes are never read, copied or published by this admission.
Existing unit, drop-in, permissions and credentials are preserved through repair.

This source still requires focused/strict/full measurement, a new hosted archive
and a new distinct native prepare/apply. The failed operation remains preserved.
