# Executable packages (Linux x86_64)

Format-2 packages execute only inside the independent Voyage runtime under the
required Linux isolation profile. The [SDK contract](extension-sdk.md) separates
protocol behavior from package authority and resource ownership. See
[architecture](architecture.md) for Helm → Vessel → Voyage boundaries and the
[verification report](test-executable-packages.md) for observed checks and limits.
This is not publisher certification, native macOS/Windows support or production
readiness for arbitrary extension code.

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

Use these commands on the executing host; inspection and review do not launch code:

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
excludes the broker capability. Private account/session/configuration stores are
refused even beneath a broadly allowed host root. Actual loaded configuration
sources retain deny-only pathname and file-identity provenance in private launch
settings across resume/reconfiguration; hard-linked host files are refused. Legacy
settings without source provenance cannot activate the host-read capability until
a fresh configuration load establishes that boundary. This never widens roots. No private ToolContext, provider credential,
workflow environment, human terminal input, shell, write or network capability is
sent to the extension. The initial broker requires offset zero and the complete
file to fit the requested bound, at most 64 KiB, then redacts the complete value.
It refuses partial/range reads rather than enabling reconstruction of secret
fragments across separately redacted ranges.

Progress is protocol-bounded and retained privately for the final typed result,
with a total of at most 64 KiB and the current output budget. Decoded progress/result/input leaves are checked together before JSON punctuation
can hide split configured-secret fragments. Full output additionally passes the
existing split-text/structured-content confidentiality checks and artifact adapter.
Post-protocol confidentiality/budget/artifact failure quarantines the package and
records a failed result instead of claiming protocol success was accepted output. **Live progress presentation is not integrated.** Raw stderr goes to
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
start duplicate kills. Blocking host-read workers remain owned by their invocation
and must drain before its reservation can be released or bytes replaced. Positive
child observation is retained across a failed ledger write, avoiding unsafe PID
recapture and permitting a later persistence retry. Run resource management closes SDK admission, drains its
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

## Admitted lifecycle events

The source integration dispatches optional `run_start` and `run_finish` handlers
at actual Agent run boundaries, including operator runs. Start occurs after input
checkpointing; finish occurs before successful acceptance, with steering closed.
Cancellation, failed main work and crashes do not require an effectful finish
handler. Metadata is only `{run,event}`, not conversation/provider state. These
handlers are separate from advertised model tools and cannot be invoked by a
fabricated tool name. Registry filtering also filters delegated handlers.

The entire set at either boundary is bounded by the current command timeout and
120 seconds, including approvals. Every handler uses current execution authority,
exact package review, isolated spawn and the same retained cleanup manager. A
failed handler quarantines its package instance, not unrelated packages or the
main voyage. Fixed response outcomes and unknown interrupted outcomes are retained
in the package ledger separately from cleanup. `helm extension execution-status
BINDING` reads the latest 32 records without child diagnostics or private paths.
Repeated admission of the same package/digest/run/lifecycle event is refused;
restart never replays a previously admitted handler. No model-authored messages
are fabricated to represent runtime lifecycle effects.

## Limits and remaining SDK scope

See the [verification report](test-executable-packages.md) for actual commands,
results, retained failures and the distinction between coverage and correctness.

- Live progress presentation remains unfinished under #75. Bounded progress is
  currently retained in the checked final result, not streamed into the TUI.
- Native macOS/Windows, alternate ELF ABIs, dynamic linking, extra package resources,
  host writes/network/secrets and publisher trust are not supported by this version.
- The repository contains the standalone SDK and examples; this does not silently
  add an SDK distribution to the legacy runtime-release archive scripts.
- SDK-wide authoring/support and the remaining UI/adversarial matrix are tracked
  separately under #75. No human acceptance/testing gate substitutes for missing
  evidence. Required isolation unavailable always refuses launch.
