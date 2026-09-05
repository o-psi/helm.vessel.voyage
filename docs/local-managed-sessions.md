# Private managed sessions

Use `helm managed` to keep a local session in a private Journal and run it from
separate CLI invocations. Helm owns the conversation, tool execution, usage, and
recovery state. These sessions are not shared with Vessel, and this command does
not start a network listener or an attachment worker.

Choose an absolute storage directory whose parent exists. It identifies a local
Helm installation, not your project directory. Different storage roots have
independent local identities. Keep the same directory when continuing a session.

```sh
STORE="$HOME/.local/share/helm-managed"
helm --workspace "$PWD" managed --directory "$STORE" create --name "Project work"
helm managed --directory "$STORE" list
```

Copy the session UUID and revision from the output, then submit a turn:

```sh
helm managed --directory "$STORE" submit SESSION_UUID --expected-revision 0 \
  "Inspect the project and explain its test setup"
```

The configured provider, model saved when the session was created, workspace,
local roots, tools, and resource limits apply. Use the normal `--config`,
`--provider`, `--workspace` and `--access` options when creating the session;
submitting uses its saved workspace/model. Required interactive approvals and
questions cannot wait for stdin in this command. Explicit configured unattended
grants still apply; otherwise an action requiring human approval is denied.

Each submit prints an admission receipt before execution, followed by provisional
model output and tool activity. Streamed text is not a completion guarantee.
The final result reports the durable run state and observed cleanup. Only an
accepted completed run with confirmed owned-resource cleanup exits successfully.
Persistent terminal processes belong to the foreground invocation; they are not
silently reconstructed by the next submit.

## Concurrent work and retries

Only one foreground owner can mutate a session. Another submit receives a busy
or revision conflict without executing tools. Other sessions can run concurrently.
Use `list --limit 20 --after SESSION_UUID` for bounded pages; metadata excludes
conversation text and provider continuation state. `--json` emits JSON records
for scripts, including an explicit `next_after` cursor.

Ordinary submit invocations represent new commands. For an exact retry, reuse the
original receipt's command UUID, expected revision, expiry timestamp and prompt:

```sh
helm managed --directory "$STORE" --json submit SESSION_UUID \
  --expected-revision REVISION --command-id COMMAND_UUID --expires-at-ms EXPIRY_MS \
  "The original prompt"
```

An identical retry observes the existing run, even after its deadline; it never
replays tools. Changing the payload under the same command ID is rejected. Create
is create-only: `create --id SESSION_UUID` never replaces an existing session.
If create output is lost, use `list` to find the session instead of blindly
creating another one.

## Cancellation and recovery

Use the session and run UUIDs from the receipt or active-run metadata:

```sh
helm managed --directory "$STORE" cancel SESSION_UUID --run RUN_UUID
```

This records a cancellation request for that exact run. It does not claim that
work has already stopped. The foreground owner observes the request and performs
cooperative cancellation and cleanup. Ctrl-C requests the same cooperative stop.
A cancellation committed before terminal persistence takes precedence over a late
model answer. A run that was already terminal is not rewritten by a late request.

After an unexpected process exit, explicitly recover the session:

```sh
helm managed --directory "$STORE" recover SESSION_UUID
```

Recovery requires exclusive ownership. It marks abandoned active execution
interrupted, preserves canonical partial output, and marks saved PTYs stale.
It does not retry tools or claim old processes survived. Inspect the outcome and
submit a deliberate new command with the current revision when appropriate.

A development Journal from an older supported schema needs an explicit upgrade
while all owners are stopped:

```sh
helm managed --directory "$STORE" upgrade
```

Existing JSON sessions stay in their current store. This command does not import,
move, expose, delete, or maintain a second writable copy of them. Keep private
backups when changing builds. Local actor UUIDs provide attribution and retry
scope; they are not login credentials or permission grants.

The required verification platform is Linux. Other platform code remains
available, but native macOS and Windows behavior is not claimed as verified for
this release. Observed owned-PTY cleanup is not an OS sandbox; a process that
escapes into a different session needs separate operating-system containment.
