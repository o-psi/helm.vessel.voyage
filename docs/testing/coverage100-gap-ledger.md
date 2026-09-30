# Production coverage gap ledger (#353)

## Objective and scope

Target: 100% of reachable production behavior with meaningful assertions. This is
unfinished work, not a declaration that a lower percentage is sufficient.
The measurement remains the full Rust workspace with default features and the
standard test/dependency exclusions. No module exclusions were added. Web/JavaScript,
Python process checks, doctests, native other-platform runs and branch coverage are
not represented by this line percentage.

## Latest separately published baseline

Concurrent browser-controls delivery #379 published a newer full-workspace record
in `675f12ed1f05b03f6fc45828993f95fec8e55b85`, measuring clean source
`782354d2eaba298d50d10baeecd4bc524ab35d71` on September 30:
**89,290 / 116,781 lines (76.459356%)**, 8,111 / 11,013 functions and
140,542 / 192,137 regions; **2,039 passed, zero failed, eight ignored**.
Its report retained all 504 prior workspace files, the same standard-library
source and 16 distinct current object hashes, with no new exclusions. The record
adds 136 covered and 128 measured lines relative to the checkpoint below.

The release integration branch incorporates that delivery, but its identity,
adoption, recovery, browser-shadow and prepared coverage changes are newer and
remain unmeasured. This baseline is evidence for its named source only. No new
tests or coverage were run by the release coordinator during the user's current
implementation-first hold; the final combined measurement is still required.
`coverage/latest.json` records the published measurement, not the current release
branch's coverage. The detailed area table below is the retained September 29 gap
map; it has not been silently regenerated or presented as current edited totals.

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

## Largest remaining areas

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

## Reachability and evidence ledger for the next pass

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
