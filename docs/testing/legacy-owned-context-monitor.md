# Exact ordinary legacy context monitor

This source preparation repairs the `change-at` fixture selector recorded in
[#401](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5946876839).
It does not establish a native context refusal or rollback result.

Arm one monitor in a fresh disposable, seeded ordinary user namespace before
**one** qualified current bootstrap upgrade. Use the actual maintained
`local-owner` installer route, not a shell PID or independent remote UpdateApply
worker. For each case use a separate fresh fixture; retain all previous attempts.

```sh
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 change-at --auto-local-owner --context account-namespace
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 change-at --auto-local-owner --context unit
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 change-at --auto-local-owner --context enablement
```

Explicit `--installer-pid PID` remains available with the same strict admission.
Neither selector relaxes executable/hash/argv/UID/start/pidfd or fresh operation
binding. The monitor arms before staging appears, excludes all baseline receipts,
requires one new canonical `local-owner` record and binds its actual qualified
installer, exact `upgrade --no-start` argv, staged manifest, account namespace,
activation and snapshot proof. It records an exclusive attempt before any scan.

The effect window requires `applying`, an old managed pointer/catalogue schema1,
private single-link snapshot/backup/quarantine files, exact retained proof evidence
and backup hash, and a quarantine naming that exact previous/target operation.
The manager's actual HOME/XDG presence and values, original unit inode/content and
persistent **enabled** state remain pinned. An absent XDG_DATA_HOME is recorded
as absent, not replaced with a guessed old value. Namespace review refuses another
operator context. A runtime enablement state is not treated as persistent enabled.

Only the retained exact updater receives one pidfd SIGSTOP. The monitor observes
that same PID/start in kernel `T`/`t`, checks every updater thread's direct child
inventory is empty (including unreaped helper children), and rereads the exact
record/snapshot/context after stopping. A missed, busy, advancing or changed
window remains unqualified and does not mutate context. It never pauses a
replacement PID. Snapshot equality and owner identity are rechecked after durable
pre-effect evidence, before mutation.

The monitor performs one of these actual context changes:

- set manager XDG_DATA_HOME to a new private fixture directory;
- append one comment to the exact reviewed unit inode, save the full private
  original unit bytes, and reload that manager;
- disable a persistently enabled unit and positively observe `disabled`.

It retains the true prior values and observed changed values. Finally it sends at
most one pidfd SIGCONT to that same verified owner and positively observes it
leave the stopped state. Missing, replaced or inaccessible owner identity is an
explicit continuation obligation; no replacement, repeated signal or forced
service action is attempted. An unconfirmed context command remains unconfirmed.
No context restoration, upgrade replay, reconciliation, service start or recovery
is performed by this monitor.

`context-CASE-*` receipts separately record requested stop/change/continue and
observed results. `context_changed_and_owner_continued` attests this bounded fault
injection only. It **does not** prove updater refusal, canonical preservation,
rollback, quarantine retention, full cleanup or old-reader readiness. Observe those
independently against the retained operation and original fixture baseline.
A changed namespace/unit/enablement must not be overwritten or promoted to a
successful rollback by the updater. Restore the operator context only as a later
explicit operation after preserving the native evidence; never retry that upgrade.

The original broad `kill_at` context implementation is retired. Fault delivery at
other death boundaries continues through `native_legacy_faults.py`. Complete raw
canonical histories (including provider_state), private ownership locks and
supported status/recovery outcomes retain the gates in
[ordinary legacy qualification](ordinary-legacy-update-ct119.md).

## Coordinated source gate

Fifteen private Python contract parents cover real stored namespace presence, strict
namespace refusals, actual owned directory/unit changes, no-op enablement refusal,
private-file and exact-start boundaries, and exclusive admission/result behavior.
They use a fake manager/UID projection and no native updater or actual signal.
They are prepared, **not executed**. AST/diff/path checks precede the parent's one
coordinated gate; Python checks do not contribute to Rust coverage or native proof.

```sh
/usr/bin/python3 -I -B -c 'import pathlib,sys,unittest; sys.path.insert(0,str(pathlib.Path("installer/tests").resolve(strict=True))); unittest.main(module="native_legacy_contexts_tests")'
```

Run from the pinned workspace root under the coordinated bounded unit. The
explicit local import bootstrap is required by isolated Python; no ambient
site, human credentials, manager or native signal is used by these contracts.
