# Integrated #255 focused Helm stream verification

## Recorded result

`helm/tests/streaming255.py` v2 completed successfully on Linux using the
released integrated development binaries (`build-manifest-v2.json`). Four journeys passed:
local completion, local interrupted stream, scoped-loopback completion, and
scoped-loopback interrupted stream. These are actual Helm PTYs connected through
Vessel to independent Voyage processes, not screenshots rendered from invented
snapshots. The provider is a gated synthetic OpenAI Responses SSE server.

Observed in **each** journey:

- `write_file` previews and a separately collapsed **Reasoning summary** label
  appeared before provider completion and before either file existed.
- Ctrl+Shift+Down followed by Ctrl+Space expanded the supplied summary;
  Ctrl+Shift+Down/Up traversed away and back, and Ctrl+Space collapsed it
  independently. Normal 120×36 and narrow 60×32 layouts were exercised.
- Two interleaved calls retained Unicode and long arguments. Even after complete
  JSON arguments had arrived, neither effect existed while the final provider
  event was withheld.
- Ctrl+Q detached real Helm without cancelling the voyage. Another Helm attached
  and displayed the same provisional tool/reasoning data without a second provider
  request or a tool effect. PageUp was needed to reach the reasoning header above
  the long previews; absence from the bottom viewport was not data loss.
- Completion produced exactly two canonical calls and both full expected Unicode
  file contents, with no remaining provisional tool records.
- Actual EOF before `response.completed` produced a failed run, no file effects,
  and an **unfinished disclosure** label rather than a completed action.
- Exact fixture-owned Helm, gateway, Vessel and Voyage cleanup was observed.

The scoped route used a real public HTTP gateway and session-bound credential,
including scoped account-use rights. It was **loopback remote semantics**, not a
separately hosted Vessel, deployed TLS proxy or public-network test. Synthetic
account/default enrollment reuses the current `provider_attempts.py` setup rather
than relying on the retired cache-only launch assumption.

## Reproduce

Use a coordinated released build; this script does not run Cargo:

```sh
python3 helm/tests/streaming255.py \
  --source-root /home/psi/voyage \
  --bin-dir /home/psi/voyage/target/debug \
  --manifest /home/psi/voyage/target/ux272/build-manifest-v2.json \
  --evidence "$PWD/target/stream255/new-run"
```

The source root supplies current focused fixture helpers. The manifest must match
all three binary hashes. The script hashes workspace Rust sources and Cargo
manifests/lockfile before and after execution and rejects source or binary changes
during a measurement. This proves a stable measurement interval; it does not
rebuild or independently prove the build owner's source-to-binary attribution.
No native-platform or live-provider conclusion follows.

Linux socket pathname limits require short private `/tmp/vdr-*` runtime roots.
Raw synthetic PTY captures, snapshots, account/grant fixtures and cleanup records
remain there; only summaries and the source hash inventory go into `--evidence`.
Do not publish raw fixture credentials or runtime databases. No real credentials,
browser session, private human frame or paid provider is used.

## Provenance and evidence

Successful measurement: `target/stream255/v2/result.json` and
`target/stream255/v2.log` in the assigned streaming worktree. The result records
`passed`, `source_unchanged` and `binaries_unchanged` as true, with no changed source
paths. Source-before hashes are retained alongside it. Fixture roots are listed in
the private result for direct evidence inspection.

Released build manifest for that measurement:

- Source attribution: `b2d935c86f8795613c946abc779da5beedbd3cc3+dirty`
- Build owner's source fingerprint:
  `f7081d0abf055198de6b3865af0bbac430b9d97f4c73f1ee86c8aa559b9d11a4`
- Helm SHA-256:
  `a376657a42160f7ef213e2a064a924c1217ee51412834b42bb6e0702afe46f78`
- Vessel SHA-256:
  `42313601f087cbf91c512c613e4a6f05c06d703e491dbd96537479f43372e8ce`
- Voyage SHA-256:
  `9bdd7c4483efbeadb32e3731b7efda3ff8e3919bf259105ddbfbd84da0ebd5cf`

Run8 passed behavioral assertions but failed its overall source-freeze check:
concurrent parent changes touched `inspection.rs`, `sidebar.rs`, and
`transcript/history_ux.rs`. Its binaries did not change. Run9 is the subsequent
successful stable interval. Earlier harness failures involved socket path length,
a rejected `/proc` path alias, required submit identity arguments, keyboard
selection direction, viewport position, and scoped account binding. They were
fixed and the actual journeys rerun, not reported as passing preparation.

The parent subsequently released v2 for unrelated inspection/branch UX fixes.
All four streaming journeys were rerun successfully against v2 with unchanged
source and binary hashes throughout. This verifies the streaming surface on v2,
not its unrelated inspection/branch fixes. The earlier run9 evidence remains
available for the prior build.

## Additional #273 acceptance execution

The extended script was executed against the parent's **ux273 v2** released
manifest, separately from the historical v2 measurement above. Nine real Helm
journeys ran in `target/ux273/stream255-extended4`: the original four local/scoped
journeys plus five new local synthetic-provider cases. All four original journeys
passed again. Both source and binaries remained unchanged during the run. The
aggregate result is **failed**, not a full #255 acceptance pass, because the
finalized reasoning keyboard anchor is lost.

