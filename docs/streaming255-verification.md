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

## Remaining acceptance gaps

This focused verification does not close all of #255. It does not cover Anthropic
thinking disclosure, a separate provider **thinking** label, opaque/signature
handling, malformed JSON, cancellation, explicit provider-error events,
fragmented call IDs, secret-redaction canaries, all-tool preview parity,
final expanded-to-collapsed transition, strict pixel/cell Unicode typography,
scroll-anchor transfer to final cards, or an exhaustive duplicate-card visual
count. Canonical reconciliation and absence of provisional records are asserted;
these should not be overstated as those additional UI checks. Unsupported-provider
behavior and Pi UX research are outside this script. Full native macOS/Windows,
real provider, separate-host remote, TLS and browser evidence remain unestablished.

Python syntax compilation and `git diff --check` passed. No Rust source, Cargo
manifest or lockfile was changed by this verification task, and no Cargo command
was run.
