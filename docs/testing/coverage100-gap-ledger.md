# Production coverage gap ledger (#353)

## Objective and scope

Target: 100% of reachable production behavior with meaningful assertions. This is
unfinished work, not a declaration that a lower percentage is sufficient.
The measurement remains the full Rust workspace with default features and the
standard test/dependency exclusions. No module exclusions were added. Web/JavaScript,
Python process checks, doctests, native other-platform runs and branch coverage are
not represented by this line percentage.

## Current audited baseline

Clean source `ca71ecdbe32e30277741eab7145d88c2d66cd7bd`, recorded in
`coverage/latest.json`, passed **2,339 tests, zero failures, eight ignored**.
It covers **96,873 / 131,550 lines (73.639681%)**, 8,776 / 12,018 functions
and 152,704 / 217,634 regions. **34,677 line gaps remain**, including one known
standard-library line. This does not meet #353's reachable-production objective.

All **536 current workspace files**, all **535 prior files** and the same known
standard-library source are retained. All 14 test targets and three integration
entrypoints contribute 17 distinct instrumented objects. No foreign-checkout
source maps remain. Only 248 own profiles were merged; 235 foreign profiles remain
unchanged. The mixed default export with 2,092 source mappings was rejected.
Exact manifests and reports remain under ignored `target/coverage-report/v103-bound-final`.
Earlier qualified raw profiles and all 17 prior object hashes were preserved.

Compared with the published `83ab2bf` record: 242 more covered lines, 513 more
measured lines and a 0.103613 percentage-point drop. New ordinary wake and
independent guardian authority paths expand the unexecuted denominator. No
exclusions were added. Their proof/refusal cases passed; actual native execution
and same-browser client journeys remain separate acceptance gates.

Historical detailed exports require regeneration against current audited
objects/profile before selecting line addresses. Native Root, external-provider
and other-platform evidence is not inferred from this default-feature percentage.

## Historical audited checkpoint and area map

The September 29 clean-source checkpoint is
`7b72f40bd25c5783ca78e8422f689f45404b42c1`, published with its summary in
`0fdb477427a1115d8ee1cc831b41b30dff2ed70e`: **89,154 / 116,653 lines
(76.426667%)**, 8,100 / 11,003 functions and 140,268 / 191,926 regions.
Workspace tests passed: 2,036 passed, zero failed, eight ignored. The prior
checkpoint had 89,011 / 116,082 lines; the new result adds 143 covered and 571
measured lines, a 0.252749 percentage-point drop. Fresh system installer native
transaction paths added unmeasured ordinary-user behavior. No denominator
exclusions were added; native fixture success was not counted as coverage.