| Additional case | Executed result |
| --- | --- |
| Malformed streamed JSON | PASS: invalid argument delta and final arguments; failed run, no canonical tool call, no file effect, one provider request |
| Cancellation mid-stream | PASS: cancelled run, no calls/effects, one provider request; real Helm detach/reattach did not replay |
| Explicit `response.failed` | PASS: failed run, no calls/effects, one provider request; detach/reattach did not replay |
| Configured secret split across chunks | PASS: account-secret prefix withheld before remaining chunk; full value redacted after assembly in public snapshot and Helm; cancelled with no effect/replay |
| Anthropic thinking | PASS: separate **Provider-exposed thinking** label, initially collapsed, keyboard-expandable live text; signature and redacted-thinking canaries absent from public snapshot, finalized history and provider-attempt history |
| Finalization collapse | PASS: expanded Anthropic thinking automatically collapsed on finalization |
| Finalization keyboard anchor | **FAIL**: Ctrl+Space without re-selection does not expand the finalized disclosure. Ctrl+Shift+Down to reselect it restores expansion/collapse |

The executed v2 failure identifies the cause in
`helm/src/process_client/ui/transcript/stream.rs`: finalization changes the
reasoning key's `false` suffix to `true`, while `reconcile()` maps only tool-call
anchors. A narrow fix now transfers the reasoning reading anchor to offset zero
of the finalized key without copying expansion. Its focused regression covers
summary/thinking, unrelated anchors and repeated snapshots after manual expansion.
Rustfmt and diff checks passed; parent build, Rust test/coverage and post-fix
real-PTY verification are pending. The v2 failure evidence above is unchanged.

Final test-script verification reran the five new journeys in
`target/ux273/stream255-extended5`; exit status **1** correctly reports the recorded
anchor failure. `result.json` records `source_unchanged: true`,
`binaries_unchanged: true`, and `passed: false`. These directories and `.log` files
are under the parent checkout's ignored `target/`, with source-before hashes and
per-case summaries. Private fixture roots contain PTY transcripts, snapshots and
provider-attempt evidence. All owned fixture cleanup checks passed. No credentials
or raw diagnostics are published here.

Run1 refused mismatched Voyage binary identity before launching any fixture.
Run2 had a harness error: valid streamed arguments were contradicted only in the
redundant final output; execution of those valid streamed arguments was observed.
The corrected malformed case invalidates both deltas and final output and passed
in runs3–5. Run3 first reproduced the anchor failure; run4 completed the entire
matrix and recorded it, and run5 verified the final nonzero failure exit. Earlier
evidence remains intact rather than being relabelled as passing.

Pinned ux273 v2 release for these added measurements:

- Manifest: `target/ux273/build-manifest-v2.json`
- Source: `dfdecea084aab46501c3d04e618c9940d4924fff+dirty-tests-and-preserved-user-work`
- Build-owner fingerprint: `964815dac62d3137b73672aeb50e32b1db3e17d6d0917c9b1f0d9fab0dab3072`
- Helm: `2a84a4d5e5f49e04f96f1c493cb9f9d1e88d63d5a88702ec4a6e322a17c44322`
- Vessel: `42313601f087cbf91c512c613e4a6f05c06d703e491dbd96537479f43372e8ce`
- Voyage: `9bdd7c4483efbeadb32e3731b7efda3ff8e3919bf259105ddbfbd84da0ebd5cf`

Reproduce the five added journeys (omit `--extended-only` for all nine):

```sh
python3 helm/tests/streaming255.py --extended-only \
  --source-root /home/psi/voyage --bin-dir /home/psi/voyage/target/debug \
  --manifest /home/psi/voyage/target/ux273/build-manifest-v2.json \
  --evidence /home/psi/voyage/target/ux273/stream255-next
```

The script refuses a binary/manifest mismatch. Parent builds and coverage must
remain serialized with these source/binary-frozen journeys.

## Remaining acceptance gaps

The failing reasoning keyboard anchor is an observed product gap. Fragmented call
IDs, all-tool preview parity, strict pixel/cell Unicode typography, tool-card
scroll-anchor transfer and exhaustive duplicate-card visual counts remain outside
this bounded extension. Canonical reconciliation and absence of provisional
records are asserted, not those additional UI properties. Cancellation here is
an exact runtime command with real Helm reattachment, not the Helm stop-confirmation
interaction or automatic replay after process death. Anthropic is synthetic SSE,
not a live provider/protocol conformance claim. Signature non-display assertions
cover the public snapshot/Helm/attempt history, not every private transport byte.
Unsupported-provider behavior and Pi UX research remain outside this script.
Native macOS/Windows, live providers, separate-host remote and TLS remain untested.

Python syntax compilation, CLI help, Rustfmt and `git diff --check` passed. Rust
changes are confined to `helm/src/process_client/ui/transcript/stream.rs`; no Cargo
manifest or lockfile was changed and no Cargo command was run by this worker.
Parent owns the build, coverage, integration, issue update and publication.

## Post-fix finish-up verification

The integration owner applied the narrow finalized-reasoning anchor migration and
ran `helm/tests/streaming255.py` against `target/ux273/build-manifest-v3.json`.
`target/ux273/stream255-final/result.json` records all nine journeys passing,
unchanged binaries/source during measurement and observed fixture cleanup.
The Anthropic case now expands/collapses finalized thinking at the same keyboard
anchor without reselection; final collapse remains the default. Earlier failure
records are retained. Malformed calls, cancellation, explicit provider failure,
split-secret withholding and opaque thinking/signature exclusion also passed.
