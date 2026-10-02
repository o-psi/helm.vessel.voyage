# Exact old public history qualification

This fixture correction is source-bound to published `v1.0.2`, commit
`bba5f4b1d8d7561d30030c90a4244deb73e27702`. The checksum-verified old archive
has SHA-256 `f7b078e654e514d23d4ab9021337837b90579babf9f79e21f2a01fdce4fcb4c8`.
The public old Snapshot is not the raw JSON persisted in its journal.

`voyage/src/attachment/runtime/process.rs` uses `projection::recent`.
`runtime/process/projection.rs` `full` exposes thirteen explicit public fields,
including null option/default empty array values; `bounded` also adds exact
`message_index` and `projection_truncated`. `model.rs` canonical serialization
omits absent optional/empty fields and persists private `provider_state`, which
public projection intentionally excludes. A raw-object equality assertion is
therefore structurally incorrect even when every canonical text byte is retained.

The new pure verifier checks **all thirteen public fields**, exact complete text,
role, order, timestamps, coordination/tool/interruption/steering identities and
all content/tool references. It compares canonical JSON bytes, retaining numeric
and boolean type distinctions. It requires `public-v1`, exact session identity,
the exact `observation=snapshot` marker, integer journal revision, zero offset,
complete total and no truncation. Unknown or omitted public fields,
changed text/order/identities and partial history refuse. It does not silently
fall back to counts, previews, new commands or another reader.

Complete raw DB before/after equality remains a separate mandatory gate,
including private provider replay state. A public reader cannot attest private
metadata it deliberately does not expose. The verifier returns only safe count/
identity/hash metadata after full payload equality; it never publishes text.

## Separate read-only acceptance checkpoint

The original failed monitor output is evidence and must not be overwritten.
After source approval/publication, transfer `native_legacy_history.py` beside the
maintained qualification driver. A fresh exclusive read-only attempt/result uses
one of these exact cases in the same disposable qualification namespace:

```sh
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 qualify-restored-history --case startup
python3 installer/tests/native_legacy_qualification.py --ack-disposable-ct119 qualify-restored-history --case helper-snapshot
```

This command **does not upgrade, signal, restart, stop, inject or replay** an
operation. It observes restored original service image/unit/enablement, schema1,
no quarantine and complete raw histories. It requires the original recorded
candidate or owned updater/helper PID/start witnesses to be positively retired;
an unreaped zombie remains an obligation, and a reused PID is never signalled.
It binds the restored supervisor PID/start before each read interval and rechecks
it afterward. Around actual Snapshot reads it samples only that supervisor’s
direct children across its bounded thread inventory: exact qualified old Voyage
image, `observe-suspended --directory` and original fixture session paths. Each Snapshot read has a separate session-bound interval and must capture its
own distinct helper identity; one sampled helper cannot qualify a second missed
read. Open pidfds bind every captured helper PID/start; positive exit/reaping and unchanged
baseline/end child identities are observed separately. A missed transient helper
is **cleanup unknown**, never a fabricated witness or unrelated global absence.
No signal is sent. The helper watcher and descriptor lifecycle are bounded.

It reads each actual old Snapshot once, verifies the full public projection and
rechecks raw histories/service/retirement afterward. New private
`readonly-restored-CASE-attempt.json`, `-qualified.json` (or
`-history-verified.json` when helper cleanup is unknown) and `-result.json` preserve
the original monitor/fault evidence and distinguish failed versus qualified reads.
The original proof/backup/restored marker receipts remain independently required
where that fault boundary produced them; this reader checkpoint does not invent
a missing backup or turn another unconfirmed operation into success.

The helper-snapshot case is the actual local-owner updater and its direct helper,
not the separate remote UpdateApply worker. These read-only checks cannot qualify
that different route, native browser/public TLS, external services or another
platform. Incomplete quarantine/ownership/service/history conditions remain
unqualified. Original signal witnesses bind the exact fresh operation and qualified target;
helper records bind the exact parent PID/start, full pre-signal source/program
identity and delivery. Helper premutation absence of legacy proof remains
explicit, with no invented backup requirement. No current production, authority,
rollback or coverage exclusion is changed by this fixture correction.

This preparation contains fourteen pure projection/readonly-orchestration/helper
cleanup test parents. AST/diff checks are complete; tests have **not run**. They
await the parent’s coordinated gate with the browser/frontdoor source cohorts.
