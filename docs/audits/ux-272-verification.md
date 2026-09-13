# #272 integrated offline verification

Status: **V3 integrated full run exited 0: all 16 prepared offline cases passed, with observed cleanup.** This is scoped partial matrix coverage, not complete X01–X16 certification.
This ledger is not an X01–X16 pass. Parent owns integration, final build, coverage,
commit/publication and issue disposition. No Cargo command is run by this worker.

## Scope and fixture repair

Parent posted the #272 scope; GitHub issue 272 was read successfully during this
repair. There is no worker authentication blocker. Own verification scripts and
this document only, in the assigned `ux272/verification` worktree.

The original runner imported a nonexistent `helm/tests/voyage_fixture.py` and
called an incompatible deleted broad fixture. That runner did not establish
readiness. The replacement loads **existing**
`voyage/tests/delivery_recovery.py` and `helm/tests/private_terminal.py` by absolute
paths derived from `Path(__file__).resolve().parents[2]` (the repository root).
No deleted fixture is restored. A narrow subclass supplies a synthetic loopback
SSE provider, configurable decision timeout and disposable inspection workspace.
Imports were actually executed against both surviving files; that is only a
preparation check, not runtime evidence.

## Reproducible integrated execution

Parent supplies a manifest AFTER its final integrated executable build:

```json
{
  "source_revision": "base commit or clean commit",
  "source_fingerprint": "explicit dirty marker plus final source digest if dirty",
  "binary_sha256": {"helm": "sha256", "vessel": "sha256", "voyage": "sha256"}
}
```

```sh
python3 helm/tests/ux272_journeys.py \
  --bin-dir /path/to/parent-released/binaries \
  --build-manifest target/ux272-build.json \
  --output target/ux272-integrated-unique-run
# --case NAME may be repeated for a focused rerun; full delivered run still needed.
```

Output must not already exist. The runner currently registers 16 focused cases (not one-to-one X01–X16).
Each case gets a fresh local supervisor, independent
voyage processes, synthetic provider and PTY; failures are retained and subsequent
cases still run. The runner checks binary hashes before and after execution and
writes per-case outcomes, cleanup errors, fixture source hashes and observations
to `result.json`. Private capture/log paths are retained locally, not published.
The surviving fixture starts with legacy synthetic tokens, but actual creation uses
`vessel auth accounts connect/add`, a synthetic environment key installed before
supervisor launch, public `account_set_default`, and account-bound `start_settings`.
This follows `voyage/tests/provider_attempts.py`; endpoints are numeric loopback
OpenAI Responses, never live authorization. Sessions enter teardown tracking only
after successful admission. Setup does not inherit the operator account.

The `recovery` case sends a submit over the existing private runtime socket and
never reads its response, then resolves the ORIGINAL command ID plus original
payload. It never resubmits. `exact-replay` is a separate intentionally adversarial
dedup test; it must not be described as ordinary recovery.

## Journey scope ledger (actual outcomes below)

