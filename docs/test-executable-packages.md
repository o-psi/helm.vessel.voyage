# Executable package verification (Linux x86_64)

## Observed results

The local #63 verification used the merged #14/#21/account/#32 source, not the
initial source-only draft. The final focused fixture passed **18 recorded groups**,
including positive fixture cleanup, through real Vessel-supervised Voyage owners.
The endpoint was an isolated scripted local provider; no paid provider, real login,
production service or host sandbox configuration change was used.

- Locked Voyage/Helm check and Helm/Vessel/Voyage development builds passed.
- **17 focused Rust extension/SDK tests passed** before final workspace coverage.
  The final whole-workspace measurement and its actual test totals are recorded in
  [coverage/latest.json](../coverage/latest.json), not inferred from this subset.
- **3 Go SDK tests passed**, and transformation, read and conformance examples built
  as static x86_64 Linux ELF executables. Go source formatting passed.
- Warning-mode Clippy completed across affected packages/all targets without
  extension diagnostics. Strict Clippy **did not pass**: inherited account/provider
  and UI warnings remain. New extension style findings were corrected, not silenced.
- JSON, Python source syntax, documentation paths and diff checks passed.

### Native fixture groups

1. Real pack/install/inspect, inactive installation, refusal of declarative enable,
   exact capability review, supervised invocation and actual lifecycle records.
2. Structured operator commands and required-profile denial of host files, network,
   session escape and inherited environment; no leaked setup descriptors/system
   runtime mounts; private temporary storage remains usable.
3. No executable fallback for sandbox-off or read-only runs.
4. Concurrent active-run controls do not replay lifecycle handlers. Exact repeated
   operator-command admission preserves the original run and lifecycle records.
5. Stale digest refusal, revoke-before-drain update/disable during active work,
   cancellation, observed cleanup, replacement and noninherited review.
6–11. Plugin crash, newline-free flood, wrong invocation ID, duplicate terminal
   response, unauthorized host capability, and a descendant retaining pipes.
   Each is followed by observed cleanup and a fresh explicit invocation, not replay.
12. SIGKILL of the exact fixture-owned Voyage process, independent guardian cleanup,
   fenced recovery and a fresh turn. The old tool invocation remains singular with
   an **unknown** outcome; recovered cleanup is not fabricated tool success.
13. Artifact integrity failure and inactive project shadowing without user fallback.
14. The standalone transformation SDK example returns the actual uppercase result.
15. The separately authorized host-file example; outside-root, explicit in-workspace
   private configuration and hard-link reads are refused.
16. A colliding manifest excludes the whole package before initialization/effects;
   its unique tool is also unavailable and its invocation ledger stays empty.
17. Disable and removal.
18. Positive fixture process and durable cleanup observation.

The successful held-child case is intentional: PID-namespace teardown stops the
child when its parent exits, allowing the valid result to complete cleanly. A
held pipe is not automatically a failed tool if actual cleanup is observed.

## Tools, acquisition and commands

Observed local tools: Rust **1.98.1**, LLVM **22.1.8**, cargo-llvm-cov **0.9.1**,
bubblewrap **0.12.0**, and Go **1.27.1**, Linux x86_64. Go was initially absent from
the command path. Its user-local installation used system-CA-verified HTTPS from
`https://go.dev/dl/?mode=json` and the official archive:

- `https://go.dev/dl/go1.27.1.linux-amd64.tar.gz`
- Size: **70,553,950 bytes**
- SHA-256: `63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445`

The archive size/hash were checked before extraction, and archive paths/types were
validated. No system-wide upgrade or implicit runtime installation was performed.
Package activation does not install Go or download runtimes.

Commands, with the existing shared target selected under the coordinator's build
lease (replace example paths with the actual locally built artifacts):

```sh
cargo check -p voyage -p helm --locked -j 8
cargo test -p voyage --locked --lib extensions -j 8
cargo test -p voyage --locked --lib extension_sdk -j 8
cargo build -p helm -p vessel -p voyage --locked -j 8
cargo clippy -p voyage -p helm -p vessel --all-targets --locked -j 8
```

From `sdk/extension-v1`:

```sh
gofmt -w sdk/*.go cmd/*/*.go
GOTOOLCHAIN=local go test ./...
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o CONFORMANCE ./cmd/conformance
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o READ_EXAMPLE ./cmd/read
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o TRANSFORM ./cmd/transform
```

From the checkout root:

```sh
python3 voyage/tests/executable_packages.py --bin-dir TARGET/debug \
  --conformance-bin CONFORMANCE --read-bin READ_EXAMPLE --transform-bin TRANSFORM
```

The fixture reuses `voyage/tests/delivery_recovery.py` for supervised connections
and private synthetic state. It does not recreate the removed broad suites.
The final coverage workflow follows AGENTS.md; Go/Python/native checks are separate
from workspace Rust percentages. Every current Cargo workspace executable must be
retained when correcting stale shared-target report selection. Source/object
validation, detailed reports, profiles and logs stay in ignored target evidence.

## Retained failures and limits

Earlier failures remain retained, not rewritten as passes:

- A private-config unit fixture omitted the required local endpoint for no-auth
  provider configuration; its test configuration was corrected.
- Rust's internally tagged unit variant accepted extra shutdown-ack fields. The
  variant was changed to a strict empty struct and the regression then passed.
- The first native fixture used active `execute_tool` instead of idle
  `operator_tool`; a later control-receipt assertion looked at the outer admission
  status rather than the nested outcome. Both schema assumptions were corrected.
- The initial held-child assertion expected failure despite successful namespace
  cleanup. The corrected check requires actual child creation and observed cleanup.
- The runtime-crash fixture initially expected `failed`; the retained journal
  correctly recorded `interrupted`. The assertion was corrected without increasing
  the timeout or replaying the interrupted effect.

Raw local evidence is retained under ignored `target/issue-63-evidence` and
`target/coverage-report/issue63-final`. No private fixture state is published.
Coverage measures execution, not correctness. This evidence does not establish
native macOS/Windows behavior, arbitrary filesystem-stall guarantees, aggregate
resource budgets, every hostile encoding or a full live progress UI. #75 retains
its broader SDK/UI/support obligations; unsupported capabilities fail closed.
