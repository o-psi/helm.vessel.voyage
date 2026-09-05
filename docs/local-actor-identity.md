# Local admission identity

`attachment::local_actor::LocalActorStore` is a storage prerequisite for #78.
The managed CLI and dedicated remote worker select their installation directory
explicitly and obtain its verified identity before local admission or recovery
attribution. Ordinary JSON chat/TUI sessions do not use this identity store.
Opening it neither transfers sessions nor alters Journal schema.

One installation data root has a dedicated private `local-actor/` directory.
Its version-1 record contains random installation and local-principal UUIDs.
All managed journals in that installation use the same pair; local admissions
map `installation_id` to `TurnAdmission.machine_id`. Command IDs remain unique
per intended command and stable on exact retries. Different data roots are
independent installations. Relocation preserves identity; copying an entire
installation copies its identity. The API has no current-directory fallback,
implicit rotation, import, reset, or enrollment dependency.

These IDs describe local attribution and deduplication, not authenticated
identity or authorization. They do not identify a Vessel operator or grant
sharing, tool, workspace, approval, or remote access. No hardware fingerprint,
OS account name, provider credential, or enrollment key is stored. Each
frontend must still enforce current policy and authorization at admission and
steering dispatch. The actor store does not make raw Journal callers trusted.

## Initialization and recovery

The caller provisions the parent directory; the API accepts an absolute,
dedicated child directory. Initialization uses a nonblocking exclusive lock.
Contention returns an error for the caller to retry within its own deadline;
it never waits indefinitely. The lock is released when initialization ends,
so idle handles and concurrent sessions can use the same installation identity.
Unix explicitly unlocks before closing, including when post-acquisition sync or
path validation fails. This avoids lock retention by an unrelated process fork
before close-on-exec; a regression retains a duplicate of the same open file
description while forcing the real path-validation failure.

Initialization writes and syncs a bounded `candidate.json`, then prepares a
bounded fixed publication slot and atomically publishes `actor.json` without
replacing an existing entry. Linux uses `renameat2(RENAME_NOREPLACE)`; macOS
uses `renameatx_np(RENAME_EXCL)`. Windows uses the private storage helper's
`MoveFileExW` without replacement and with write-through. The candidate remains
as recovery evidence. Before returning an identity, the API verifies the
published bytes and repeats the supported file/directory durability barriers.
Existing valid published identity bytes are never rewritten.

A failed operation returns no usable identity. Retry reuses a valid candidate
or observes a published identity; it never chooses replacement IDs for a
malformed record. Empty/partial candidate or publication files require explicit
operator recovery and are preserved. Conflicting candidates, unknown schema
versions, oversized records, and invalid UUIDs fail closed. The fixed slots
bound automatic crash debris to the lock, candidate, publication slot, and
published record. There is no automatic deletion of existing evidence.

Atomic rename and successful OS flushes are the supported durability boundary;
these do not promise survival of arbitrary filesystem, hardware, or power-loss
failures. Process-crash fixtures are not power-loss tests. On Windows directory
publication follows the verified local-NTFS helper contract; no POSIX directory
fsync guarantee is claimed. Unsupported filesystem/platform operations fail
rather than falling back to replacing publication.

## Private storage and verification

Unix creates owned 0700 directories and 0600 files, walks ancestors using
pinned directory descriptors without following symlinks, opens children with
`openat`/`O_NOFOLLOW`, and rejects linked, wrongly owned, or permissive files.
It checks metadata after opening and validates that the selected path still
refers to the pinned directory before/after publication and identity access.
Windows uses owner-only verified ACLs and retained native handles that pin the
directory hierarchy on supported local NTFS storage. Each child is checked on
open. A same-user process with unrestricted authority can still edit its own
private data; these checks are not an OS sandbox or cryptographic authentication.

Unit and actual subprocess fixtures cover restart, concurrent first use,
immutable publication, stale/missing identity, malformed records, bounded
contention, retained partial candidates, every outer initialization error/crash
boundary, and private-path substitution. An integration test reopens both actor
storage and Journal and observes the original command receipt after expiry
without adding a second user turn. It does not run a provider or tools. Windows
helper tests cover no-replacement publication and exact uncertain-candidate
reuse; native CI results must be observed separately from Linux results.