| Matrix | Prepared executable coverage | Remaining / limitations |
|---|---|---|
| X01 | `readiness`, `composer`: empty account/readiness, no implicit voyage, real first multiline send | Live enrollment/model selection not authorized |
| X02 | `composer`: bracketed multiline combining/CJK/ZWJ text, select/cut/internal-yank/undo/redo, canonical exact message and single inference | Images/acquisition not covered; PTY parser not a grapheme-width oracle |
| X03 | `stop`: held incremental provider text, composer input during run | Busy steer/admission queue not covered |
| X04 | `stop`: Ctrl+C detach, still-running owner, reconnect `/stop` Esc back, reopened review and cancelled + observed cleanup | Private terminal case remains separate; Esc Stop review back verified by case assertion before confirmation |
| X05 | `stop` SSE delta; decision cases real canonical function calls | Slow tool argument interleaving/malformed calls not covered |
| X06 | `approval-approve`, `approval-escape`, `approval-expiry`: actual decision UI, file effect/no-effect, exact denial versus expiry, stale response rejection | Policy downgrade not covered |
| X07 | `question-choice`, `question-custom`, `question-back`, `question-escape`, `question-expiry`: real focus, custom multiline paste sanitization, cancellation/timeout, stale response | Skip and focus session-switch variants remain |
| X08 | `composer`: long canonical response, page/resize and no duplicate canonical history; frames saved | Exact visual anchor needs frame review; load-older/search not certified |
| X09 | `sessions`: exact two local identities, rename, observed sidebar switching, separate drafts and destinations | `historical` verifies selected first saved user boundary, distinct identity/context, source/files unchanged; same-title cross-host/archive/restore remain |
| X10 | `historical` establishes branch is not file rollback/history mutation | Destructive compact/clear/delete confirmation requires separate explicit case |
| X11 | `inspection`: dirty staged/unstaged/untracked/binary synthetic repo, real `/diff`, canonical OSC52 `/copy` cancel/confirm | Staged/untracked/binary display not all asserted; host clipboard receipt cannot be proven by OSC52 emission |
| X12 | `recovery`, separate `exact-replay`: original identity and one inference | Owner death/restart and transient provider failures not covered |
| X13 | Existing `helm/tests/private_terminal.py` is runnable separately | Do not turn its source/help into evidence; enrollment private paths excluded |
| X14 | Not applicable to synthetic provider-only runner | Explicit browser consent/private capture needs a separate local human boundary harness |
| X15 | X11 invokes actual operator command path | Workflow trust/schema/dispatch failures not covered |
| X16 | `readiness`, `composer`: 120×40/80×24/30×10, tiny input guard, resize | Linux xterm-256color only; not native macOS/Windows or color/accessibility certification |

Approval/request expiry uses the real configured tool deadline, NOT an expired
submit envelope. All assertions are provisional until execution. Failed assertions
may reveal either a product defect or a harness expectation error; retain the
failure and investigate against source instead of weakening it to a pass.

## Executed pinned competitors — 2026-09-13 17:26:26 UTC

Host Node: **26.8.2**. Each invocation ran with fresh HOME/XDG, only an allowlisted
environment, and `unshare --user --map-root-user --net`. Namespace creation passed.
No dependencies installed, auth attempted or network permitted. Raw bounded
results are retained under this worktree's ignored
`target/ux272/competitors-01/` (`result.json`, individual diagnostics and generated
component probe). Scripts hash each exercised source path.

| Pin | Actual command/path result | What it establishes |
|---|---|---|
| Pi `71dca871bc80b6bc97be37f0ca3189399d651fff` | Unmodified TypeScript `keys.ts`, `kill-ring.ts`, `undo-stack.ts` executed using native Node type stripping: **exit 0** | Ctrl+C/Escape/arrow decoding; exact combining/Chinese/ZWJ Unicode kill accumulation and yank rotation; undo snapshot independent of later mutation |
| Pi same pin | `node packages/coding-agent/dist/cli.js --help`: **exit 1**, missing distribution | Interactive startup/auth/composer **blocked**, not a Pi UX failure |
| OpenCode `95daf90670b7c039c436c85537da5fbfe2205b41` | `node packages/opencode/bin/opencode --help`: **exit 1**, CommonJS `require` in source package ES-module context | Raw pinned source launcher is not directly runnable in this Node/package context; no interactive journey evidence |
| Codex `36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` | `node codex-cli/bin/codex.js --help`: **exit 1**, missing optional `@openai/codex-linux-x64` | Built platform payload absent; no interactive journey evidence |

The Pi component result is useful X02/X04 substrate evidence, not full editor
paste/selection/rendering or application stop behavior. No comparative latency,
click counts, native-platform parity, provider correctness or full-journey pass
can be inferred from these probes.

## Historical preparation and initial execution

