# Security, approvals, and operations

## Access modes

Set `access = "approval"` in configuration or pass
`--access read-only|approval|unrestricted` for a single invocation:

- `read-only` allows file, directory, search, todo-list, and terminal-output inspection. It
  rejects shell commands, file writes, persistent-terminal mutation, mutating todo actions,
  worktree changes, and unknown external MCP tools.
- `approval` allows ordinary inspection without prompts, asks before every file write and before
  commands classified as consequential, and keeps explicit denials in force.
- `unrestricted` does not ask for tool approval. Workspace read/write roots and `deny_commands`
  remain enforced; this mode is not an OS sandbox bypass.

Subagents inherit the same Helm configuration and therefore cannot use commands or writes when
the parent is read-only. Unattended execution cannot display an approval prompt, so actions
that require one follow `unattended_approval`. Dedicated
[remote workers](remote-sessions.md) use unattended execution and retain the
executing Helm's local policy. Enrollment and presence alone grant no execution
or session-sharing authority.

## Voyage authority and operator interfaces

Every Helm session is a [voyage](voyages.md). Planned multi-Helm participation
separates the interface Helm, coordinating Helm and executing participants.
Selecting machines for a voyage limits where coordination may send work; it does not grant permissions on those machines.
A coordinating Helm may run remotely and must remain subject to every executing
Helm's local roots, policy, approvals and credentials. Viewing or coordinating a
voyage does not imply approval authority. Remote approval routing and scoped
operator capabilities remain unfinished.

Today, the dedicated worker API supports an enrolled owner and explicitly started
worker. The authenticated `/ui` route is a status page, not the planned Helm
operator interface or a multi-Helm voyage console. Disconnecting an operator HTTP
request does not cancel its accepted work; losing the worker's Vessel transport
conservatively cancels its current turn. A durable remote voyage coordinator and
interface reconnection must not be inferred from this existing worker behavior.

## Execution boundary

Helm confines filesystem tools to canonical readable and writable roots. Existing
symlinks are resolved before authorization. New paths are anchored to their nearest
existing canonical ancestor. Command denial scans every parsed word, preventing a
denied executable from being hidden behind a command chain. Unparseable commands are
denied. These controls complement rather than replace an OS sandbox.

Tool subprocesses start with an empty environment. `inherit_env` is an explicit
allowlist; names resembling keys, tokens, secrets, passwords, or credentials are
rejected. Values in `[env]`, MCP environment values, provider credentials, and
`redact_values` are removed from tool results, errors, displayed tool arguments, and
approval targets. Values shorter than four characters are not treated as secrets to
avoid corrupting ordinary output. Do not place secrets directly in prompts.

## Attended and unattended approvals

Interactive chat and local `run` are attended. An `always` or `on-risk` policy may
display an approval prompt. Each request carries a unique approval ID, execution ID,
action, redacted target, reason, and interaction mode; the outcome is emitted as a
structured tracing event.

Unattended execution never waits for terminal input. By default,
`unattended_approval = "deny"`: a tool requiring approval immediately returns an
unavailable/denied result and the agent can report the blocked action. Operators may
set it to `allow` only when prompts, workspace, and OS identity form a trusted
automation boundary. `approval = "never"` bypasses questions for both modes but does
not bypass roots or the deny list.

## Logs, health, and diagnostics

Use `--log-format json` with either binary for collector-friendly structured logs.
Approval and tool events carry correlation identifiers. Vessel accepts a safe
`X-Request-ID` or creates one, includes it in response headers, and records it in the
request span.

- `GET /health` is process liveness and does not inspect dependencies.
- `GET /ready` verifies database access, not attachment availability.
- `GET /metrics` exposes `voyage_connectivity_enabled`: 1 when attachment presence
  is configured, otherwise 0. This is not a remote-execution readiness signal.
- `GET /v1/diagnostics` requires operator authentication and reports version,
  configured presence or managed execution separately from current connections.
- `helm doctor` prints a secret-free JSON check of configuration, workspace,
  credential presence, session location, environment allowlist, and MCP names.

Do not expose diagnostics or metrics publicly without network-layer authentication.
They contain no credentials or prompts but reveal operational topology. Retain JSON
logs according to local policy and restrict them as potentially sensitive metadata.

The Vessel status page at `/ui` and diagnostics are disabled unless
`VESSEL_OPERATOR_TOKEN` (or `--operator-token`) is set. It accepts that secret as an
HTTP Bearer token or as the password in HTTP Basic authentication. Use a randomly
generated secret, keep it out of command history by preferring the environment
variable, and terminate TLS before Vessel outside loopback. The status page never emits
the configured token into HTML, logs, diagnostics, or persisted control-plane state.

## Incident checklist

1. Capture UTC time, build version, execution/request/approval IDs, and redacted
   logs.
2. Stop new dispatch while preserving database and session files.
3. Stop affected workers and isolate suspected compromised hosts. Use the
   [enrollment lifecycle](attachment-cli.md) to request revocation; inspect and
   resume uncertain transactions instead of assuming a lost reply means success.
   Local detach disables an identity locally and does not revoke it on Vessel.
