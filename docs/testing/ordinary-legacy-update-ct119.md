# Offline ordinary legacy update qualification on CT119

Issue [#401](https://github.com/o-psi/helm.vessel.voyage/issues/401). This is an
executable qualification plan, **not a passing native result**. Parent delivery
owns artifact publication, controlled transfer, CT effects and qualification.
Do not reuse CT106 or production directories. Do not edit catalogue schema,
registration/guardian evidence or release manifests to make any case pass.

## Inputs and isolation

Use the disposable Debian 13.6 CT119 ordinary `qualification` account, UID1000,
with its real `/run/user/1000` D-Bus and user manager. Its HOME must be exactly
`/home/qualification`; default XDG directories must be in the manager UnitPath.
An unprivileged/nonnested CT is sufficient for ordinary user services if actual
manager execution is available; absence is an unknown native gate, not permission
to replace systemctl. Reserve 2 CPUs/3 GiB/no swap, retain the private evidence
within its 8 GiB volume. No network or provider budget is needed.

Transfer by the authorized host route, without changing the guest network:

- published v1.0.2 GNU x86-64 archive, SHA256
  `f7b078e654e514d23d4ab9021337837b90579babf9f79e21f2a01fdce4fcb4c8`;
- the qualified current nightly archive, its independent published SHA256 and
  exact BUILD.txt/source identity;
- current `install.sh` with an independently pinned SHA256;
- `installer/tests/native_legacy_qualification.py` and maintained
  `voyage/tests/delivery_recovery.py`, preserving their repository-relative paths.

The driver checks archive digests before extraction, rejects archive links/devices
and escaping paths, and records private evidence. It never removes prior installs
or automatically repeats a failed mutation. Start each adverse case from a fresh
CT or an offline host snapshot taken after seed and after all processes are
observed stopped. Taking/reverting such host snapshots requires the parent's
existing provisioning authority; this driver cannot perform them.

## Real seed and successful current bootstrap

Run these as qualification, substituting the transferred paths and independently
verified hashes. `--ack-disposable-ct119` makes the target explicit; it does not
skip namespace checks.

```sh
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 preflight
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 seed --old-archive /home/qualification/inputs/voyage-v1.0.2-x86_64-unknown-linux-gnu.tar.gz
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 upgrade --candidate-archive /home/qualification/inputs/current.tar.gz --candidate-sha256 PUBLISHED_SHA256 --install-sh /home/qualification/inputs/install.sh --install-sh-sha256 REVIEWED_SCRIPT_SHA256
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 observe
```

Seed invokes the actual old installer/service, creates two old supervised Voyages,
and completes exactly two fixed loopback synthetic provider responses. It observes
suspension, schema1/catalogue and journals ≤12, and retains canonical histories.
The current route is the normal public bootstrap's trusted `VOYAGE_RELEASE_DIR`
entry after controlled offline archive transfer, **not** the immutable old Vessel
UpdatePrepare/UpdateApply endpoint. Qualification of network acquisition is
separate. Successful upgrade must preserve the two canonical histories, journal
schemas, actual managed MainPID/executable/account namespace and clear quarantine
only after catalogue2 commit.

## Real activation failure and old reader

On a fresh seeded CT, run `fail-startup` in another qualification terminal **before
one** explicit upgrade invocation. The monitor waits for a real candidate MainPID
with catalogue2 while quarantine still exists, pins the PID handle and signals
only that exact candidate executable. At most five signals are allowed; it never
signals a previous release or a different UID. A missed startup window or an
exhausted bound is unqualified, not a passing failure case.

```sh
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 fail-startup
```

The upgrade command is expected to return a refusal/nonzero result. The monitor
requires actual previous executable/service readiness, schema1 restored, exact
canonical histories, no remaining quarantine, and two public Snapshot reads
through the actual old Voyage helper. Merely switching `current` is insufficient.
Retain the original durable update operation; do not retry it.

## Worker and owned helper death

On separate seeded cases arm `kill-at` against the **actual current installer PID**
(the driver pins UID/executable). `candidate-bin.json` is created before launch;
use `/proc` to identify the exact current installer, not a shell or guessed PID.

```sh
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 kill-at --installer-pid EXACT_PID --boundary snapshot
```

Boundaries: `snapshot` (pinned proof persisted), `restored` (positive restored
marker), `committing` (durable commit edge), and actual direct helper invocations
`helper-snapshot`, `helper-restore`, `helper-verify`. Helper matching pins direct
parent, UID and the embedded quiescent-helper program/action before signaling.
A requested signal is **not** observed retirement: retain `/proc` PID/start-time
and pidfd exit evidence, inspect every inherited ownership lease, verify no live
helper remains, then use current `voyage-installer remote-update status OP`
read-only reconciliation. Never call apply/prepare to recover unknown outcomes.
If a helper window is missed, retain the limitation and repeat only on another
fresh qualification namespace, not by replaying the same operation.

Acceptance: previous-pointer observation requires pinned schema1 restored marker,
exact namespaces/service/old reader; candidate-pointer observation requires
pinned unchanged proof, exact live candidate namespace/service and authenticated
endpoint. Missing/mismatched proof or a live helper retains fence and unconfirmed
receipt. The driver provides timing/signaling and observations; parent must retain
actual worker/helper retirement and reconciliation outcomes for each boundary.

## Live-owner and independently changed context matrix

These are additional native gates, not covered by the successful two-idle seed:

1. Hold one real legacy synthetic provider stream with a bounded loopback handler,
   submit once with exact ID/revision and wait for actual active owner. Invoke the
   current bootstrap once; require definite refusal, unchanged owner PID/run,
   zero cancellation commands and no snapshot/quarantine/publication. Release the
   provider response and observe canonical completion/cleanup. Do not cancel to
   make the upgrade eligible.
2. While updater is stopped with its owned pidfd after snapshot, alter only the
   disposable manager's XDG_DATA_HOME, then continue. Candidate or old restart
   using the changed namespace must not clear quarantine or claim rollback.
   Record exact original manager value and restore it only after retaining refusal
   evidence; this is external context restoration, not an updater retry.
3. Independently alter the disposable reviewed unit bytes/enablement after review.
   Keep original bytes/state pinned. Require refusal to overwrite that change and
   no successful reconciliation based merely on executable identity. Runtime-only
   enablement must never become persistent enablement implicitly.

The driver does not synthesize these authority mutations or implement privileged
snapshot management. Parent must supply and observe these real fixture boundaries;
leave the corresponding rows unknown until then. Preserve all failed state and
private evidence. Cleanup stops only positively matched qualification resources
and observes PID/helper/guardian exit; it never deletes CT106 data or receipts.

The driver implements the bounded real live-owner variant as `live-owner`, with
all four archive/bootstrap arguments shown for `upgrade`. Run on a separately
seeded CT. It creates one additional explicit held owner, requires exactly one
provider request, unchanged owner/run/service/pointer, absent mutation receipts
and quarantine, then releases that original response and observes completion.
No cancel command is sent. This additional owner's history is private evidence,
not substituted into the two-idle upgrade comparison.

The independent context controls are executable via `change-at --installer-pid
EXACT_PID --context account-namespace|unit|enablement`. They wait for a durable
snapshot, stop the pinned updater, change only the dedicated account's real
manager environment, reviewed unit comment or enablement, record original values
privately, and resume that exact updater. They do not restore those external
changes or replay the update. Native outcome still requires parent observation
of quarantine, receipt, actual service PID/namespace and unchanged operator state.
For enablement, seed must actually be enabled: changing disabled to disabled would
not create independent-change evidence. Snapshot windows can be missed and must
remain unqualified. No source test, simulation or signal request supplies that
missing native observation.

A killed local updater can leave `applying`/`committing` until the implemented
1,230-second receipt deadline. Status must honor that bound: do not edit receipt
phase/timestamps, shorten the timeout or infer reconciliation from a signal.
Retain pending evidence until the actual status path qualifies it.