- Explicit-path fixture imports executed successfully; APIs reviewed against the
  surviving fixture and current parent UI/runtime source. In particular, the
  existing failure-envelope argument is `allow_error`, provider construction is
  in `Fixture.__init__`, approval defaults to Deny, and custom question input
  requires Enter after selecting Custom. The runner uses those actual APIs.
- Real loopback HTTP/SSE component probes executed for questions, write approval
  and canonical Unicode text, with fixture provider shutdown and no owned child
  processes (`/tmp/vdr-31g76djt`). No product binaries were used in that check.
- No old binary, `--help`, syntax compile or import check counts as a journey pass.
- Initial parent manifest was released and used; v3 below subsequently verifies
  both corrected product defects.
- Prior competitor component observations above belong to the earlier preparation
  ledger; this repair did not rerun them or upgrade them to full journey evidence.
- Initial case execution and failure investigation are recorded below; the final
  full v3 run supersedes the preparation-only status.

## Executed integrated evidence — first parent build

Manifest: `target/ux272/build-manifest.json`, source
`b2d935c86f8795613c946abc779da5beedbd3cc3+dirty`, fingerprint
`82430adf687cec19bc2d1053431fb3d19ec058a5169137a7d4fd3f6074916b78`.
The manifest and each result retain all three executable SHA-256 hashes. All runs
used `/home/psi/voyage/target/debug`, with no worker Cargo commands.

- `target/ux272/journeys-run1` through `run3`: retained harness failures. Old
  fixture credentials lacked a named default; old session tracking waited on failed
  creation; initial readiness label was obsolete; a synthetic environment key
  installed after supervisor spawn was absent in its child environment. Corrected
  all before passing runtime checks. These are NOT passes/product auth failures.
- `journeys-run4`: exit 0, recovery/composer/approval-approve/question-choice.
- `journeys-run5`: full 16-case run, exit 1. Twelve passed with observed cleanup;
  inspection, sessions, historical, approval-escape failed. The latter two test
  expectations/session focus were corrected, not suppressed. All five question
  cases and genuine approval expiry passed here.
- `journeys-run6`: typed approval-denied and approval-expired assertions passed;
  historical hydration and session-focus cases still failed (exit 1 overall).
- `journeys-run7`: sessions passed after Esc explicitly returns sidebar focus to
  composer; history review became reachable using Ctrl+Home, then failed an old
  expected label. `journeys-run8` retained that label failure.
- `journeys-run9`: exit 0, actual historical cutoff passed after Ctrl+Home hydration:
  only first user message in distinct child, unchanged source/files, no inference,
  observed source/child cleanup. Correct actual label is “Through saved user message”.

Product findings communicated to parent:

1. **Idle inspection shape mismatch:** `/diff` reports “Executing-host tool
   inventory unavailable”. `inspection_bridge::load` assumes `value` is an array,
   but idle Controls returns `{value:{inventory:[...],source:"builtin_preflight",…}}`.
   Request validation also needs the actual nested inventory while retaining idle
   preflight disclosure and execution-time authority. Evidence:
   `journeys-run5/inspection/ui-0.pty`, fixture `/tmp/vdr-qerf23_h`. Canonical OSC52
   copy cancel/confirm and exact decoded bytes passed before this failing step.
2. **Short-history branch hydration prerequisite:** visible complete snapshot
   history leaves `loaded_revision=None`; `/branch` refuses as loading/stale.
   Explicit Ctrl+Home hydrates and then historical branching succeeds. Source
   paths are `transcript/mod.rs::merge_snapshot`, `transcript/history.rs` and
   `sidebar.rs::prepare_branch_review`. Do not claim ordinary `/branch` passed.

No remaining owned fixture processes were observed in the completed runs. The
recorded per-case teardown verifies private stopped markers and process absence.
A new parent manifest is required after Rust corrections; full final rerun pending.

