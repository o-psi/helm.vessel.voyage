# Helm enrollment lifecycle CLI (partial #10)

`helm attachment` manages a dedicated Vessel enrollment identity. It never starts
a worker, grants session sharing, imports other credentials, contacts a model or
enables remote execution. Provider login remains under `helm auth`.

Use `--directory PATH` to select an existing parent with a dedicated enrollment
child. Without it, the child is `attachment` under Helm's platform data directory.
Only new enrollment creates the default parent if necessary. Session directories
and unrelated state are never opened or transferred by these commands. Existing storage
must pass the same Unix owner/mode or native Windows owner/protected-DACL checks as
the enrollment client; unsafe storage is rejected, not silently repaired.

The operator supplies the invitation ID and its one-use secret through their
existing authenticated distribution process. The CLI never accepts the secret in
a positional argument, environment option or flag value. For example, with a
terminal, omit the stdin flag and enter the key only at the hidden prompt:

```sh
helm attachment --origin https://vessel.example enroll --invitation-id INVITATION_UUID
```

For automation, use a secret-producing program that does not print or log the key:

```sh
secret-source | helm attachment --origin https://vessel.example \
  enroll --invitation-id INVITATION_UUID --invitation-key-stdin
helm attachment status
helm attachment rotate
helm attachment revoke
```

`secret-source` is a placeholder for your chosen secret input source, not a Voyage
command. Avoid typing an invitation into a shell command/history. Stdin must be a
pipe or file when `--invitation-key-stdin` is explicit, not an interactive terminal. Exactly 43 base64url characters are
accepted, optionally followed by LF or CRLF; other whitespace, extra input and
oversized values are rejected. Reads are capped and time out after 30 seconds.
Secrets are not printed, logged or placed in session records. Error messages omit
raw HTTP responses and potentially secret-bearing invalid arguments.

Without the stdin flag, both stdin and stderr must be a terminal; redirected or
unattended invocation fails immediately without reading a pipe. The fixed hidden
prompt accepts 43 ASCII base64url characters, Enter (CR or LF), and Backspace.
Invalid characters, bracketed-paste control sequences and oversized input fail
closed. Escape, Ctrl-C and Ctrl-D cancel; the prompt times out after 30 seconds.
Unix SIGINT/SIGTERM/SIGHUP and Windows Ctrl-Break also cancel. The reader serializes
mode entry, native reads and cancellation, discards queued input before restoring
the exact saved terminal mode, and finishes restoration before returning/exiting.
Mode changes do not wait for terminal output to drain; cancellation diagnostics
are a bounded best effort so stopped terminal output cannot trap cancellation.
A bounded quiet drain catches queued paste tails, not keystrokes arriving after the
prompt has returned. No program can restore a destroyed terminal or run cleanup
after forced process termination (for example SIGKILL or TerminateProcess).

Unix uses native termios; Windows saves and restores the complete console mode and
reads console input records. See Microsoft's
[console-mode contract](https://learn.microsoft.com/en-us/windows/console/setconsolemode).
No input is routed through the model, TUI conversation, provider, or session store.

New enrollment requires `--origin`; the stored origin is then authoritative.
Omit it for existing lifecycle commands, or pass the same origin explicitly.
An attempt to change it fails while preserving the identity. HTTPS is required;
`--allow-insecure-loopback` permits only literal loopback HTTP for development and
must be supplied for each network operation on such an enrollment. Redirects,
userinfo, query strings and non-origin URL paths are rejected. Offline status
and detach can inspect an existing loopback enrollment without enabling a request.

## Status, uncertain outcomes and detach

Status prints a small JSON object containing status and, when present, origin,
machine/owner IDs, epoch, local-disable state and pending operation/transaction ID. It never emits
private keys, invitation keys or proof material. Missing state reports
`{"status":"unenrolled"}` without creating directories, files, locks or an
identity. Existing state is privately read under its existing lock without a
rewrite; an occupied lock reports Busy. Status is local evidence, not a server
liveness or revocation query.

Enroll, rotate and revoke persist their pending transaction and required key
material before sending requests. A lost response, denial, cancellation or network
failure leaves that original transaction available. Inspect status, then resume:

```sh
helm attachment resume                                      # pending enroll: hidden prompt
secret-source | helm attachment resume --invitation-key-stdin  # pending enroll: automation
helm attachment resume                                      # rotate/revoke
```

Pending enrollment requires the same invitation key; other pending operations
reject that input flag. Resume obtains a fresh challenge for the original
transaction and does not invent another identity or proposed rotation key.
Server recovery is currently bounded to five minutes; an expired recovery requires
operator repair. `rotate` and `revoke` are rejected while another mutation is
pending. Ctrl-C returns exit 130 with recovery guidance. Successful commands return
0, fixed runtime/input errors return 1, and invalid command syntax returns 2.
An interrupted request is not proof that the server did nothing.

`helm attachment detach` works offline and disables this local identity without
requesting server revocation. It retains the local tombstone and all sessions.
A redundant detach of an already confirmed revoked identity is a read-only no-op
that preserves the `revoked` status. During an uncertain mutation, detach records
`locally_disabled: true` while retaining the pending operation, original transaction
and required keys. Explicit resume reconciles the server outcome but keeps the
identity detached; it cannot silently reconnect. Confirmed revocation is reported
as `revoked`, independently of local disable. After recovering a pending enrollment
or rotation, `revoke` can still request server revocation from the detached state.
A failed private-state write reports failure; do not assume detach became durable.

The development state adds a strict local-disable field. Existing state without
this field can be read; older builds reject newly written state rather than
silently ignoring disable. Preserve private backups when changing builds; do not
remove this field or restore an active backup to work around that rejection.
Re-enrollment of a detached directory is intentionally not automatic; use an
explicit separate identity directory and operator-issued invitation when needed.

## Verification and remaining scope

The deterministic `tests/system/attachment_enrollment.py` fixture starts the real
Vessel enrollment API behind a loopback failure-injection proxy and runs the Helm
binary. It covers enroll/status, lost-response resume, key rotation, revocation,
denied and redirected requests, origin mismatch, fixed/secret-free errors, bounded
input and offline detach with preserved session data. Unix additionally interrupts
a request after the server commits; native Windows cancellation needs separate
console-control validation. Client unit tests verify inspection does not create
missing storage or rewrite existing state and rejects unsafe/missing locks.
Both CI workflow families run the CLI fixture on Linux. Native portability jobs
are not required development gates.
The dedicated `tests/system/attachment_secret_prompt.py` uses actual Unix PTYs or
an allocated native Windows console with the Helm binary. It checks nondefault
mode restoration, no echo, input bounds/editing, cancellation, empty input queues,
nonterminal refusal and real Vessel enrollment/denial/lost-response resume. Unix
also stops terminal output and verifies cancellation restores modes and clears
queued secret input even while the marker write is blocked. Windows
coverage requires a separate native run; Linux PTY results do not establish it.

The explicit [foreground presence command](attachment-presence.md) maintains the
authenticated production socket without dispatching work.
[Dedicated remote execution](remote-sessions.md) is a separate opt-in command.
Enrollment alone does not select a voyage's machine scope or make this Helm its
coordinator. The planned remote Helm interface, cross-Helm coordination, approval
dispatch and services remain required by
[the full attachment contract](attachment-production-contract.md), and #10 remains
open beyond this lifecycle interface.
