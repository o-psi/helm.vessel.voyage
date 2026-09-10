# Conversation and file-editing check

This focused offline Linux check exercises a normal user task through a real
Vessel and independent voyage processes. It uses a scripted loopback OpenAI Chat
provider, not a live account.

From the repository root:

```sh
cargo build -p vessel -p voyage --locked -j 8
python3 voyage/tests/conversation_files.py --bin-dir target/debug
```

Python 3 with the standard library and Linux pidfd support are required. Run
without Python's `-O` option: the fixture uses assertions for verification.
Coordinate Cargo builds with other work sharing `target/`.

## What it checks

1. A user asks to change a shopping list. The provider receives the exact prompt
   and the native `read_file` and `apply_patch` tools.
2. The voyage reads the file and sends its content and SHA-256 back to the
   provider. The provider requests a patch using that digest.
3. The actual file contains the requested change; the surrounding lines and a
   random marker are unchanged. The assistant's final reply is saved.
4. After the voyage suspends, a follow-up receives the earlier user message,
   assistant reply, and tool results. Another native read observes the edited
   file, and a second reply is retained.
5. Retained snapshots contain both completed turns and successful tool outcomes.
   Fixture-owned voyage processes and the supervisor are stopped before success
   is reported.

The fixture creates a private temporary directory, separate HOME/XDG directories,
workspace, local provider, and Vessel. It does not inherit provider credentials.
It prints its evidence directory, retaining snapshots, synthetic provider requests,
fixture errors, and the supervisor log. Polling and HTTP calls are bounded; failures
exit nonzero. The fixture leaves evidence in place for diagnosis.

## Limits

The scripted provider makes this a repeatable check of our application, not a test
of whether a real model chooses the right tools or follows instructions. It does
not establish live provider access, Helm TUI behavior, native macOS/Windows support,
or recovery after a crash. Suspension here is normal idle suspension.

This Python process check is separate from the workspace Rust coverage percentage:
its execution is not included in the default `cargo llvm-cov` test measurement.
See [validation](quality.md) for the other focused checks and their limits.
