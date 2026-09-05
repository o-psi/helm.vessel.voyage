# Helm cutover and rollback runbook

This runbook governs replacing an incumbent terminal LLM harness. Feature completion
alone is not approval to cut over. The operator owns the evidence record and final
decision.

## Connectivity transition

This build deliberately removes legacy pairing and task workers before implementing
attachment. The remote adoption gates and legacy drills below are **blocked**, not
passing or skippable evidence of remote readiness. Do not run the old pairing commands
against this build. Local-only trials can continue with that limitation recorded.
See [retirement, backups and rollback hazards](vessel-connectivity-retirement.md).
The remote drill checklist must be updated for the new protocol when it ships.

## Entry gates

- Linux CI is green for the exact release commit. For macOS or Windows adoption,
  record separate platform test evidence; routine CI does not test those platforms.
- The release archive checksum and `helm --version` match the proposed version.
- `python3 eval/run.py live` passes every representative workload and its JSON
  evidence is attached to the release record.
- Vessel pairing, restart/reconnect, cancellation, credential revocation, and queued
  task recovery drills pass in the intended deployment environment.
- There are no open critical security, data-loss, orphan-process, or credential bugs.
- The incumbent remains installed and its configuration is backed up.

Record the release commit, archive checksum, platform, model/provider, policy mode,
evaluation evidence location, known exceptions, approver, and UTC decision time.

## Staged adoption

1. Install the archive into a versioned directory; verify checksums before extraction.
2. Copy configuration manually. Never put provider or Vessel credentials in shell
   history, release archives, or evaluation evidence.
3. Run read-only research and inspection tasks for one day.
4. Run writing and disposable-workspace coding tasks for two days.
5. Run normal coding and administrative diagnosis with explicit approvals for five
   days. Keep a fallback counter and classify every fallback.
6. Pair one non-critical Helm with Vessel and test offline/reconnect behavior.
7. Make Helm the default only after seven consecutive days without a critical
   fallback, data loss, policy bypass, unrecovered session, or orphaned process.

During the observation window, capture task category, outcome, fallback reason,
latency, interruptions, approval surprises, and any manual repair. Secrets and raw
sensitive prompts must be redacted.

## Rollback triggers and procedure

Immediately roll back for policy bypass, destructive action without the configured
approval, credential disclosure, corrupted/lost sessions, unrecoverable Vessel queue,
or repeated orphan processes. Roll back within the same working day for provider
failure loops, broken terminal restoration, or two material task failures in one
category.

1. Stop dispatching new Vessel tasks and preserve task/audit state.
2. Cancel active work. Confirm child processes have exited before continuing.
3. Disable Helm pairing credentials and restore the incumbent as the default command.
4. Preserve Helm config, session store, logs, version, and evaluation evidence; do not
   delete or mutate incident evidence.
5. Verify one representative task through the incumbent.
6. Open an incident with impact, timestamps, reproduction, redacted evidence, and the
   exact release checksum. Re-entry requires a fixed release and a fresh observation
   window for the affected category.

Rollback changes the default harness; it does not require deleting Helm data. Session
exports remain available for deliberate migration after the incident is understood.

## Manual drill checklist

- Interrupt model generation, a filesystem operation, and a long child process.
- Close the terminal unexpectedly and verify terminal state and process cleanup.
- Restart Helm and restore, name, branch, compact, and export a session.
- Disconnect Vessel before pull, during execution, and during result upload.
- Restart Vessel with queued and running tasks; verify lease/recovery semantics.
- Revoke a Helm credential and demonstrate that heartbeats and task pulls are denied.
- Exhaust provider rate limits and verify bounded retry, cancellation, and useful error
  reporting.
- Fill the session/data filesystem and confirm atomic failure without lost prior data.
