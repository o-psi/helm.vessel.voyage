# Ordinary transcript interaction cohort (#353)

Implemented and verified at `beb0e01be386ae896af74c2ea97f423b24974b13`, initially prepared at `fa66fd2` using the `c14fdbf`
uncovered-address map. [The objective record](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5947849268)
selects ordinary Helm transcript `layout.rs` (169 zero addresses), `activity.rs`
(117) and `input.rs` (89). These **375 candidate addresses** are neither a proven
reachable count nor a promised coverage gain. No denominator, source exclusion or
Root/system/adoption/platform scope is changed.

Existing helpers/small role/reflow smoke tests did not exercise this complete
sequence of maintained App/View/Snapshot display and user input behavior. The new
thirteen parents invoke production `transcript::draw` with actual ratatui
TestBackend frames and production `App::transcript_input` with keyboard/mouse/
resize events. They use the maintained private synthetic App fixture. There is no
fake renderer, executor, provider, public transport or native process owner claim.

## Prepared substantive journeys

- Canonical Unicode history: actual PageUp anchors, repeated narrow/wide reflow,
  independent appended output while reading, and explicit Ctrl+End follow.
- Loaded-display search cannot expose collapsed tool output; actual disclosure
  and search reveal only saved content without changing canonical history/draft.
- A real rendered five-action accordion is opened through its hit rectangle,
  navigated/disclosed by keyboard, and withholds process `data`/`env` even in
  expanded details. Mutable metadata remains byte-equivalent.
- Typed failed command/output-limit/timing facts and refused/unknown Vessel
  admissions remain distinct from independent completed work.
- Unconfirmed user delivery is not deduplicated against earlier text, changed
  text or mismatched parts; only its exact later canonical text/parts settle it.
- Live suffix becomes one saved answer; running/retryable/unknown cleanup states
  keep the draft and pending obligations, without claiming Ready prematurely.
- Mixed text/image metadata order, interrupted turn boundaries and authored JSON
  remain canonical content, never synthesized receipt or executor state.
- Ctrl+Home requests only earlier history; Unicode search/user jumps retain the
  exact revision and do not enqueue a mutation or replay another command.
- Provider-exposed reasoning disclosure is explicitly separate from the answer;
  finalization carries the reading position to its fresh key and collapses old
  disclosure without modifying reasoning effort or saved messages.
- Resize clears stale hits, while help/details/terminal/interaction and key-release
  fences prevent transcript controls from acting on another focused surface.
- Archived/deleted/unavailable views retain correct read-only restore/recovery
  guidance and withhold a deleted conversation's stale cached display.
- An actual selected-target switch followed by a prior transcript hit cannot
  toggle or mutate the new voyage or either preserved draft.
- Artifact metadata/partial saved details render from typed references without
  reading, downloading or executing the referenced artifact.

Assertions compare canonical Message values using boolean equality, keeping large
content graphs out of failure formatting. Exact selected session identity, drafts,
pending intent and empty route-task queues are checked where applicable. No
program or network task is launched, so no process-exit/native-cleanup success is
invented. UI cleanup notices represent observed protocol facts supplied by the
fixture; they do not independently attest runtime cleanup.

## Coordinated gate

Clean measured source `beb0e01be386ae896af74c2ea97f423b24974b13` passed final formatting and strict workspace all-target/all-feature Clippy. The complete workspace coverage run passed **2,804 passed/0 failed/8 ignored** across14 Cargo targets. The corrected audit retains17 exact current objects, all542 workspace mappings plus the retained standard-library mapping, and235 preserved foreign profiles. Line coverage is 106617/134951 (79.004231%). This is execution evidence, not full100% reachable-production or native proof. See committed coverage/latest.json and the parent publication/build record.

All13 new transcript parents executed and passed. The following optional focused
command is a reproducer; its mixed summary is not a publication measurement:

```sh
cargo llvm-cov --workspace --no-clean --json --summary-only --output-path target/verification-v103/transcript-focused-summary.json --locked -j1 -- transcript::ordinary_journey_tests
```

The full local measurement/audit above is complete. Normal publication with the
compact summary and actual hosted build/artifact follow-through remain the
parent-owned delivery stage; update that outcome only after actual qualification.
Keep prior failure evidence and foreign profiles. Source preparation or improving
a percentage does not close #353 or any native/browser acceptance obligation.
