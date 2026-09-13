# #272 X13 private terminal — integrated v2 evidence

## Scope and identity

Executed on Linux 6.18.50-2-lts x86_64, Python 3.14.7; observed completion
2026-09-13T18:22:43Z. This is a focused offline Linux synthetic-PTY check, not
completion of all [X13 scenarios](ux-comparison.md). No paid provider, operator
credentials, human terminal input, private human frames, Rust edits or builds.

Binaries: `/home/psi/voyage/target/debug`; manifest:
`/home/psi/voyage/target/ux272/build-manifest-v2.json`.
The manifest attributes source to `b2d935c86f8795613c946abc779da5beedbd3cc3+dirty`,
source fingerprint `f7081d0abf055198de6b3865af0bbac430b9d97f4c73f1ee86c8aa559b9d11a4`.
Hashes recorded by the journey and checked again after execution:

| Binary | SHA-256 |
|---|---|
| helm | `a376657a42160f7ef213e2a064a924c1217ee51412834b42bb6e0702afe46f78` |
| vessel | `42313601f087cbf91c512c613e4a6f05c06d703e491dbd96537479f43372e8ce` |
| voyage | `9bdd7c4483efbeadb32e3731b7efda3ff8e3919bf259105ddbfbd84da0ebd5cf` |

## Harness and invocation

Narrowly adapted [private_terminal.py](../../helm/tests/private_terminal.py):
synthetic named connection/account and host default using the working
`voyage/tests/provider_attempts.py` setup; versioned launch envelope and explicit
`start_settings` binding replace obsolete `start_configured`. HOME and all XDG
directories are isolated. The only API key is a synthetic loopback fixture value.
Added an explicit private Ctrl+C byte-forwarding assertion and retained the exact
child cleanup identity in the lifecycle artifact. Existing private capture,
Ctrl+], socket-loss, policy and cleanup assertions were not removed.

From the assigned worktree root:

```sh
python3 -m py_compile helm/tests/private_terminal.py
timeout 220s python3 helm/tests/private_terminal.py --bin-dir /home/psi/voyage/target/debug
```

Both exited 0. Journey output: PASS (14 grouped boundary checks).
Local synthetic evidence: `/tmp/hpt-k5dg4uhw`, notably `lifecycle.json`,
`completed.json`, `provider-requests.json`, `provider-errors.json` and synthetic
outer PTY captures. These are local verification artifacts, not human captures.
Session `4f1d644f-975c-44cb-b6b8-e18fc51962cb`, run
`7cfb4282-66a8-476f-9051-08b27d90d9a7`, terminal
`47484072-c1e1-4983-b587-d293e2596587`.

## Actual observations

- Real Helm F3 inventory → named program → Enter private attachment passed.
  Initial outer dimensions 120×36; resize 103×31 reached a 103×27 child area.
- Bold truecolor cell, hidden/visible cursor, application/normal cursor encoding,
  bracketed/unwrapped paste, and resize passed.
- Ctrl+C reached the **raw synthetic child as ETX** while Helm and child remained
  alive. This verifies forwarding, not a cooked terminal's SIGINT behavior.
- Ctrl+] detached without forwarding itself or queued private tail to child or
  composer. Ctrl+Q exited the restored TUI without cancelling independent work.
- CLI reattachment retained the private screen; SIGTERM/SIGHUP attachment exit
  restored outer terminal modes without cancelling the child.
- Stale incarnation, stale run, nonexistent terminal and read-only private-write
  requests returned errors. Only stale incarnation had `outcome_unknown=false`;
  the other three were conservatively `true`. No retry/replay was attempted and
  the fixture observed no extra child input. This is not a blanket assertion of
  definite refusal receipts.
- Injected Vessel socket loss restored CLI modes without cancelling/replaying;
  restarted Vessel observed the same independent owner. The run follower exited
  1 as expected for the injected transport loss, not as a cancelled-run claim.
- Child termination during an incomplete private paste stopped the TUI rather
  than leaking pending private input into chat and restored outer modes.
- Model reads after attachment reported unavailable capture, model writes were
  refused, and repeated private output after detach never restored model capture.
  The synthetic canary was absent from provider requests, canonical snapshot,
  fixture runtime persisted state and runtime diagnostics. Exactly five provider
  requests occurred, with no unexpected replay.
- Completion reported `session_resources=[]`, cleanup phase `observed`, pending
  cleanup `[]`. Child PID 246549, `/proc` start identity `10091638`, was no longer
  alive after `process.terminate`. Remaining fixture voyages `[]`, cleanup errors
  `[]`, provider thread stopped. No forced cleanup was reported.
- Loopback private screen samples: 80×24, 22.10 ms / 46,664 reserialized bytes;
  240×120, 284.06 ms / 692,266 bytes. These single samples are not benchmarks.

## Limits / handoff

No product failure was observed in this run. Pair/import/forget cancellation,
interrupted real account enrollment, deployed remote TLS, native macOS/Windows,
hardware terminal behavior and cooked-child SIGINT remain unexecuted. This check
uses a small outer CSI probe, not a full terminal emulator. No broad suite was
restored. No coverage run was attempted because this task explicitly freezes Rust
and prohibits builds; the harness-only delivery does not claim new Rust coverage.

The parent requested a safe pause after this successful run to rebuild Helm for an
inspection load-gate change. The v2 processes were already cleaned up. These
results apply **only to the hashes above** and do not validate that later fix or
any v3 binary. No commit or push was performed.

## Released v3 rerun

After the parent explicitly released `target/ux272/build-manifest-v3.json`, the
same unchanged harness command above ran again and exited **0, PASS**, all 14
grouped checks. Completion/identity verification recorded
2026-09-13T18:26:25.851514+00:00.
Evidence: `/tmp/hpt-m53ikwdt/lifecycle.json` and sibling synthetic artifacts.
All journey hashes matched v3 and were independently rehashed after execution:

- Helm: `a1dd62e3eaee8c6b5ae965362782cd9303d687805be4f940f45c0fa085c77cf6`.
- Vessel and Voyage: unchanged from the table above.
- Source revision: `b2d935c86f8795613c946abc779da5beedbd3cc3+dirty`.
- v3 source fingerprint: `e92cadac4f9686f412263bebaadd3ef2598a04aab242cc594237442b36e80f1c`.

Session `2bc5c47b-ac45-4b40-8dcd-a76fe1603452`, run
`74d4ae70-11ab-4297-b408-446937e06024`, terminal
`37cafce5-390a-46af-b3cd-09a41c917248`. Child PID 249784,
start identity `10110626`: no longer alive.
Cleanup phase `observed`, pending `[]`, remaining fixture voyages `[]`, cleanup
errors `[]`, provider stopped. Socket-loss follower exit 1 remains the expected
injected transport-loss observation. The same conservative terminal-error
`outcome_unknown` distinctions recorded above occurred. Screen samples were
22.45 ms (80×24) and 310.05 ms (240×120), not benchmark claims.

This rerun validates these private-boundary assertions against v3; it does not
exercise or validate the unrelated inspection load-gate fix. All X13 scope and
platform limitations above remain. See also the separate
[browser/operator evidence](ux-272-browser-operator-v2.md).
