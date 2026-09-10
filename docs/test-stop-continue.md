# Stop a task and continue

Focused offline Linux happy-path check for issue #246:

```sh
python3 voyage/tests/stop_continue.py --bin-dir target/debug
```

Use already built `vessel` and `voyage` binaries from the source being verified.
The test never builds Rust or contacts a real provider. It uses the standard-library
`Fixture` and bounded wait helper in `voyage/tests/delivery_recovery.py`; importing
that module does not run its regression cases.

The check starts an actual Vessel supervisor and independent voyage process in a
private temporary HOME, XDG tree and workspace, with synthetic OAuth tokens and a
loopback streaming provider. The provider holds the first response until the user
cancels. The test verifies:

- Inference has actually started and the run is running before cancellation.
- The cancelled run releases its provider connection and reaches a checkpoint with
  no pending cleanup; its public run state is `cancelled` and retained turn phase
  is `interrupted` (the runtime's distinct turn-outcome vocabulary).
- The voyage suspends, its process disappears, and its matching incarnation has
  durable `cleanup_observed` evidence before the next message is submitted.
- A fresh owner incarnation accepts the next user message and completes a new run.
  Its provider request includes both user messages and no cancelled assistant reply.
- After another clean suspension, canonical history is exactly the two user
  messages and the one successful assistant reply, with no duplicated or lost text.
  There are exactly two provider calls: one per user turn.

The fixture prints its private temporary evidence directory, retaining snapshots,
stop evidence, provider request bodies/errors, results and supervisor logs. Final
fixture teardown verifies owned voyage process disappearance and matching durable
cleanup evidence, then stops its supervisor and loopback server. It does not signal
unrelated voyages. Run without Python's `-O` option, which disables assertions.

This proves the ordinary cancel-during-inference and continue workflow, not tool
cancellation, crash recovery, races, paid-provider behavior, Helm keyboard handling,
or native macOS/Windows process cleanup. It depends on Linux `/proc` and pidfd
support, like the existing fixture. Python process checks are separate from Rust
coverage; integration must record the required coordinated workspace coverage.
