# Executable packages: source integration for #63 / #75

**Status: unverified source draft, not a delivered executable extension release.**
The #63 owner has not been admitted to a build slot. No compilation, executable
example, supervised journey, isolation test or coverage measurement is claimed.
The [SDK contract](extension-sdk.md) separates protocol implementation from the
required executing-Voyage adapters. See [architecture](architecture.md) for the
Helm → Vessel → independent Voyage boundary.

## Implemented source path

Format 1 retains its declarative UTF-8 archive, 64 KiB bound, `model_context`
capability and existing grants. Those grants never authorize an executable.
Format 2 is separate: at most 8 MiB JSON, one canonical-base64 static executable
of at most 6 MiB, SHA-256-bound manifest and artifact, protocol 1, compatibility
line `0.1`, platform `linux-x86_64`, runtime `static-elf`. Initial artifacts are
ELF64 little-endian x86_64 ET_EXEC without PT_INTERP or PT_DYNAMIC. The validator
checks bounded program tables, load ranges, entrypoint and non-W+X segments.
Validation is structural, not publisher trust or a successful execution test.
Additional resources, application dependencies, interpreters and runtime downloads
are unsupported. Host kernel/bubblewrap are trusted enforcement dependencies,
not downloaded package dependencies.

The manifest contains `format`, `id`, `version`, `voyage`, `protocol`, `platform`,
`runtime`, `entrypoint`, `contents` (one `{path,sha256}`), `capabilities` and
`definitions`. Capabilities are exactly `["execute"]` or
`["execute","host.file.read"]`. Definition schemas are bounded, validated offline
and cannot be changed by initialization. Unknown fields, duplicate content keys,
invalid hashes/base64 and unsupported capabilities fail validation. Definition
JSON preserves duplicate-key checks before a map can overwrite values.

Local pack accepts only declared regular files, with descriptor-relative,
no-follow traversal. Install and update read complete validated archive bytes;
they never extract or execute an installer. Pinned same-origin HTTPS acquisition
retains its existing no-redirect, no-proxy, finite timeout, index-integrity and
identity constraints. The index stays bounded to 64 KiB; a package download can
be up to the format-2 archive limit. Catalogs retain the 128-package count bound
and have a 16 MiB encoded-byte bound; a full catalog refuses new publication.
Project candidates shadow the same user package, including invalid/inactive
project candidates. No implicit fallback executes a different package.

## Review is not live activation

These are **source-defined commands to verify after a scheduled build**, not
commands run as evidence in this delivery:

```sh
helm extension pack ./package ./example.helmpkg
helm extension install ./example.helmpkg
helm extension inspect example
helm extension review-executable example --expected EXACT_ARCHIVE_SHA256 --capability execute
helm extension execution-grants
```

Use a separately reviewed `--capability host.file.read` after `execute` when the
manifest requests it. Review must name the exact capability list and archive
digest. Format-1 `enable` refuses format 2. Executable reviews live in a separate
private `execution-grants.json`, with format 2, and bind scope/location/package to
exact bytes. Repositories do not grant their own execution authority. Independent
orphan revocation is `helm extension revoke-execution BINDING --expected DIGEST`.
Revocation does not claim active work stopped.

`inspect.active` remains declarative activation; `execution_reviewed` means only
that exact executable bytes were reviewed. `pending_execution` is an observed
ledger count, or null when unavailable, never an inferred zero. Review does not
probe or start a process. The actual executing Voyage additionally requires
current policy, a supervised owner, non-read-only access, required isolation,
resource ownership and a collision-free live definition. Sandbox-off never
substitutes for isolation. Packages omitted from the real live registry are not
available merely because they have a review record.

Source namespaces are `ext_PACKAGE_TOOL` and `extcmd_PACKAGE_COMMAND`, with package
hyphens mapped to underscores. Combined names must fit 64 bytes. Colliding package
definitions are refused rather than overwriting built-ins or another package.
The existing `/tool NAME JSON` operator path uses structured arguments through
Vessel and Voyage; there is no shell-string interpolation or new Helm executor.
User commands are registry-backed namespaced definitions, not arbitrary new
slash aliases. Both operator and model calls retain ordinary runtime admission.

