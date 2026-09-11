# Executable extension SDK, protocol 1

Status: **bounded source implementation for #63/#75, not a verified executable
release**. The #63 build slot is still unadmitted after #213's release. No compilation, tests, coverage,
static linking or supervised isolation journey has been established for these
files. Parent integration owns package admission, sandbox/resource adapters,
module and registry wiring, command routing and admitted lifecycle dispatch.
This dependency alone does not resolve either issue. See [architecture](architecture.md)
and [current state](current-state.md) for the runtime boundary.

## Distribution and trust

The accepted separate format-2 package is canonical-base64 JSON, at most 8 MiB,
with integrity-pinned static Linux x86_64 ELF contents. Initial SDK examples need
one executable each, no runtime dependencies or resources. The package parent
validates ELF tables, rejects interpreters/dynamic dependencies, and pins file,
manifest and capability digests. Format-1 guidance grants do not authorize code.
Discovery, inspection, installation, SDK construction and registry population
must not start extensions. Each admitted invocation launches lazily.

Execution requires the parent's empty-runtime Linux bubblewrap profile from
sealed immutable files. It must provide no host system runtime mounts, workspace
mounts, inherited environment, networking, provider credentials or PTYs. There
is no sandbox-off, configured-MCP or native-subprocess fallback. Other platforms
are unsupported by this initial contract. SDK source portability is not native
execution/security evidence. Process policy is not itself an OS sandbox.

## Wire contract

[`protocol.schema.json`](../sdk/extension-v1/protocol.schema.json) describes fields;
Rust `extension_sdk/wire.rs` and `transport.rs` enforce framing and state limits.
Private stdin/stdout carries one UTF-8 JSON object per LF-terminated line. Never
print logs on stdout. CR, empty frames, duplicate object keys, trailing data,
unknown fields/variants and partial-frame EOF fail closed. JSON schemas describe
shape, not authority, deadlines or all byte/state limits. JSON Schema maxLength
counts characters; all limits below additionally count **UTF-8 bytes**.

| Bound | Protocol 1 |
| --- | --- |
| Frame | 1 MiB including LF; bounded before allocation growth |
| JSON tree | depth 48 (root 0), 4096 value nodes per complete envelope |
| Each input/output schema | 256 KiB; offline bounded validator |
| Aggregate definitions | 512 KiB, also subject to JSON tree bound |
| Definitions | 32 tools, 16 commands, 2 lifecycle handlers |
| Active/queued | one active per package executor, at most eight waiting |
| Progress | 4096 bytes, 10 per rolling second, 256 per call |
| Host reads | 32 per call, one outstanding, IDs strictly 1 through 32 |
| Read path/data | path 4096 bytes; data 1–65536 bytes, UTF-8 only |
| Initialization | 5 seconds including initial write |
| Invocation | absolute runtime deadline, narrowed to at most 120 seconds |
| Cancel write / shutdown / cleanup | 100 ms / 1 second / 5 seconds |

Invocation deadline includes queue, launch, initialization, approvals and broker
backpressure. Shutdown and cleanup have independent bounded allowances after it.
A full queue refuses immediately; cancelled/expired queued work never launches.
Definitions near the envelope limit must leave room for initialization identity.

### Initialization and definitions

Host sends `initialize` with `protocol:1`, identity, definitions and capabilities.
Extension replies `initialized` echoing the exact identity, supported pinned
definitions and capabilities. It must compare against its compiled-in contract,
not merely accept an arbitrary host manifest. No protocol negotiation fallback.
Identity is `{session,incarnation,run,invocation,package,digest}` with UUIDs for the first
four and exact archive SHA-256 for the digest. Only the executing runtime may
choose/admit it. A matching echo proves compatibility, **not authorization**.

Definitions require all three arrays: `tools`, `commands`, `lifecycle`. Each entry
has `name`, `description`, `input_schema`, `output_schema`. Names match
`[a-z][a-z0-9_]{0,47}` and are unique across all three arrays. Description is at
most 4096 bytes without controls. Lifecycle names are only `run_start` and
`run_finish`. Schemas reuse Voyage's existing offline `CompiledSchema`, including
reference restrictions and bounded instance validation. No remote retrieval.
Capabilities are exactly `["execute"]` or `["execute","host.file.read"]`, ordered
and without duplicates; all other capabilities fail before launch.

The runtime registers collision-checked namespaced names. Wire invocation uses
the pinned local name and kind. Commands are explicit structured operator calls,
never built-in overrides, slash-string interpolation or shell text. Parent owns
the final external namespace spelling and routing, not this transport.
Lifecycle entries do nothing on discovery or executor construction. Parent must
admit and invoke them at real run boundaries; cancelled/crashed runs do not need
an effectful finish handler. No ambient hook dispatch is implemented here. The
current format-2 package integration explicitly refuses nonempty lifecycle arrays
until that integration exists; it does not silently ignore subscriptions.

### Invocation and replies

Host sends `invoke {invocation,kind,name,arguments}` after initialization. Input
and returned `result.value` must match the pinned schemas. A reply is one of:

- `progress {invocation,text}`: bounded/rate-limited untrusted text; parent broker
  must redact and sanitize before presentation.
- `result {invocation,value}`: exactly one structured JSON result. Parent adapts
  it into existing typed tool results and applies output redaction. No protocol-1
  binary/artifact/provider-output escape hatch is introduced.
- `error {invocation,code}`: `invalid_input`, `failed`, or `cancelled`. Arbitrary
  subprocess error text, stderr, paths and provider state are never diagnostics.
