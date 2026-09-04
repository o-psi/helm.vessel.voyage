# Voyage 0.1 delivery map

Voyage 0.1 is a complete local-first system, not an MVP milestone. Work is divided
into streams with explicit ownership so they can progress in separate Git worktrees.

| Stream | Branch | Issues | Primary ownership |
| --- | --- | --- | --- |
| Helm UI | `work/helm-ui` | #1–#4 | Ratatui frontend, streaming UX, sessions and artifacts |
| Helm runtime | `work/helm-runtime` | #5–#7, #20 | Agent reliability, PTYs, tools, MCP and patch editing |
| Helm connectivity | `work/helm-serve` | #8–#10 | Pairing, outbound task protocol, identity and enrollment |
| Vessel | `work/vessel` | #11–#14 | Persistence, scheduler, leases and operations UI |
| Security and operations | `work/security-observability` | #15–#16, #21 | Isolation, approvals, audit, metrics and diagnostics |
| Quality and release | `work/quality-release` | #17–#19, #22 | Tests, evals, readiness gates, dogfood, packaging and deployment |

## Integration rules

- Shared wire changes originate in `work/helm-serve` and include compatibility tests.
- Security requirements are treated as acceptance criteria in every stream; the
  security stream owns common enforcement and adversarial coverage.
- Quality gates land early and expand alongside each feature, rather than being a
  final stabilization phase.
- Each issue receives focused commits and is integrated through a Forgejo pull request.
- `main` must remain formatted, Clippy-clean, and green under workspace tests.

## Completion definition

The milestone is complete only when Helm provides its full-screen terminal experience,
remote execution has a versioned authenticated lifecycle, Vessel survives restart and
manages queued work, policy is defense-in-depth, operators can diagnose the system,
and supported platforms receive reproducible, documented release artifacts.

Issue #19 is the explicit incumbent-harness replacement gate. Completing individual
features is insufficient until #22 demonstrates the representative workload suite and
dogfood period without critical fallbacks.