## Isolated invocation and host reads

Registry construction pins definitions without starting code. A lazy invocation
rechecks exact catalog bytes, project precedence and review under the grant lock,
and persists package/invocation/run/session/incarnation ownership before spawn.
It seals the verified ELF in a memfd; the required bubblewrap profile mounts those
exact sealed bytes read-only at `/extension`, without reopening a package path.
The profile omits ordinary system/runtime mounts, host roots, networking and
inherited environment. It uses the existing namespaces, seccomp, FD hygiene,
no-new-privileges, bounded tmpfs and resource limits. Additional SDK restrictions
refuse setsid/setpgid so descendants cannot leave the observed process session.
Limits remain per-process/same-UID, not aggregate descendant budgets. Native
macOS/Windows and other Linux architectures are unsupported.

The broker exposes only a separately requested UTF-8 file read. It rechecks current
execution authority and authorized canonical roots, opens every component without
following a replaced symlink, requires a bounded regular file, and checks authority
again before returning data. A delegated registry excluding `read_file` also
excludes the broker capability. No private ToolContext, provider credential,
workflow environment, human terminal input, shell, write or network capability is
sent to the extension. The initial broker requires offset zero and the complete
file to fit the requested bound, at most 64 KiB, then redacts the complete value.
It refuses partial/range reads rather than enabling reconstruction of secret
fragments across separately redacted ranges.

Progress is protocol-bounded and retained privately for the final typed result,
with a total of at most 64 KiB and the current output budget. Full output passes
the existing split-text/structured-content confidentiality checks and artifact
adapter. **Live progress presentation is not integrated.** Raw stderr goes to
null; refusal diagnostics are bounded authored strings, not child error bodies.

## Update, disable, removal and interruption

```sh
helm extension update example ./replacement.helmpkg --expected OLD_ARCHIVE_SHA256
helm extension disable example --expected CURRENT_ARCHIVE_SHA256
helm extension remove example --expected CURRENT_ARCHIVE_SHA256
```

Mutations serialize with executable admission. They durably revoke execution
review before checking drain state or replacing bytes. Pending work therefore
can produce an error **after review was revoked**, while old bytes remain
installed. Inspect before retrying; do not infer an atomic rollback or replay a
tool call. Updates do not inherit old review, even for the same package identity.
An existing invocation retains sealed old bytes. New invocations recheck grants;
there is no hot-swapped command path under a running call.

Owned child observers survive dropped callers and bounded cleanup waits. They
retain the original process-session identity until positive descendant observation
and child wait. A timed-out observation retains its original task; retries do not
start duplicate kills. Run resource management closes SDK admission, drains its
tasks and checks all retained children. Terminal results, cancellation requests,
EOF, released locks and stored PIDs are not cleanup proof.

Durable extension reservations join the existing host-resource session/incarnation
ledger. Existing fenced guardian recovery can resolve them only after observed
cleanup; boot-change evidence follows that same recovery path. `voyage host-resources
inspect` can show unresolved records. Manual host-resource attestations do not
release executable reservations. Cleanup observation does not prove the tool
succeeded or an external disclosure was rolled back. Unknown effects are never
replayed automatically. Corrupt review/ledger records are preserved and fail
closed rather than authorizing a replacement.

## Explicit remaining scope

- Nonempty lifecycle definitions are currently **refused at package validation**.
  The protocol recognizes run-start/run-finish, but real admitted run-boundary
  integration remains unfinished. No ignored subscriptions or ambient hooks.
- Live progress presentation, complete package-specific diagnostics and the full
  supervised adverse-state journey remain unfinished.
- Rust/Go compilation and tests, sealed-byte/isolation/no-effects evidence,
  held-pipe/descendant/crash/cancellation races, interrupted publication, and the
  joined real package workflow have not run. Focused Rust test source is not a pass.
- Merge current main, resolve overlap with other wave owners, then obtain an
  explicit #63 build slot. Run the required final workspace coverage, retain raw
  evidence and commit the measured `coverage/latest.json` with the verified
  implementation. Integration/publication requires the shared integration lock.
- Neither #63 nor #75 is closed by this draft. Unsupported platforms, live-provider
  certification and production changes are not implied.
