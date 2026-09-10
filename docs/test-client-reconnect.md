# Client disconnect and reconnect check

This focused offline Linux happy path for issue
[#248](https://github.com/o-psi/voyage/issues/248) uses **real Helm clients**, a real
Vessel supervisor, and independent voyage processes. A scripted loopback OpenAI
Chat provider holds a streaming response while the first Helm client disconnects.
It requires no provider account or paid requests.

From the repository root, with Python 3.11+ and Linux pidfd support:

```sh
cargo build -p helm -p vessel -p voyage --locked -j 8
python3 voyage/tests/client_reconnect.py --bin-dir target/debug
```

Coordinate builds with other work sharing `target/`; when suitable binaries are
already built, run only the Python command. `--bin-dir` also accepts an absolute
path. Do not use Python's `-O` option: assertions are the test oracle, and the
fixture rejects optimized execution.

## Verified lifecycle

1. Start an isolated Vessel and configured voyage. Launch `helm connect run` as a
   separate process against that session. The provider sends a unique partial
   response, then blocks on a bounded gate. Wait until the real Helm client prints
   the partial output before disconnecting it; this is not a pair of unrelated
   HTTP snapshot requests.
2. Send SIGINT to Helm. Assert a successful client exit and its explicit detach
   notice. Before releasing the provider, inspect the same session/run: it must
   still be running with the same owner incarnation and the same fixture-owned
   voyage PID. The Vessel must also still be alive.
3. Release the provider only after the first Helm process has exited. With no Helm
   client attached, the original run must complete, save the exact conversation,
   and suspend normally. Its voyage process must exit. Polling snapshots is used
   only as a test oracle; it does not submit or drive continuation work.
4. Start a new `helm connect chat` process against the **same session UUID**. It
   must render the completed retained output. Check the retained canonical user
   message and assistant reply, the original run ID, and exactly one provider
   request: reconnecting must not replay the task.
5. Send a follow-up through the reconnected Helm client. The provider verifies the
   earlier prompt and completed assistant reply in its input, then returns the
   marker. Helm must display it, and both completed turns must remain in the
   canonical conversation. Close chat stdin (EOF) and observe a successful detach.
6. Observe fixture-owned voyage cleanup and wait for the supervisor and all Helm
   clients to exit before reporting success. Provider request handlers are bounded
   and joined when the fixture shuts down.

The fixture uses a private temporary root, isolated HOME/XDG directories and
workspace, synthetic prompts, and a loopback-only provider. Its subprocess
environment does not inherit provider credentials. HTTP calls, gate waits,
state polls, and process waits are bounded. Any failed assertion exits nonzero;
cleanup runs on failure too and never signals unrelated voyages.

The printed `/tmp/voyage-client-reconnect-*` directory retains snapshots before
and after detach, completed/disconnected and reconnected snapshots, the follow-up
snapshot, both clients' stdout/stderr, provider requests/errors, and the Vessel
log. `lifecycle.json` records the session/owner identity, observed PID, client exit
codes, cleanup result and SHA-256 of each tested binary. Evidence is kept for
local diagnosis rather than added to the repository.

## Limits

This uses Helm's **line/one-turn interfaces**, not its full-screen TUI. The first
client uses the normal run observer (SSE in the current local transport); the
reconnected chat client polls retained snapshots/output. It verifies a deliberate
SIGINT detach and EOF exit, not a broken network, packet loss, abrupt client kill,
SSE cursor replay, remote TLS, Vessel restart, or crash recovery. Normal completed
voyage suspension is not a claim that a dead process survived a restart.

The deterministic provider verifies application lifecycle and retained context,
not model intelligence, live provider behavior, or native macOS/Windows support.
The command tests the binaries supplied via `--bin-dir`; build them from the
intended source before claiming source-specific results. This Python process
check is separate from the workspace Rust coverage measurement. See
[validation](quality.md), [architecture](architecture.md), and
[process access](process-access.md) for related contracts and limits.