4. Confirm no managed or shell child process remains. Follow the
   [remote recovery procedure](remote-sessions.md#recover-locally-after-a-crash)
   for interrupted runs and unresolved cleanup; a cancellation receipt alone is
   not evidence that effects stopped.
5. Run `helm doctor`, Vessel `/ready`, `/metrics`, and `/v1/diagnostics` locally.
6. Preserve private evidence and follow the [operator runbook](cutover.md) for
   material failures.


## Windows enrollment storage

The dedicated enrollment client and authority use `voyage-storage` on Windows.
The [enrollment CLI](attachment-cli.md) and authenticated Vessel enrollment HTTP
service use these storage primitives; storage support alone does not establish
complete native-platform workflow validation.
Native Windows test results are required before treating the implementation as
ready for deployment; Linux tests and Windows cross-compilation are not runtime
ACL or durability evidence.

Supported directories are on local fixed NTFS volumes with persistent ACLs.
Network shares, other filesystems, device/alternate-stream paths, and reparse
points (including junctions in ancestors) fail closed. Existing stores must be
owned by the process account, with no grants to other principals. The directory
must have a protected DACL granting that account full control and inheriting those
rights into files and subdirectories. Unknown ACE forms and unverified permissions
are rejected. No existing ACL is silently repaired and no privileges are enabled.
Privileged administrators and malicious processes under the same account are
outside this isolation boundary.

New directories, client keys, database files, rollback journals, lock files, and
replacement files receive explicit account ownership and owner-only permissions
before any secret is written. The directory hierarchy remains held open without
delete sharing while a store is open. Files are checked through handles for their
ACL, type, reparse status and single-link identity. The client keeps a separate
exclusive process lock whose handle is not inherited by subprocesses. Vessel
continues to permit independent SQLite connections, using SQLite transaction locks
and returning busy rather than treating contention as successful enrollment.

Client replacement flushes file contents and requests same-volume write-through
publication without a copy/delete fallback. Failure stops further client writes;
operators must reopen and resume the original pending transaction rather than
inventing a replacement identity. A failed publication can have an uncertain
outcome, so the client preserves its fail-closed poisoned state. No existing key or
lock file is unlinked to work around errors.

Windows Vessel uses SQLite `synchronous=FULL`, `journal_mode=PERSIST`, and
`temp_store=MEMORY`. The rollback journal is securely precreated and retained so its
owner does not change to an elevated token's default group. Bootstrap briefly sets
SQLite exclusive locking before PERSIST because preparing the journal-mode pragma
can recover a hot journal using the initial DELETE mode. It restores normal
locking before authority transactions; independent clients remain supported. The non-Windows
rollback-journal behavior is unchanged. Persistent rollback journals can contain
sensitive previous pages and belong in the same private storage/backup boundary as
the database. Do not delete a journal to resolve an enrollment error.

Native tests cover ACL rejection, inherited permissions, junctions/hard links,
failed replacement, real subprocess lock exclusion/release, client state recovery,
and SQLite restart, contention, and abrupt subprocess termination with a verified
hot rollback journal. The crash fixture requires recovery of the original owner,
receipt, and committed state; Windows also rechecks private database/journal ACLs
and retained PERSIST mode. They do not prove survival of sudden hardware
power loss on every storage device. Windows ACL support here is limited to the
dedicated enrollment stores; the existing SessionStore and attachment-journal
Windows security prerequisites remain separate.


### Windows attachment journal storage

The attachment journal uses the same local NTFS, current-user ownership, protected
inheritable DACL, and no-reparse rules as enrollment storage. It precreates private
`journal.sqlite3` and `journal.sqlite3-journal` files, verifies any existing WAL/SHM
sidecars before SQLite opens them, and uses the same exclusive-to-PERSIST-to-normal
bootstrap to retain a private hot rollback journal. Temporary SQLite data stays in
memory. This keeps independent SQLite clients possible; it does not take a global
lifetime database lock.

Each session's execution sidecar is checked before OS locking. Database handles
and execution guards retain checked ancestor handles; a guard keeps these pins
alive even after its Journal is dropped. Native tests require directory rename
rejection until the last guard closes, real subprocess lock exclusion/release,
and restoration of committed canonical text, run state, replay and deduplication
after killing a writer with spilled pages and a verified hot journal. Existing
schema 2/3 upgrade and import provenance rules are unchanged. Unix database and
execution sidecars additionally reject hard links before and after opening.

These APIs remain local persistence foundations. They do not enable attachment
network access or automatically transfer any JSON SessionStore state. Source
session transfer remains unsupported outside Unix. Other unsupported storage
platforms fail before creating journal data. Windows tests must run natively before
this journal is ready for native Windows deployment; Linux results alone are
insufficient.
Persistent rollback journals contain prior session pages and must remain inside
the private storage and backup boundary. Do not delete a journal to recover an
error. Hardware power-loss guarantees remain limited by the storage device.