- `host.file.read {invocation,request,path,offset,max_bytes}`: only with the
  declared read capability. Offset is an unsigned 64-bit **byte** offset.

The host broker rechecks invoking roots, current access/approval/delegation
ceilings and redaction. Its result is `host_result {invocation,request,text}` or
`host_error {invocation,request,code:"denied"}`; errors never include private
reasons or paths. Reads are neither arbitrary filesystem access nor a permission
granted by package activation alone. Broker must refuse invalid UTF-8 or offsets
that split codepoints rather than lossy-decode; never read unbounded data first. The initial integrated broker accepts only
offset zero and a complete file fitting the requested bound (at most 64 KiB).
Nonzero offsets and partial files are refused so multiple range reads cannot
reassemble redacted secret fragments. There is no write/network/shell/
secret/PTY capability and no local fallback. Broker approval futures must be
cancellation-safe. Any bytes from the child while a host read is outstanding
fail the call: the child waits for that reply, including withholding progress.

All replies correlate to the active invocation. Duplicate, wrong, late or
out-of-state messages, floods and invalid results fail closed and quarantine the
executor. On failure/cancellation host attempts `cancel {invocation}` for at most
100 ms; no cancel acknowledgment is defined. It then sends `shutdown`, expects
`shutdown_ack` and EOF within one second, closes pipes and asks the lease for
positive cleanup observation within five seconds. A terminal reply racing cancel
may fail closed; do not retry effects to manufacture a clean result.

## Rust integration contract

`voyage/src/extension_sdk.rs` exposes crate-local interfaces:

- `validate_definitions(&Value, &[String]) -> anyhow::Result<()>` for packaging.
- `Definitions::parse`, `Definitions::find`, `Definition`, `Kind` for pinned
  registry construction, never dynamic child-defined registration.
- `Executor::new(Value, Vec<String>, Arc<dyn LaunchAdapter>) -> Result<Arc<Executor>>`.
  Keep exactly one executor per admitted package snapshot to enforce queue bounds.
- `Executor::start(&Arc<Self>, Invocation, Arc<dyn Host>) -> Result<InvocationHandle>`.
  Invocation supplies identity, kind, name, arguments, absolute Tokio deadline,
  and a runtime cancellation token. `completion()` returns `Completion` containing
  `Result<Value>` and independent `Cleanup::{Observed,Pending}`.
- `LaunchAdapter::launch(&Identity, Instant) -> Result<Launched>` must atomically
  revalidate the exact catalog grant and persist package/invocation/resource intent
  **before effects**. Return boxed private async reader/writer plus `Box<dyn Lease>`.
  Launch timeout/drop/error may have effects: adapter must own cancellation-safe
  tracking and recovery even if no lease is returned. Failure reports pending,
  not imaginary successful cleanup.
- `Lease::terminate_and_observe() -> Cleanup` owns child/process-tree resources.
  Timeout/drop/panic must retain durable unresolved state and guardian recovery.
  Persist cleanup disposition even if the result receiver no longer exists.
- `Host::read(&Identity,&str,u64,usize,Instant) -> Result<String>` and
  `Host::progress(&Identity,&str) -> Result<()>` are narrowed broker callbacks.
  No raw ToolContext is sent to or made available to the child.
- Revoke catalog grants **before** `Executor::close()`. Closing requests cancellation
  and closes admission; it does not release reservations or prove cleanup.
  `Executor::shutdown().await -> Cleanup` closes and waits at most seven seconds
  for all detached tasks/queue permits, returning Pending if any completion,
  panic or child cleanup is unresolved. Parent must retain child observers when
  lease observation times out. Lazy process instance identity equals invocation.

Caller drop cancels its call but does not abort the detached supervising task.
That task retains the admission permit, child I/O and cleanup lease through bounded
shutdown/observation. A failed call or pending cleanup closes the executor and
cancels queued work. Parent must retain ledger blockers, fence update/remove and
use existing incarnation-bound guardian recovery; neither dropped locks, direct
child exit, EOF nor acknowledgments establish descendant cleanup. Runtime crash
is not task survival: retain uncertainty and never replay automatically. Adapter
panics or process death require the same durable recovery. SDK contains no raw
`Command::spawn` path and cannot independently establish those guarantees.

## Authoring and compatibility

The [Go reference SDK](../sdk/extension-v1/README.md) uses only the standard
library. Examples supply compiled-in definitions and cover pure transformation
and separately authorized bounded host read. Run one invocation per process;
exit after Serve returns, so stdout closes for the shutdown handshake. Cooperate
with cancellation, avoid diagnostics containing private inputs, and treat all
host errors as bounded refusals. Host may terminate even an uncooperative handler.

Protocol 1 is the sole initial version. There is no historical SDK compatibility
claim. Unsupported versions/capabilities fail closed; they must not crash Helm or
block unrelated packages. An incompatible future wire or capability change needs
an explicit new protocol and documented migration, not silent reinterpretation.
No deprecation window is promised for this unpublished, unverified source draft.

## Remaining verification/integration

Module-local focused tests cover strict JSON/framing/definitions and in-memory
handshake/result/cleanup disposition. They have **not been run** during the pause.
They are not OS isolation evidence. Parent must wire the module and adapters,
validate both examples as actual static ELF packages, and verify supervised
install/review/activate/invoke/cancel/update/disable/remove, stale grants, collision,
capability refusal, isolation/secret boundaries, crash/descendant cleanup, dropped
caller, held-pipe/flood/timeout/race cases and resource recovery. Required Rust
coverage must be measured after final integration under a scheduled released
build slot. No target artifacts or coverage measurement accompany this source handoff.
