# Owned ordinary frontend cohort preparation (#353)

This extends the connected frontend family in
[the reachability map](coverage353-reachability-map.md). Audited source
`c15ef79b6568616fc7c0d24350ef3defdd97ee67` has 162 uncovered / 317 measured
lines in `helm/src/process_client/frontend.rs`. Prior scripted tests qualify
opening/configuration refusal contracts; this preparation executes public
`open`/`run` through real ordinary service and runtime processes.

## Actual owners and explicit effect bounds

A parent Helm library test starts one explicitly selected child test per journey.
The child has its own Linux session, verifies its parent PID/session witness,
clears inherited environment and uses private synthetic HOME/XDG/workspace.
Only the LLVM profile destination and necessary fixture PATH/markers are retained.
An owned current `vessel local-serve` process is prestarted in that private state
directory with the current `voyage` executable. `local::connect` observes this
service; it cannot start a service in the developer account. Vessel launches the
ordinary Voyage independently through its production process supervisor.

The only provider is an owned literal-loopback HTTP/SSE listener, with credentials
disabled and a randomly named absent API-key environment variable. It asserts that
no Authorization header is sent. Counted held inference establishes active work
before release. It offers only synthetic models and a fixed canonical response,
or one synthetic HTTP 500 refusal with one permitted attempt. There is no login,
paid provider, Root identity, system unit, human terminal, browser or external API.

## Five substantive process journeys

| Journey | Acceptance and failure assertions |
| --- | --- |
| Saved frontend run/resume | Public run captures private configuration, starts one session/independent Voyage, accepts the exact canonical workspace and user prompt once, completes one inference and preserves one canonical assistant answer. Public resume keeps session/workspace/history and creates no second session or provider request. |
| Fresh temporary run | Public `no_save` run completes one provider request; the deletion receipt has its command UUID, `status=applied`, `deleted=true`, `cleanup=observed` and a stopped process. Snapshot of the deleted session refuses. |
| Temporary provider failure | The one synthetic provider failure produces the actual plain frontend `state failed` result, followed by exact observed deletion/stop. A generic unrelated connection error does not satisfy the failure oracle. |
| Temporary branch with explicit configuration | A real parent session remains unchanged while the public frontend creates a separate no-save branch, configures its selected model and runs one exact request. Only the branch is deleted with observed cleanup; the parent's canonical messages and model remain byte-equivalent JSON values. |
| Actual lost Submit reply | A private scripted forwarding socket sends the original command to the real Vessel, waits for the actual accepted response, then closes without returning it. The frontend reports uncertainty. The original serialized Submit/UUID resolves to the same accepted run, while a changed original payload refuses. Reconfiguration of the held active run refuses. Disconnecting Helm keeps that same run alive; release completes exactly one canonical answer with no replacement prompt and one inference request. |

The forwarding fixture only controls response delivery. It does not admit a run,
create a journal, invent a receipt or host an executor. Every accepted result and
retained receipt comes from the actual ordinary Vessel/Voyage processes.

The held runtime is observed as exactly one process for that session's private
directory and executing Voyage binary, distinct from Vessel and the Helm child.
Each PID is pinned to its `/proc` start identity. Cleanup first stops any remaining
owned session through Vessel, then requires all executing Voyages to disappear,
the pinned runtime identities to retire and inspection to leave no live/starting/
unconfirmed process. Only after that evidence is the fixture Vessel killed and
reaped. Failure teardown signals only the exact owned executable/private-root/PID
identity; this fallback is not called successful runtime cleanup. Parent children
have bounded waits/output and must leave no executing Vessel or Voyage behind.

## Runnable source identity and coordinated verification

**Preparation has not run Cargo, compiled, executed this fixture or measured
coverage.** Rustfmt, source/path review and `git diff --check` are the only checks.
The source adds a Linux test module declaration and one new test file; no dependency,
production behavior, hosted quality job or Root/platform capability is added.
#353 remains open with the full reachable-production objective and denominator.

The fixture derives `target/debug/vessel` and `target/debug/voyage` adjacent to the
current Helm test executable. Their existence or `v1.0.3` version alone is **not**
source identity. Before execution, the sole Cargo coordinator must build the
required runnable binaries at the exact integrated source, preserving the current
coverage toolchain, target and forwarding wrapper. In the existing instrumentation
environment (including `-C instrument-coverage` and the own-workspace profile
prefix), the required compile command is:

```sh
cargo build -p vessel -p voyage --bins --locked -j 1
```

Do not run that as an ordinary uninstrumented build over the audited publication
paths. Establish the instrumentation environment using the installed
`cargo-llvm-cov` version's `show-env --export-prefix` support under the coordinator's
bounded unit, preserving its existing flags/wrapper; inspect the actual returned
environment before applying it. Keep source frozen through execution and export.
Record the exact source commit/dirty fingerprint, executable hashes, matching Cargo
depfile source/version identities, current hardlink aliases and LLVM coverage
mapping/counter sections. Reject historical or foreign binaries. If the current
workspace's Cargo test build already produced those exact audited binaries, reuse
them after proving that identity; do not rebuild merely for ceremony.

The focused default library filter, still inside that exclusive window, is:

```sh
cargo llvm-cov --workspace --no-clean --json --summary-only --output-path target/verification-v103/frontend-owned-focused-summary.json --locked -j 1 -- process_client::frontend::owned_process_tests::owned_ordinary_frontend_preserves_canonical_history_exact_delivery_and_cleanup
```

Include the actual executed Vessel/Voyage entrypoint objects and their own raw
profiles in the final corrected export. They are existing runnable workspace
entrypoints, but the coordinator must verify their *current* binary membership and
source identity; passing the Helm parent harness does not establish that. Retain
child libtest summaries separately from the one Cargo parent target result.
Focused mixed profiles are not a publishable workspace coverage record.

Inspect concrete failures, preserve canonical/uncertainty/cleanup assertions and
fix substantive causes. Run strict applicable checks and one combined independent
full workspace measurement after all cohort corrections, including the separately
prepared plain terminal work. Audit all prior/current source/object/profile
membership before publishing the compact summary and update the shared #353
record with exact checks, source and remaining gaps. No 100% or native remote/provider
claim follows from these five ordinary local journeys alone.
