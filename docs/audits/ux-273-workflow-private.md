# #273 — offline workflow/operator and private setup acceptance

Recorded 2026-09-13T22:13:48Z on Linux. Scope follows the parent-posted #273
acceptance, extending [the prior X15 ledger](ux-272-browser-operator-v2.md)
and [X13 private terminal evidence](ux-272-private-terminal-v2.md).
No Rust edit, Cargo invocation, commit, push, paid provider, operator credential,
or real pairing invitation was used by this worker.

## Runnable check and result

From the repository root:

```sh
python3 helm/tests/ux273_workflow.py \
  --bin-dir /home/psi/voyage/target/debug \
  --build-manifest /home/psi/voyage/target/ux273/build-manifest-v2.json \
  --output /home/psi/voyage/target/ux273/workflow-v13
```

**Exit 0; three grouped cases passed.** Final evidence is
`target/ux273/workflow-v13/result.json`; synthetic fixture evidence remains in
`/tmp/vdr-kcxo3bji` (setup), `/tmp/vdr-88rdu3lh` (forms), and
`/tmp/vdr-ih1ag440` (workflow). v12 also passed; v13 additionally verifies workflow
focus-loss discard, checks the pairing field actually contained mask characters,
and scans PTY artifacts and fixture files again after shutdown.

The harness reuses existing `ux272_journeys.Fixture`, `Journey`,
`delivery_recovery.Fixture` and `private_terminal.OuterPTY`; no deleted suite was
restored. The named synthetic account is set as the fixture host default and
launched using current `start_settings` with the fixture's versioned JSON launch
configuration. Real Vessel/Voyage owners and Helm PTYs run against an explicitly
scripted loopback Responses provider. The workflow owner explicitly uses
unrestricted local policy; routing does not override execution policy.

### Observed acceptance

- **Account setup interruption:** `vessel auth accounts setup` reads a synthetic
  key without echo; Escape and Ctrl-C both exit 0, restore original termios,
  and leave no `cancel-only` account. No provider request occurs.
- **Connection private form:** `/vessels`, Add, then Tab to the actual
  `Owner pairing invitation (masked)` field. Synthetic paste produces mask
  characters, not the value. Escape and reopen leaves that field empty. No
  connection is submitted. These are setup forms, not authentication success.
- **Live operator registry:** `/actions` → search `Run an operator tool` opens
  the live `Operator actions` panel; shell's second field identifies private
  workflow references as unavailable. Blank-command review and cancellation
  dispatch no tool or conversation message. This does not claim arbitrary
  extension-schema injection or a completed operator shell invocation.
- **Workflow trust/input:** discovered repository definition displays exact
  digest/untrusted content. Escape makes no conversation/provider effect.
  Trust allows masked private input; a missing required integer fails to advance
  into a submitted run. Escape discards the form. A separate private-editor
  journey sends terminal focus loss and observes editor removal without submission.
  Required strings allow empty strings; this test does not invent a nonempty-string
  constraint.
- **Runtime failure paths:** missing repository trust and schema version 999
  fail closed on the real observation path. Suspended observation reports only
  `suspended observation unavailable`, not a detailed validation diagnostic.
  An invalid discovered definition blocks collection until corrected/removed;
  after its removal the valid trusted preview succeeds.
- **Rejected dispatch:** a public workflow envelope using observed revision +100
  returns `result.status = rejected`, `reason = stale session revision`, with no
  history insertion. The private handle consumed by that preparation cannot be
  reused with a new command. This is not a replay of uncertain external effects.
- **Private transport and suppression:** the test explicitly supplies a fresh
  volatile handle after refusal. Accepted workflow run
  `121d7776-5e88-46f0-9b23-da41436ec5e9` in session
  `c310ba6f-7f49-46aa-995b-eacbbf233927` invokes real shell with
  `workflow_secrets: ["token"]`. The subprocess reads `HELM_WORKFLOW_TOKEN`,
  checks its synthetic length, and writes the value to stdout **and** stderr.
  Canonical tool output is exactly `{"status":"exited","code":0}`. Neither
  raw stream enters tool history or the loopback provider request. The provider
  completes the second turn; there are no provider fixture errors.
- **Cleanup:** accepted run completes with no pending cleanup. Fixture shutdown
  verifies no matching owned voyage processes remain, supervisor exit, and
  per-session `cleanup_observed: true` with the registered incarnation.
  All cases have empty cleanup-error lists. Post-shutdown scans find no synthetic
  secret in fixture files or captured PTY artifacts. These scans are not memory
  forensics or a claim about OS swap.

## Artifact identity and verification

Before and after the final run, all binaries matched parent manifest v2:

| Binary | SHA-256 |
| --- | --- |
| Helm | `2a84a4d5e5f49e04f96f1c493cb9f9d1e88d63d5a88702ec4a6e322a17c44322` |
| Vessel | `42313601f087cbf91c512c613e4a6f05c06d703e491dbd96537479f43372e8ce` |
| Voyage | `9bdd7c4483efbeadb32e3731b7efda3ff8e3919bf259105ddbfbd84da0ebd5cf` |

Manifest source: `dfdecea084aab46501c3d04e618c9940d4924fff+dirty-tests-and-preserved-user-work`;
fingerprint `964815dac62d3137b73672aeb50e32b1db3e17d6d0917c9b1f0d9fab0dab3072`.
The parent records build command `cargo build -p helm -p vessel -p voyage --locked -j 8`.
`python3 -m py_compile helm/tests/ux273_workflow.py` and worktree diff whitespace
checks passed. Parent owns issue update, final measurement, integration and publication.

## Earlier failures retained, not passing evidence

`target/ux273/workflow-v1` through `workflow-v11` retain harness development
failures: wrong Journey/PTY interfaces, launch TOML instead of versioned JSON,
misnamed command-envelope argument, incorrect screen labels/field navigation,
matching a decorative bullet as a private mask, and assuming revision 0 was stale.
The released v1 Voyage hash changed during a parent build/test; the guard refused
that execution before fixtures started and resumed against manifest v2.

In v5 the harness pressed Enter on an empty required secret **string**, which is
valid, then mistakenly pasted the canary into the subsequent **public** field.
That failed assertion is not a product secret leak. Later runs paste only while
PRIVATE is displayed and exercise invalid/missing input with an integer field.
In v9 revision 0 was current and the workflow was accepted; it is not counted as
a refusal. Final rejection uses observed revision +100 and checks `status`, not
an invented `state` field. Early operator navigation accidentally selected archive
in an isolated synthetic fixture; final navigation explicitly uses searchable
Actions. No real user session or repository was affected.

## Boundaries

This is Linux offline acceptance, not native macOS/Windows storage/terminal
verification, provider login, real remote pairing/TLS, browser navigation or full
Chromium acceptance. Account interruption covers Escape/Ctrl-C, not SIGKILL,
SIGHUP, timeout or socket loss. Connection form covers explicit cancel, not an
interrupted successful enrollment. Workflow private focus loss is executed;
account/connection terminal focus-loss policies are not generalized from it.
Existing Rust workflow, secret-binding, operator and account tests complement
these executed journeys but were not run by this worker. No full X13/X15 parity
claim or unrelated X14 browser claim is made.