The corrected LLVM export retained all 503 prior workspace source files, using
all 13 executed workspace test binaries plus three current production entry points
with 16 distinct object hashes. The default export mixed historical objects; its
raw evidence is retained but its totals are not published. The corrected summary
contains 504 workspace files plus the known standard-library thread-local source
(#251: 5 lines, 4 covered). Thus the reported 27,499 uncovered lines include one
standard-library line; 27,498 are attributed to workspace source files. These are
line gaps, not proof of unasserted production branches or unreachable behavior.
Coverage measures execution, not assertion quality or correctness.

This ledger was derived from the retained `summary-current.json`, not a new export
from shared objects changing during builds. It predates the root-local lifecycle
and public configured-start increments; their final numbers require the parent's
combined measurement. `coverage/latest.json` remains authoritative.

## Historical September 29 areas

These are reachable test gaps unless a specific environment restriction is named.
They must not be relabeled untestable merely because fixtures are difficult.

| Area | Uncovered lines | Total lines |
|---|---:|---:|
| `helm/src/process_client` | 9,253 | 33,313 |
| `vessel/src/process` | 3,323 | 11,675 |
| `voyage/src/server` | 1,751 | 5,230 |
| `voyage/src/attachment` | 1,639 | 11,668 |
| `voyage/src/tools` | 1,417 | 7,737 |
| `installer/src` | 1,242 | 3,993 |
| `voyage/src/github` | 806 | 3,137 |
| `voyage/src/extensions` | 661 | 1,995 |
| `voyage/src/provider` | 549 | 5,747 |
| `voyage/src/subagent` | 507 | 2,399 |
| `helm/src/plain_terminal.rs` | 278 | 469 |
| `crates/voyage-protocol/src` | 228 | 1,820 |
| `voyage/src/server.rs` | 227 | 227 |
| `vessel/src/main.rs` | 225 | 323 |
| `voyage/src/workflow` | 220 | 412 |
| `voyage/src/build` | 214 | 593 |
| `vessel/src/process_http.rs` | 193 | 583 |
| `voyage/src/host_browser.rs` | 178 | 1,224 |
| `voyage/src/agent.rs` | 177 | 1,227 |
| `helm/src/managed` | 175 | 373 |
| `voyage/src/participant` | 173 | 308 |
| `voyage/src/completion` | 172 | 717 |
| `helm/src/clipboard.rs` | 170 | 375 |
| `helm/src/main.rs` | 163 | 478 |
| `voyage/src/accounts.rs` | 158 | 775 |

## Current largest areas and concrete next assertions

This map is derived from the current audited `ca71ecd` export above.
Directory rows aggregate that directory only; the identity-helper row also
includes its same-named entry file. An unexecuted line is not proof of an
unreachable branch or a missing assertion. Native requirements remain required
evidence, without being removed from this denominator.

| Area | Uncovered lines | Total lines |
|---|---:|---:|
| `helm/src/process_client/` | 8,595 | 33,720 |
| `vessel/src/process/` | 8,677 | 18,996 |
| `installer/src/` | 3,899 | 7,287 |
| `voyage/src/server/` | 1,748 | 5,611 |
| `voyage/src/attachment/` | 1,640 | 12,432 |
| `voyage/src/tools/` | 1,397 | 7,865 |
| `voyage/src/github/` | 788 | 3,137 |
| `voyage/src/identity_helper.rs` and `identity_helper/` | 336 | 1,503 |
| `voyage/src/subagent/` | 478 | 2,402 |
| `voyage/src/provider/` | 424 | 5,747 |
| `voyage/src/extensions/` | 384 | 1,995 |

The largest individually unexecuted modules are system adoption (1,623 lines),
Vessel migration (1,064), identity transition (819), administrative execution
(528), and identity-account supervision (448). Do not treat them as passing
because ordinary metadata or fixture checks succeeded. Their scoped admission,
durable exact-effect claims, credential separation, boot/service publication,
retired-owner transfer and rollback paths require meaningful local fault
assertions plus the applicable disposable native journeys.

The new identity-helper preparation exercises real ordinary private registries
under isolated child process HOME/XDG directories: scoped account/default and
transport visibility, actor-bound retained enrollment metadata, denied/stale
bindings, profile secrecy, private configuration review, exact frozen retry,
missing publication retention, immutable base settings and participant provenance.
Two scripted loopback model-catalog journeys assert the selected endpoint and
capability-change refusal; they do not execute inference or call a live provider.
Inherited provider variables and credentials are cleared; only the coverage file
destination is retained when instrumented. Root-peer network/enrollment/default
mutations still need native evidence and are not replaced by these fixtures.

The prepared scope-authority cases assert exact current-grant matching/revocation,
refusal of unprotected minting and malformed request identities, bounded local
framing/timeout with fixed sanitized refusal, and observed cleanup of an owned
Tokio task. They retain the production root/epoch/UID gates. They do not establish
successful root service startup, positive cross-UID lease delivery or native
process cleanup. These source assertions passed in the current full-workspace run; they do not
replace the separate native qualification obligations above.

## Environment-specific evidence still required

- Protected Linux control-store owner separation: the ignored native-root test
  requires the disposable environment documented in `docs/quality.md`. No policy
  weakening or root execution on this development machine was performed.
- Live-provider authentication/conversation tests require provider and budget
  authorization. Synthetic provider replies do not establish live service behavior.
- macOS/Windows paths require builds and native behavior tests on those platforms.
- Actual terminal capture/raw-mode restoration, Chromium page execution, systemd
  activation, descendant cleanup under process death and OS resource exhaustion
  require controlled integration/fault environments. Synthetic state/transport tests
  cover only their explicit contracts, not all those real effects.

## Reachable work remaining

Expand UI account/profile and event-loop state journeys; supervisor account/start and
participant ownership races; runtime bootstrap/admission/cleanup; external-tool
transport/cancellation; filesystem error boundaries; browser mirror/private-fence
interleavings. Preserve exact operation identities and assert no automatic effect
replay. Use bounded local fixtures and explicit fault injection rather than adding
coverage exclusions or deleting defensive checks.

The September 29 corrected evidence is retained under
`target/coverage-report/v103-system-install/summary-current.json`, with the raw
export and current-object audit beside it. Earlier evidence under
`target/coverage-report/coverage100-final` and `target/coverage100/baseline-gaps.json`
is historical. These ignored artifacts are local evidence, not portable source files. Issue #353 remains open
until the target is actually met or the operator changes scope.

## Historical September 29 reachability and evidence ledger

The summary-only export does not include uncovered line addresses or branch
execution. Classifications below come from source responsibilities and existing
fixture boundaries; they do not assign every unexecuted line to an environment.
No gap is declared unreachable and no source is removed from the denominator.

| Measured source | Uncovered / total lines | Feasible next assertions | Separate evidence still needed |
|---|---:|---|---|
| `installer/src/system_install.rs` | 430 / 564 | Explicit scope/identity validation, default identity serialization, journal identity and refused replay | Root ownership, real systemd unit publication/readiness/reboot on disposable native Linux |
| `installer/src/system_preflight.rs` | 294 / 401 | Account/path/namespace fact validation and truthful unsupported classifications | Actual root capabilities, namespace restrictions, effective credential provisioner and distinct OS identities |
| `vessel/src/process/service.rs` | 386 / 783 | Admission, stale registration and lifecycle decisions with bounded local fixtures | Native independently owned process startup and observed shutdown |
| `vessel/src/process/bound_lifecycle.rs` | 352 / 398 | Protected binding freshness and refusal of fabricated cleanup | Real cross-UID launch, guardian retirement and boot-change evidence |
| `vessel/src/process/guardian.rs` | 308 / 312 | Malformed admission and deterministic receipt/identity validation | Native root subreaper, detached descendants, capability clearing and ECHILD observation |
| `helm/src/process_client/ui/accounts.rs` | 378 / 1,384 | Account/profile state journeys and denied selection with synthetic transport | Real provider authentication only with provider/budget authorization |
| `helm/src/process_client/terminal.rs` | 326 / 452 | Isolated PTY and subprocess failure/restoration assertions | Native terminal capture, signals and raw-mode restoration; do not attach human terminal |
| `voyage/src/extensions/runtime.rs` | 294 / 462 | Local extension transport cancellation, malformed replies and bounded cleanup | External executable/native resource failure behavior where actual OS effects are asserted |
| `voyage/src/provider` | 549 / 5,747 | Protocol parsing and deterministic failure/retry with local scripted providers | Live authentication, billing and conversation under an explicitly approved provider/budget |

The lifecycle coverage increment exercises retained journal reopen across pre-effect,
staging, stopping, publication and saving interruption, durable activation uncertainty,
exact rollback release matching, candidate-start refusal, incompatible schemas, unknown
fields, shared-readable files and hardlinked records. These are local persistence and
admission assertions, not native service tests. Refactoring keeps root UID 0 fixed at
the production reader; fixture readers use only their explicitly owned private files.
The final measurement must retain all existing objects/source and include the new
lifecycle module; its coverage improvement is not claimed before that measurement.

Focused verification for this increment: `cargo test -p voyage-installer --locked
-j 8` with `umask 077` passed 100 tests, zero failures and two ignored tests. Strict
all-target/all-feature installer Clippy and diff checks also passed. These counts
are package verification, not a new workspace coverage measurement.
