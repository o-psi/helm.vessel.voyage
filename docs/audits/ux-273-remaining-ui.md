# #273 focused remaining UI evidence

Scope: X02/X03/X09/X10, synthetic Linux controlling PTYs, disposable local
Vessel/Voyage processes, loopback Responses provider. No live providers,
credentials, desktop clipboard contents or screenshot capture. No Cargo or Rust
edits by this worker; parent owns fixes, build, coverage and publication.

## Runner

`helm/tests/ux273_remaining_ui.py` imports the surviving #272 focused fixture and
private PTY emulator. The two-route case imports only the surviving scoped
`approval_semantics.Gateway`. It does not restore deleted broad fixtures.
Named/default synthetic accounts are enrolled through the actual account CLI.
Image dispatch implements native `GET /models` metadata and uses synthetic
`gpt-4o`; the inherited `fixture-model` deliberately has no known image support.
Clipboard acquisition invokes a local deterministic `wl-paste` helper, including
failure and a held child for cancellation/reaping. It never queries a desktop.

```sh
python3 helm/tests/ux273_remaining_ui.py \
  --bin-dir /path/to/parent-released/binaries \
  --build-manifest /path/to/parent-build-manifest.json \
  --output /path/to/unique-private-evidence
# --case images / steering / routes / lifecycle may be repeated.
```

The runner verifies before/after binary hashes against the manifest and retains
PTY traces, frames, canonical evidence and observed fixture cleanup. Its inherited
`passed-partial` status means the named assertions passed, not full matrix
certification. Reports record all harness hashes and scoped limitations.

## Actual release-v2 results

Parent release: source `dfdecea084aab46501c3d04e618c9940d4924fff` plus explicit
dirty tests and preserved user work; fingerprint
`964815dac62d3137b73672aeb50e32b1db3e17d6d0917c9b1f0d9fab0dab3072`.
Manifest: `target/ux273/build-manifest-v2.json`. Private evidence remains under
`target/ux273/remaining-ui-*`; no raw traces or fixture storage are published.

| Case | Actual evidence | Result |
|---|---|---|
| X02 | Run 7 `images`: four independently owned images and authored text persisted; fifth image refused; failed acquisition retains all data; Escape cancels held helper, observed child exit, no stale insertion; one explicit submit sends four images to loopback provider and canonical user message | Passed scoped case; cleanup observed |
| X03 | Run 4 `steering`: held provider run, image-bearing steering refused and full draft retained; text-only replacement admitted while busy, visible **Steering admitted · awaiting delivery in this run** frame; after release canonical steering status `applied`, exactly one authored steering user message | Passed scoped case; cleanup observed |
| X09 | Runs 3 and 4 `routes`: two independent loopback Vessel gateways, same-title voyages, actual F2 switching between their canonical unique prompts and return to original route, separate drafts | Passed scoped case; both gateways/Vessels/providers stopped |
| X10 | Run 11 `lifecycle`: exact-source archive/restore, source draft persisted before/after; Clear cancel and stale review preserve history; reopened Clear actually removes source history without changing same-title peer; Delete cancel and stale review preserve history | Partial; final Delete rejected by product schema bug |

### Concrete Delete blocker

Run 11 final exact-source Delete receipt:

```json
{"command_id":"416e0069-0e06-4fb6-a9a3-e4f20ff11489",
 "reason":"no such table: remote_events","status":"rejected"}
```

`voyage/src/attachment/journal/deletion.rs:44` still executes
`DELETE FROM remote_events`, but the current journal has no such table. Parent
was notified immediately; no worker Rust edit was made. The durable transaction
left lifecycle `deleted=false`. This is not a successful deletion and prevents
claiming full X10 completion. Parent must fix/verify the actual schema cleanup and
release binaries, then rerun the focused lifecycle case (preferably all four).

## Harness corrections and honest limits

Earlier runs remain failed debug evidence, not passing acceptance:

- Runs 1–2 corrected assumed fixture method names and full-UUID picker search.
  F2 searches displayed title/route, not hidden UUIDs. The inherited fixture has
  `finish`/`note`, not `finished`/`observe` journey methods.
- Parent's Cargo test replaced Voyage during run 2; its after-hash assertion
  failed. Parent supplied release v2; this was not a product failure.
- Native image model metadata cannot be represented by old HTTP 501 HTML.
  An unknown model refusal and invalid-metadata run were inspected, not passed.
- Archive is asynchronous. Wait for durable archive metadata before opening the
  frozen F2 catalogue; restore receipt is not a fresh owner snapshot. Opening F9
  too early correctly creates a stale/unavailable menu. Wait for the restored
  composer and reopen reviews; never substitute a newer identity into them.
- Escape first leaves the review editor, then closes Actions. Retained drafts
  were verified both on disk and screen; apparent loss during guessed positional
  navigation was not accepted as a product defect.
- Stale reviews here cover incarnation changes from sleeping owner restart as
  well as revision changes; accepted refusal can be `Voyage restarted; reopen
  Actions`, not only `Voyage changed`.

The PTY emulator is not a native grapheme-width oracle. Real desktop clipboard,
screenshot selection, public TLS, native macOS/Windows, live provider behavior,
operator-selected capture consent and broader X01–X16 acceptance remain outside
these synthetic cases. No missing harness setup is classified as unavailable.

## Parent integration outcome

The Delete failure was fixed in `journal/deletion.rs`: scrub the retired
`remote_events` table only when present, without recreating it or swallowing other
SQL errors. Focused tests cover fresh and legacy journals and exact repeated Delete
receipt identity. On rebuilt v4 binaries, `target/ux273/remaining-ui-final` passed
steering, two-route navigation and the complete lifecycle case, including final
Delete, observed cleanup and unchanged peer history. Its image case failed before
submission with the existing safe “upload failed” result and retained full draft;
no effect was replayed. A fresh isolated `images-v4-recheck` passed all image
assertions on identical binaries. That intermittent upload failure's cause is not
established by the generic diagnostic; it is not relabeled a passing run.

Thus all four scoped cases have observed passes, not a claim that one full run
passed or that intermittent stability is resolved. Existing native, live-provider
and broader interruption limitations remain.
