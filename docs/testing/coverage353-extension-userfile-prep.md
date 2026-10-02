# Prepared extension registration and UserFile authority journeys (#353)

This is uncompiled, unrun source preparation on
`coverage353-extension-userfile`, based on
`c3f997f8ac1d1d1c821cb7a6a5b05f84f2ca795e`. The objective and boundaries were
[recorded before implementation](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951020330).
No Cargo, tests, shared target/profile writes, main mutation, provider call or
production host operation occurred during preparation.

## Address inputs and scope

The retained audited BEB LCOV has 149 zero-execution candidate input addresses in
`voyage/src/extensions/runtime.rs` and 126 in `server/authorization.rs`. These are
selection inputs, not a reachable count or promised coverage gain. Authorization
is mixed: its separate Broker/root-bound scope is retained in the measured
source but receives no privileged positive qualification here. No denominator,
source exclusions, features or deferred Root/system/adoption/platform scope change.

Existing tests hand-construct `ExtensionTool` with scripted SDK peers, cover SDK
output/confidentiality/quarantine mechanics, and check basic private grant helper
admission. The new eighteen parents cover actual Catalog-to-runtime-to-registry
construction and held UserFile authority at real filesystem tool dispatch. There
are two additional child entry tests which are inert without their explicit
fixture marker. They are not additional independent scenarios.

Only the two production files gain test declarations. New test files are
`extensions/runtime_owned_registry_journey_tests.rs` and
`server/authorization/owned_userfile_journey_tests.rs`.

## Actual behavior and oracles

| Parents | Cross-boundary assertions |
| --- | --- |
| Unsupervised package and complete registration | Real reviewed Catalog packages are absent without a supervised process scope. With ordinary fixture scope, actual `register` contributes exact namespaced tool/command definitions and retains one executor. Lifecycle handlers are internal; readonly lifecycle dispatch admits no child. |
| Host-read provenance and ceiling | Incomplete configuration provenance excludes host-read packages as a whole. Complete source provenance and an existing read-file ceiling admit them without launching code. A later host-read restriction cannot be widened. |
| Whole-package collision and scope precedence | An existing independent tool keeps its actual result; a colliding package contributes no partial command/lifecycle set or executor. Same-named project/user packages obey actual project precedence and do not install competing executors. |
| Revocation and shutdown | Revocation removes fresh registration without silently rewriting an existing snapshot. Closed Manager admission refuses before publishing candidates. Actual shutdown closes all retained SDK tasks, with no package child present. |
| Actual Adapter/registry refusal | Wrong session/incarnation/run/package/digest, expired deadline and cancelled context refuse before executable sealing, reservation or spawn. Registry cancellation/readonly checks preserve registered definitions and contribute no child. |
| Held ordinary session grant | Successful real `read_file` and `write_file` establish the baseline. Current revocation/expiry/revision/principal/Execute withdrawal causes both subsequent real tool calls to fail; no late output file appears and accepted bytes remain. |
| Connection-derived and participant policy | Current parent identity, rights, expiry, workspace and Vessel binding changes refuse before tool effects. Accepted participant cancellation/expiry/principal/parent-session changes refuse; permitted binding revision advancement alone does not revoke continuing scope. No assignment is created. |
| Scoped read and private grant storage | WorkspaceRead permits only the bounded readonly-policy route, not Execute or write. Missing/private-permission/hardlink/symlink grant changes invalidate an already held policy; restored original bytes permit normal observation. |
| Compound admission and local owner | Private terminal and participant cancellation require their complete rights. Owner goal fencing requires a current explicit parent plus History/Execute/Cancel, with no scoped upgrade. An ungranted ordinary owner retains its own actor and receives no ambient UserFile or Broker authority. Wrong workspace/runtime paths refuse. |
| Frozen named account | Normal private Registry APIs create one synthetic stored account at a closed loopback endpoint. Frozen account scope and AccountUse withdrawal or a real registry tombstone refuse before filesystem tool dispatch. The synthetic key never reaches a provider, tool arguments or public output. |

Structural ELF bytes come from the existing package fixture and are used only for
lazy review/registration. **They are never launched** and establish neither
successful extension execution nor kernel isolation. Positive sandbox/executable
acceptance remains a separate obligation. The actual Adapter cases stop at its
pre-image checks; they do not manufacture a spawn result.

## Private fixture ownership

Nine registry parents and the named-account parent spawn only the same current
Voyage LIB test executable with an exact child filter. Each owns a private
TempDir, null stdin, private log, synthetic fixture selector, fifteen-second
lifetime and positive `try_wait` reaping. Environment is cleared; only explicit
private HOME/XDG paths, PATH and the existing LLVM profile pattern are supplied.
No ambient provider credential or human terminal input is inherited. A timeout
kills and reaps the still-owned direct child and fails the parent; it cannot be
reported as a passing cleanup result. No helper/grandchild launch or comprehensive
historical descendant/native cleanup is claimed. Ordinary direct UserFile cases
use their own temporary files and Tokio tasks, with no process-global environment
mutation.

## Single coordinated gate

After independent source review and integration with the other complete cohorts,
freeze the source and run these exact filters under the gate owner's private,
bounded independent ordinary-user unit. Retain the current workspace objects and
instrumentation; one compiler job and no competing Cargo processes:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/extension-registry-focused-summary.json -j1 \
  -- owned_registry_journeys

CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/userfile-owned-focused-summary.json -j1 \
  -- owned_userfile_journeys
```

Focused summaries remain ignored/unpublished; their retained objects can be mixed.
Do not combine `--no-clean` and `--no-report`. Resolve concrete failures and perform
required formatting/strict Clippy, then fresh full workspace coverage after the
final relevant edit. Preserve all previous/foreign profiles and objects, audit
current source/object attribution, publish the compact summary with main delivery
and own the actual hosted archive follow-through. This source preparation does
not close #353, browser/native criteria or the full milestone.
