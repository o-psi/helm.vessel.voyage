# Production coverage gap ledger (#353)

## Objective and scope

Target: 100% of reachable production behavior with meaningful assertions. This is
unfinished work, not a declaration that a lower percentage is sufficient.
The measurement remains the full Rust workspace with default features and the
standard test/dependency exclusions. No module exclusions were added. Web/JavaScript,
Python process checks, doctests, native other-platform runs and branch coverage are
not represented by this line percentage.

## Audited checkpoint

The initial September 27 baseline was 80,902 / 107,716 lines (75.106762%).
The verified expanded checkpoint `coverage100-wave10` is 83,101 / 107,727 lines
(77.140364%), with zero failed tests and three ignored tests. Its test count includes
seven duplicate baseline installer tests; those duplicates were removed before the
final delivery measurement. Consult `coverage/latest.json` for final counts/totals.
The 11-line denominator increase is test seams/new helper structure, not a narrowed
production scope. Existing inline baseline tests were retained in their original
measured location. Coverage measures execution, not assertion quality or correctness.

## Largest remaining areas

These are reachable test gaps unless a specific environment restriction is named.
They must not be relabeled untestable merely because fixtures are difficult.

| Area | Uncovered lines | Total lines |
|---|---:|---:|
| `helm/src/process_client` | 8932 | 32132 |
| `vessel/src/process` | 2203 | 9325 |
| `voyage/src/server` | 1646 | 4286 |
| `voyage/src/attachment` | 1457 | 9052 |
| `voyage/src/tools` | 1440 | 7417 |
| `voyage/src/github` | 806 | 3137 |
| `voyage/src/extensions` | 670 | 1995 |
| `voyage/src/subagent` | 512 | 2404 |
| `voyage/src/provider` | 511 | 5247 |
| `helm/src/plain_terminal.rs` | 278 | 469 |
| `crates/voyage-protocol/src` | 230 | 1582 |
| `voyage/src/build` | 229 | 594 |
| `voyage/src/agent.rs` | 226 | 1392 |
| `voyage/src/workflow` | 220 | 412 |
| `vessel/src/main.rs` | 202 | 299 |
| `voyage/src/host_browser.rs` | 176 | 1220 |
| `helm/src/managed` | 175 | 372 |
| `voyage/src/server.rs` | 173 | 173 |
| `voyage/src/completion` | 172 | 717 |
| `helm/src/clipboard.rs` | 170 | 375 |

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

Final local evidence is retained under `target/coverage-report/coverage100-final`;
the detailed baseline map is `target/coverage100/baseline-gaps.json`. These ignored
artifacts are local evidence, not portable source files. Issue #353 remains open
until the target is actually met or the operator changes scope.