Before final rerun, the diagnostic Ctrl+Home hydration workaround was removed.
The `historical` case now requires ordinary `/branch` on complete visible history.

## V2 full execution

Command (no compile by this worker):

```sh
python3 .voyage-worktrees/ux272/verification/helm/tests/ux272_journeys.py \
  --bin-dir /home/psi/voyage/target/debug \
  --build-manifest target/ux272/build-manifest-v2.json \
  --output target/ux272/journeys-v2-full
```

**Exit 1: 15/16 cases passed-partial; inspection alone failed.** All 16 case
cleanup checks passed. Manifest hashes were unchanged before/after execution.
V2 source fingerprint: `f7081d0abf055198de6b3865af0bbac430b9d97f4c73f1ee86c8aa559b9d11a4`.
Ordinary `/branch` now passed without any Ctrl+Home workaround. The full set of
question choice/custom/back/Esc/expiry and approval approve/Esc/expiry cases passed.
Parent confirmed that V2 had normalized the panel request path but accidentally
left the loader's array-only check unchanged; inspection still fails before the
panel opens. Parent is fixing and rebuilding that product code for a final rerun.

Parent separately reports its repaired `approval_semantics.py` ten-case scoped
matrix passed with observed cleanup; this is attributed parent evidence, not a
command executed by this worker. Parent owns its exact log/manifest association.

## Final V3 integrated execution — PASS (scoped)

```sh
python3 .voyage-worktrees/ux272/verification/helm/tests/ux272_journeys.py \
  --bin-dir /home/psi/voyage/target/debug \
  --build-manifest target/ux272/build-manifest-v3.json \
  --output target/ux272/journeys-v3-full
```

**Observed exit 0; 16/16 cases passed-partial; 0 cleanup errors.**
All three binary hashes matched the supplied V3 manifest both before and after
execution. `result.json` records platform, dimensions, synthetic settings, source
fingerprint, harness/fixture hashes and per-case observations. UTC start: `2026-09-13T18:25:05Z`.
V3 measured-source fingerprint: `e92cadac4f9686f412263bebaadd3ef2598a04aab242cc594237442b36e80f1c`.

Passed cases: readiness; composer; recovery; exact-replay; stop; inspection;
sessions; historical; approval-approve; approval-escape; approval-expiry;
question-choice; question-custom; question-back; question-escape; question-expiry.

Both observed product defects were corrected by parent and verified here:
ordinary historical `/branch` succeeds without forced hydration; `/diff` opens
from actual idle preflight inventory and executes the selected unstaged diff
through Voyage. Canonical OSC52 copy is verified by decoding emitted bytes;
this does not attest that a host clipboard accepted them. The synthetic repo
contains staged/unstaged/untracked/binary files; only unstaged diff output and
whole-fixture file preservation are asserted, not every inspection scope.

The runner process exited; a subsequent process listing found no `/tmp/vdr-`
fixture processes. Each case independently checked stopped-marker identity,
observed cleanup and owned process disappearance before reporting a pass.
No Cargo command was run by this worker. Parent was notified the build slot was
free for final workspace coverage and owns final integration/publication.

### Material remaining scope

- X08 resize smoke and retained frames are not proof of exact grapheme-width
  visual anchoring. No full terminal emulator, native platform or color matrix.
- Images/acquisition, busy steering queue, malformed/interleaved slow tool calls,
  same-title cross-host switching, archive/restore and destructive history
  confirmation are not covered by these 16 focused cases.
- No live provider/authentication, private human input, browser capture/consent,
  workflow trust journey or actual host-clipboard acknowledgment was exercised.
- Earlier failure logs are preserved and never relabeled as passing. Parent's
  additional focused Rust and scoped approval results remain separate evidence.
- Do not close broad #272 acceptance solely on this scoped ledger.

Broader unfinished acceptance is retained in [#273](https://github.com/o-psi/voyage/issues/273), #255 and the [integration ledger](ux-272-delivery.md).
