# Executable package verification plan

**Not run.** This is the command/acceptance plan for the source draft under #63,
not passing evidence. The owner must have an explicit coordinator-admitted build
slot before executing any command below that builds or runs binaries. Do not queue
a lock waiter before admission. The shared target and build lock remain in use by
other issue owners; remote builder capacity is not assumed.

## Prerequisites and scope

- Linux x86_64 with the repository's required bubblewrap isolation working. Missing
  isolation is a refusal/failure, not permission to use sandbox-off as a substitute.
- Existing shared Rust target, companion Helm/Vessel/Voyage binaries built from the
  same merged source, and approved Go tooling for static external examples.
- The reference Go formatter was unavailable during source work. Neither Go toolchain
  availability nor static linking has been established. Do not silently install or
  use an unadmitted remote builder while another owner holds the slot.
- Fixtures use isolated HOME/XDG directories, synthetic local credentials and the
  existing loopback provider fixture. No live provider requests, actual login,
  production services or host sandbox configuration changes are required.

After admission, hold `/home/psi/voyage/.local/issue-wave-build.lock` throughout
builds, focused checks and final coverage/report generation, using the existing
`/home/psi/voyage/target`. Merge current published main before final measurement.
These paths are coordination details for this checkout, not portable install paths.

## Focused commands after admission

From the merged checkout root, with the slot's explicit target settings:

```sh
cargo check -p voyage -p helm --locked
cargo test -p voyage --locked --lib extensions
cargo test -p voyage --locked --lib extension_sdk
cargo build -p helm -p vessel -p voyage --locked -j 8
```

From `sdk/extension-v1`, after establishing Go tooling in the admitted environment:

```sh
gofmt -w sdk/*.go cmd/*/*.go
go test ./...
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o conformance ./cmd/conformance
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o read-text ./cmd/read
CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -buildmode=exe -trimpath -ldflags='-s -w' -o transform ./cmd/transform
```

Keep generated binaries and raw evidence under ignored local evidence storage;
the relative Go `-o` names above are illustrative author commands, not permission
to add binaries to Git. Supply actual output paths for the scheduled measurement.

From the checkout root:

```sh
python3 voyage/tests/executable_packages.py \
  --bin-dir /home/psi/voyage/target/debug \
  --conformance-bin PATH_TO_STATIC_CONFORMANCE \
  --read-bin PATH_TO_STATIC_READ_EXAMPLE
```

This fixture reuses `voyage/tests/delivery_recovery.py` for supervised connections,
private synthetic state and positive cleanup assertions. It does not restore the
removed broad test suite. Its own source syntax check is not fixture execution.

## Cases and evidence

The source fixture exercises:

1. Real pack/install/inspect, inactive installation, refusal of format-1 enable,
   exact capability review and supervised operator invocation.
2. Structured commands and actual run-start/run-finish handlers, separate durable
   invocation response and cleanup records, and returned result content.
3. Required-profile denial of host `/etc/passwd`, networking, session escape and
   inherited environment, plus bounded private temporary storage availability.
   These are focused probes, not the whole OS-security matrix.
4. No executable fallback in sandbox-off or read-only runs.
5. Stale expected digests, revoke-before-drain update/disable during an active call,
   cancellation, observed cleanup, replacement and noninherited review.
6. Crash, newline-free flooding and a descendant retaining pipes; a later explicit
   invocation is fresh work, never a replay of the failed call.
7. Artifact-integrity refusal and inactive project shadowing without user fallback.
8. The separately authorized host-file example, denied outside-root and explicit
   private-config reads, hard-link refusal, disable and removal.

Rust test source additionally covers strict duplicate JSON, framing/schema bounds,
queue closure, ELF range/interpreter/permission refusal, declarative separation,
symlinks, stale reviews, unobserved-vs-attested-vs-observed reservation state and
publication interruption before and after writing new bytes. Additional source
regressions cover decoded progress/result secret fragments, private configuration
provenance across serialization/rename and retained cancelled blocking-read workers.

Retain each actual command, exit status, source identity, platform/tool versions,
fixture snapshots, failures and cleanup evidence. Never turn a timeout into a pass.
The final default-feature workspace Rust coverage run follows AGENTS.md exactly;
commit `coverage/latest.json` with the verified delivery. Validate reused report
objects belong to the measured workspace and preserve any mixed/raw reports.
Go/Python/native journeys are separate from the Rust coverage percentage.

Further malformed-response, dropped-caller, current-policy/delegation, private-store,
secret-fragment and shutdown-race cases must be assessed against actual results;
this written plan does not establish that they pass. Live progress presentation,
full platform readiness and paid-provider behavior are separate claims. Neither
#63 nor #75 closes from a syntax check, protocol schema or unexecuted fixture.
