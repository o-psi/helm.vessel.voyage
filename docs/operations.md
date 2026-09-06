# Operating the current implementation

This guide describes the **current implementation**. The current `helm` binary
contains the TUI and execution runtime; `vessel` can relay requests to an explicitly
started dedicated worker. Ordinary local chat does not require Vessel. There is no
separate `voyage` executable or background voyage supervisor on current main.
The intended process boundaries are specified in [Architecture](architecture.md).
See [Current state](current-state.md) for the implementation inventory.

Examples use installed `helm` and `vessel` commands. From a source checkout, use
`target/release/helm` and `target/release/vessel` after building them. Replace
uppercase UUID, revision, deadline and digest placeholders with actual output;
replace absolute example paths with your own existing directories. Provider
configuration is covered in [Configuration](configuration.md).

## Ordinary local work

```sh
helm --workspace /absolute/project chat
helm --workspace /absolute/project chat --plain
helm --workspace /absolute/project run "Describe this repository"
helm sessions
helm chat --resume SESSION_UUID
helm run --resume SESSION_UUID "Continue the previous work"
```

With terminal input and output, chat uses the full-screen interface. `--plain`
selects the line interface. A run executes one prompt and normally saves its session;
`run --no-save` disables that save. Resuming uses the saved session's workspace.
Model/provider overrides remain explicit invocation choices. Ordinary saved
sessions are JSON files, separate from the managed journal below.

The TUI offers `/help`, `/new`, `/resume SESSION`, `/name TITLE`, `/branch`,
and `/model`; Ctrl-Q exits from the normal input view. Session ownership can
refuse conflicting work. Branching
requires idle execution. Current TUI navigation does not establish independent
background process lifetimes: do not assume closing it detaches a surviving run.
Do not open the same saved session through competing writable copies.

## Private managed sessions

A managed directory identifies a local installation, not a workspace. Choose an
absolute directory whose parent exists and retain it for later commands.
These commands neither import ordinary JSON sessions nor publish them through Vessel.

```sh
STORE="$HOME/.local/share/helm-managed"
helm --workspace /absolute/project managed --directory "$STORE" create --name "Project work"
helm managed --directory "$STORE" list
helm managed --directory "$STORE" submit SESSION_UUID --expected-revision 0 \
  "Inspect the project and explain its build setup"
```

Create is create-only; `create --id SESSION_UUID` cannot replace an existing session.
If its response is lost, inspect `list` before creating anything else. Use
`list --limit 20 --after SESSION_UUID` for another page, or the managed `--json`
option for records with an explicit `next_after` cursor. Listing excludes transcript
and provider continuation state. Submit uses the saved workspace and model with
current local configuration and policy.

Only one foreground owner mutates a session. A conflicting owner or revision is
refused. Different workspaces can execute concurrently; sessions sharing a workspace
also contend for shared runtime resources. Managed execution supports native
providers and built-in tools. Effectful MCP configurations and `codex-compatibility`
are refused because their cleanup is not observable through this owner.
Required interactive approval cannot wait for stdin: without an explicit unattended
grant, an approval-required action is refused.

Submit prints an admission receipt before provisional output. Save that receipt.
New execution succeeds only after an accepted completed run with confirmed owned
cleanup. Streamed text alone does not establish completion. An unfinished stream
can remain provisional evidence without becoming an accepted assistant message.
Persistent terminals belong to this foreground invocation, not its next submit.

## Exact retries, cancellation and recovery

When a submit response is uncertain, preserve its original command UUID, expected
revision, deadline and prompt. Observe it by repeating the exact request:

```sh
helm managed --directory "$STORE" --json submit SESSION_UUID \
  --expected-revision REVISION --command-id COMMAND_UUID --expires-at-ms EXPIRY_MS \
  "The original prompt"
```

An identical retry observes the original run, even after the original deadline;
it never repeats tools. Changing input under the same ID is refused. Receipt
observation can return exit status zero for an active or failed run: inspect its
`existing_run` state and cleanup metadata rather than treating that exit as completion.

```sh
helm managed --directory "$STORE" cancel SESSION_UUID --run RUN_UUID
helm managed --directory "$STORE" recover SESSION_UUID
```

Cancel records intent for that exact run. Its owner cooperatively stops work and
cleans up; acknowledgment is not proof that effects stopped. Ctrl-C also requests
cooperative cancellation. Recovery requires exclusive ownership, marks abandoned
execution interrupted and saved PTYs stale, and preserves conversation/evidence.
It neither replays tools nor claims terminated processes survived.

Pending cleanup blocks new submissions and survives ordinary recovery. Independently
stop the old effects before recording an operator attestation:

```sh
helm managed --directory "$STORE" recover SESSION_UUID --acknowledge-cleanup RUN_UUID
```

An attestation differs from observed cleanup. If interrupted tool calls have no
recorded result, inspect the workspace and current revision, then reconcile:

```sh
helm managed --directory "$STORE" recover SESSION_UUID \
  --reconcile-tools RUN_UUID --expected-revision REVISION
```

