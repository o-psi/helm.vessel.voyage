# Full Terminal-Bench 4 preparation (#398)

The requested full experiment uses **gpt-6.1-sol medium**, the existing native
ChatGPT subscription account, every official TB4 task, and **five attempts per task**.
Upload the inspected results privately to Harbor Hub. It has **not started**; there
are no full-run scores or Hub upload claims.

The official `terminal-bench/terminal-bench@4.0.0` download resolved to 66 tasks and
content hash `sha256:39d9f44b40420cde8fdcc087579c0d72a7e14fa3656d603c3f0d22fb35e27732`.
The [pinned plan](tb4-plan.json) preserves each task's Hub content reference. The
complete experiment is 330 trials. Do not substitute a local subset, shorten agent
or verifier budgets, or omit inaccessible tasks while calling it a full run.

## Required resources and unresolved access

| GPU task | GPU | CPU | RAM | Declared storage |
| --- | --- | ---: | ---: | ---: |
| fp8-rmsnorm-gemm | 1 H100 | 4 | 16 GiB | 30 GiB |
| math-eval-grader | 1 H100 | 8 | 16 GiB | 30 GiB |
| jax-speedrun-gpu | 1 H100 | 16 | 32 GiB | 1000 GiB |

The current PC has integrated AMD graphics and no usable NVIDIA device. About
593 GiB disk space was available at inspection. No Modal/Daytona credentials were
configured. The full run needs an approved H100-capable host or cloud environment,
including its resource budget and credentials. Prior inference authorization does
not silently authorize a new cloud resource provider. Honor all native task resource
settings, including multi-container task services; rootless Docker on this PC alone
cannot fulfill the declared GPU/storage requirements.

Harbor CLI initially reported unauthenticated. Its official GitHub OAuth flow was
opened for the human to complete. Do not put passwords, callback codes or API keys
in chat, logs, job configs or uploads. Verify `harbor auth status` and actual Hub
access before promising an upload. A prepared local config is not execution evidence.

The task's [delivery issue](https://github.com/o-psi/helm.vessel.voyage/issues/398)
retains the current blockers, checks, build identities and remaining obligations.
Keep it open until the full run and verified upload finish.

## Adapter changes and local evidence

All TB4 tasks declare an eight-hour agent deadline. The old fixed 1800-second
wrapper deadline was too short. `VoyageAgent` now accepts `run_timeout_secs`,
defaulting to 28800 with a bounded range. Its subprocess guards allow that declared
budget plus bounded shutdown overhead; Harbor still owns the actual task deadline.
Shorter task deadlines still trigger exact-incarnation stop and observed cleanup.
No native agent loop, tool dispatch, provider configuration or task budget is replaced.

`runner.py` collects complete **public** message JSON through Helm's typed
`message_chunk` operation. Each response pins session, revision, index and UTF-8
byte offset; a final history request checks the revision. The export is bounded by
100,000 messages, 64 MiB and an observation deadline. Missing/stale/oversized output
is unavailable evidence, never a fabricated complete transcript. Canonical completion
remains independent of export success.

`trajectory.py` converts that history into validated Harbor ATIF-v1.8 with exact
native tool-call/result linkage. It excludes private reasoning and records image
metadata without resolving private pixels. It represents only the root public history;
subordinate private histories are not inferred or synthesized. Unknown/duplicate
identities fail rather than silently inventing actions. An incomplete public export
cannot be labeled a complete trajectory.

Offline checks verified a native file task with the eight-hour wrapper configuration,
complete public-history collection, validated ATIF and actual output/runtime cleanup.
Focused checks cover Unicode byte cursors, stale revisions and expired observation
bounds. A short forced-stall task verifies that the expanded wrapper does not override
the task's shorter deadline. These are integration checks, not full-run capability scores
or evidence that an eight-hour model trial was exercised.

Some task images use Ubuntu 22.04. A compatible local variant of unchanged source
`8e109046fb0bff11b17c3247a86a1afb098ccf83` was built with Rust 1.94.1 in Ubuntu 22.04,
release/locked/default features plus `reqwest/rustls-tls-native-roots`. The native
current-runtime smoke verified actual file output, complete public history, ATIF and
cleanup. The pinned plan records version 1.0.3, build image identity and binary hashes.
This is a local benchmark variant, not a published release; full-task image compatibility
and remote H100 controls remain necessary. Preserve existing runtime/build artifacts. No Rust source, Cargo
inputs or tracked tests are changed by this adapter preparation, and no new Rust
coverage or hosted test/coverage job is implied.

## Admission and upload procedure

1. Resolve the official Hub dataset reference and verify all 66 pinned task references.
   Use Harbor's package `ref` field for the resolved content hash, not a local task list
   or a mutable `latest` label. Keep `n_attempts=5`, default task budgets and no resource
   overrides. Freeze the final adapter and runtime identities before any inference.
2. Verify the approved execution environment on native CPU, H100 and multi-container
   controls. Remote sandboxes need a verified authenticated relay to the host credential
   broker; the existing Unix-socket mount is local-only. Do not copy real subscription
   credentials into untrusted task containers or expose an unauthenticated broker.
3. Reconcile account readiness with the native host account manager before long runs.
   The broker must refuse stale/changed account authority; do not replay uncertain
   provider effects or rotate subscription accounts implicitly.
4. Complete all 330 attempts and independently record rewards, exceptions, canonical
   completion, observed usage, unknown monetary cost and runtime/container teardown.
   Native run-owned service cleanup remains a known workload constraint. Do not disable
   ownership cleanup to inflate service-based scores.
5. Materialize and inspect a separate upload copy. Keep all trial IDs, original config,
   lock/task provenance and outcomes; exclude credentials, host-private files and hidden
   reasoning. Audit every uploader-included directory, including `agent`, `verifier`,
   `artifacts`, logs and exception files. A trajectory alone does not sanitize the archive.
6. Run `harbor upload /path/to/inspected-job --private` with authenticated Harbor access.
   Reconcile uncertain/idempotent upload results before retrying. Read back the job ID,
   private visibility, every task and five attempts each. Retain the actual Hub link and
   remote trial counts in #398 before calling the upload complete.

Private Hub storage is separate from the official leaderboard. Community TB4
leaderboard submissions are currently closed; do not promise a leaderboard row.
See the [official run guide](https://www.tbench.ai/run) and
[submission policy](https://github.com/harbor-framework/terminal-bench/blob/main/leaderboard/SUBMIT.md).
