# Native legacy fault arming source audit

Source reviewed: `182b188664bd150321758f1c6ed19838f952a1f1`.
This audit performs no native effects and does not qualify a death case.

## Two distinct installer execution routes

The maintained qualification driver `installer/tests/native_legacy_qualification.py`
`upgrade` stages the candidate and invokes `install.sh upgrade --no-start` with
`VOYAGE_RELEASE_DIR`. `install.sh` `launch_local` passes this exact argv:

```
candidate/bin/voyage-installer --bin-dir candidate/bin upgrade --no-start
```

`installer/src/main.rs` 135 calls `local_legacy_bootstrap`. In
`installer/src/remote/linux.rs` 701–702 this process holds the coordinator and
worker locks. It writes a fresh UUID record with `channel=local-owner`, pinned
legacy account namespace and supervisor activation, and
`HOME/.cache/voyage/upgrades/prepare-local-UUID` staging. Line 764 calls
`apply_legacy` directly. No apply user unit is launched on this route. The
installer is a child of the bootstrap shell, not the shell itself.

The separate remote `apply` entry at linux.rs 1363–1395 launches
`voyage-update-UUID-apply.service` via `launch` 315–341. Its executable is the
current installer with argv `remote-update worker apply UUID`. Its MainPID and
InvocationID can establish that role only when that distinct route actually
executes. Requiring such a unit for the maintained local-owner qualification
would miss the entire local execution and cannot establish native evidence.

## Existing monitor gaps

`kill_at` currently verifies only candidate executable path and UID, scans all
legacy records, and accepts only a helper source prefix/action. The helper pidfd
is closed immediately after the signal request. Neither returned result proves
exit/reaping. These observations are insufficient to identify execution role,
full helper program/namespace, one fresh operation or actual retirement.

## Proposed fixture-only correction

Preserve the maintained route and select the actual local-owner installer by its
exact argv above, qualified candidate SHA from staged release.json, UID/start/
image inode and an open pidfd. Baseline update record paths before the single
explicit upgrade. Accept exactly one new canonical UUID legacy local-owner
record; pin its immutable operation, previous/candidate release, staged bin,
state/account namespace and stage tuple. Refuse any changed/ambiguous context.
Do not signal the shell, a similarly named installer, an old record or a
replacement PID. A future independent-worker case must additionally pin its
actual named unit MainPID and InvocationID and exact remote worker argv; it must
be described as a different execution route.

At helper boundaries require a direct child of the retained execution pidfd,
UID/start, actual /usr/bin/python3 image/hash, full isolated `-I -c` program hash
matching reviewed `installer/src/legacy_update.py`, exact action and full
state/accounts/stage argv. Recheck direct parent, execution identity, record and
helper immediately before the single pidfd signal. Never pause a process to
manufacture a window, add production failpoints or alter receipts/security.

Persist pre-effect nonsecret witnesses, signal delivery, pidfd exit and positive
PID/start retirement separately for the installer and helper. Keep a bounded
1230-second observation deadline; missed helpers, permission denial, unreaped
zombies and expired windows remain unqualified. Preserve original operation and
all state for read-only `remote-update status UUID` observation; no effect replay.
Death observation alone is not rollback, lease cleanup, canonical preservation
or install acceptance. Those existing native acceptance checks remain required.

The parent accepted the local-owner role correction. The maintained fixture now
provides bounded `--auto-local-owner` arming and separate retirement observations;
this changes no production/Rust/Cargo/main/host behavior. Ten new offline contracts plus four existing monitor contracts passed under a bounded 1 GiB/no-swap user unit (186 ms, 20.3 MiB peak), including actual owned Python
children with full argv/parent/pidfd/signal/exit/reaping. They establish fixture
contracts, not native updater/helper death or rollback acceptance.