Reconciliation records unknown/interrupted outcomes without repeating effects or
inventing success. A `run_unconfirmed` result preserves the actual saved state;
dropping an execution future does not prove its work stopped. Journal schema is
currently 8. Stop owners and resolve interrupted work before explicitly running
`helm managed --directory "$STORE" upgrade`. Keep private backups; never remove
cleanup or ownership records to force admission.

## Dedicated remote work

Current remote execution exposes one dedicated session per foreground Helm worker.
It is separate from the planned Vessel-supervised voyage architecture. Enrollment
and presence alone grant no execution or access to existing private sessions.
Configure a private Vessel authority directory, operator token and canonical HTTPS
origin behind a TLS proxy on the Vessel host. Supply the token through the
`VESSEL_OPERATOR_TOKEN` environment variable rather than putting it in shell history.

```sh
vessel --bind 127.0.0.1:9480 --database /absolute/vessel.db \
  --attachment-directory /absolute/vessel-authority \
  --public-origin https://vessel.example --remote-execution
helm attachment --directory /absolute/enrollment --origin https://vessel.example \
  enroll --invitation-id INVITATION_UUID
helm attachment --directory /absolute/enrollment status
helm --config /absolute/provider.toml --workspace /absolute/project remote-worker \
  --directory /absolute/remote-installation --enrollment-directory /absolute/enrollment \
  --origin https://vessel.example
```

The authenticated operator creates an invitation with
`POST /v2/enrollment/invitations`, JSON body `{"ttl_ms":300000}`,
`Content-Type: application/json` and `x-voyage-request: 2`. Use an authenticated
HTTP client that supplies the operator credential privately; do not place it in
URL parameters or command arguments. Invitation TTL must be 1–900,000 ms. The
response contains `id`, the secret `key`, and `expires_at_ms`. Distribute the ID and
one-use key through a private channel, not chat or logs. Invitation creation does
not itself enroll a machine. Current enrollment administration also supports
`POST /v2/enrollment/revoke` with `machine_id`, `expected_epoch` and a stable
`transaction_id`; revocation is an explicit exact-epoch action.

Obtain an invitation and its one-use secret from the authenticated Vessel operator.
Enter the secret only at the hidden prompt; automation uses `--invitation-key-stdin`
with private piped input. HTTPS is required outside explicitly allowed literal
loopback development origins. Helm connects outbound and needs no inbound task port.
The worker creates a new dedicated session and later reopens only its exact binding;
it cannot relabel an existing private journal. Workspace/model bindings stay fixed.
Providers, credentials, execution policy and cleanup remain on the executing host.

Authenticated operator routes are `POST /v1/remote/MACHINE_UUID/command` and
`GET /v1/remote/MACHINE_UUID/events?session_id=SESSION_UUID&after=0&limit=32`.
POST requires `x-voyage-request: 2`; supplied browser origin/site headers must pass
origin checks. The command envelope carries a UUID, expiry and operation: `list`,
`inspect`, `submit` against an expected revision, or `cancel` for an exact run.
For example, the following request body inspects an existing session; substitute
actual UUIDs and an expiry in Unix milliseconds within five minutes:

```json
{
  "command_id": "COMMAND_UUID",
  "expires_at_ms": 1234567890000,
  "operation": {"type": "inspect", "session_id": "SESSION_UUID"}
}
```

For submission, use `{"type":"submit","session_id":"SESSION_UUID",`
`"expected_revision":0,"prompt":"Describe the workspace"}` as the operation.
The numeric revision must come from the current inspection. A cancel operation
contains `type`, `session_id` and `run_id`; a list contains `type`, `after` and
`limit`. These are operation payloads, not additional enabled CLI commands.
Preserve the entire envelope on uncertain retries. Inspection returns metadata,
usage and cleanup, not canonical history or provider continuation. Replay uses its
own contiguous public cursor; `snapshot_required` means inspect and resume there.
No remote create, history, model/root editing, approval delegation or recovery API
is provided. Disconnecting an HTTP observer does not cancel execution; losing the
worker's Vessel link conservatively cancels it and stops dispatch.

## Withdraw and recover a remote grant

```sh
helm remote-consent --directory /absolute/remote-installation inspect
helm remote-consent --directory /absolute/remote-installation preview \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision GRANT_REVISION
helm remote-consent --directory /absolute/remote-installation withdraw \
  --session-id SESSION_UUID --operation-id OPERATION_UUID --expected-revision GRANT_REVISION \
  --confirm CONFIRMATION_DIGEST
helm remote-worker --directory /absolute/remote-installation --recover
```

Review the preview's destination and identity before withdrawal. Grant revision is
separate from session revision. Withdrawal permanently fences this grant's later
remote admission, reads and event publication, and requests cancellation. It cannot
recall already delivered/queued bytes. Preserve the exact withdrawal for receipt
recovery after uncertain acknowledgment. Restart cannot restore the retired grant;
future authorization needs a deliberate new dedicated installation and empty session.

Remote recovery is local and never reconnects. Its `--acknowledge-cleanup RUN_UUID`
and `--reconcile-tools RUN_UUID --expected-revision REVISION` options have the same
attestation/uncertainty meaning as managed recovery. Check pending cleanup locally
because withdrawal prevents a final public event. See [Security](security.md) for
trust boundaries and [Quality](quality.md) for the current verification status.
