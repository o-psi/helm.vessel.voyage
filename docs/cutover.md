# Helm dogfood and cutover runbook

This runbook governs adopting Helm as the everyday terminal agent. Feature
completion alone does not establish readiness. Record actual workload, deployment
and recovery evidence before the operator's cutover decision.

## Current scope

Local Helm workflows and the [dedicated remote worker](remote-sessions.md) are
available for scoped evaluation. Enrollment and presence are available through the
[attachment guides](attachment-cli.md). The [multi-Helm voyage](voyages.md), unified
Helm operator interface and remote coordinator/handoff remain planned. Record those
gaps explicitly; current HTTP worker tests do not prove the intended product.

## Entry gates

- Linux CI passes for the exact candidate commit. For macOS or Windows adoption,
  record separate platform evidence; routine CI does not test those platforms.
- Archive checksums and binary versions match the candidate source.
- `python3 eval/run.py live` passes the representative workloads with an explicitly
  configured provider and budget, with reviewed evidence attached to the record.
  `validate` checks definitions only and cannot satisfy this gate.
- Enrollment, restart/reconnect, cancellation, credential revocation and recovery
  drills pass in the intended deployment, including its TLS/proxy configuration.
- No unresolved critical security, data-loss, orphan-process or credential defect
  affects the workflow being adopted.
- The existing working tool remains available and its configuration is backed up.

Record commit, checksums, platform, provider/model, policy, evaluated workflow scope,
evidence, exceptions, approver and UTC decision time. A narrower local trial is not
a claim that the full voyage product is ready.

## Staged adoption

1. Install a verified archive into a versioned directory.
2. Configure deliberately. Keep credentials out of shell history and shared evidence.
3. Run read-only research and inspection tasks for one day.
4. Run writing and disposable-workspace coding tasks for two days.
5. Run ordinary coding and administrative diagnosis with explicit approvals for five
   days. Count and classify every fallback to another tool.
6. Enroll one non-critical Helm and exercise dedicated remote execution, including
   disconnect, cancellation and recovery using the current documented APIs.
7. Make Helm the default for the verified scope only after seven consecutive days
   without critical fallback, data loss, policy bypass, unrecovered session or
   orphaned process. Full multi-Helm acceptance also requires the planned drills below.

Capture task category, outcome, fallback reason, latency, interruptions, approval
surprises and manual repair. Redact secrets and sensitive prompt content.

## Current local and dedicated-worker drills

- Cancel model generation, a tool and a long-running child; verify durable outcome
  and observed cleanup, including any explicit unresolved blocker.
- Close a terminal unexpectedly and verify terminal restoration, child cleanup and
  honest interrupted recovery; never assume a dead PTY survived.
- Restart Helm and exercise the supported session restore/metadata/export workflow.
- Disconnect an operator HTTP observer and reconnect to durable receipts without
  creating another run. Separately lose the worker's outbound transport and verify
  cancellation/cleanup; these are different current behaviors.
- Restart Vessel and the worker around admission/result acknowledgement. Retry the
  exact command identity and confirm that uncertain effects are not blindly replayed.
- Revoke enrollment and confirm subsequent connection, admission and disclosure fail
  under the current authority contract.
- Exercise denied writes, required-but-unavailable approvals, private-session access
  rejection and safe handling of malformed or stale commands.
- Inject provider rate limits and confirm bounded retry and responsive cancellation.
- Exhaust session storage and confirm truthful failure with prior evidence preserved.

## Required multi-Helm voyage drills when implemented

These are planned acceptance checks, not currently runnable product instructions:

- Open an interface on one Helm, coordinate on another and execute on additional
  permitted Helms. Also exercise overlapping roles and local-only use.
- Target a particular participant, then allow coordinator selection within scope.
  Include non-repository work and several directories without a component map.
- Close the interface and reconnect from another authorized Helm; show coordinator
  and participant state independently and avoid duplicate dispatch.
- Add/remove a scoped machine during queued and active work using the agreed
  disposition rules. Deny out-of-scope routing and unauthorized context disclosure.
- Change coordinators at supported handoff points, test conflicting/stale owners
  and preserve unknown effects and cleanup blockers through failure.
- Route questions and scoped approval decisions to authorized interfaces, including
  unavailable operators, expiry, revocation and cancellation races.
- Review attributed participant results before claiming an outcome complete. A
  completed local run does not end the open-ended voyage or prove semantic success.

Handoff, scope-removal and shared-context contracts must be decided before their
fixtures are written. Track this evidence under #77/#78/#79/#82/#83/#22.

## Incident fallback

Stop adoption for policy bypass, credential disclosure, corrupt/lost sessions,
unconfirmed cleanup or repeated consequential failures.

1. Stop admitting new remote work and preserve command/session/audit evidence.
2. Cancel owned work and inspect actual cleanup before continuing.
3. Revoke affected remote authority and restore the previous working tool as default.
4. Preserve configuration, state, logs, versions and redacted incident evidence.
5. Verify a representative task with the fallback tool and file the incident with
   reproduction, timestamps, impact and exact source/artifact identity.
6. Re-enter only with a verified correction and a fresh observation window for the
   affected workflow. Do not downgrade writable session formats blindly.

Changing the default tool does not require deleting Helm data.
