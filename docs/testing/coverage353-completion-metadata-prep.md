# Ordinary completion metadata and input cohort (#353)

All23 new metadata parents passed at clean `beb0e01be386ae896af74c2ea97f423b24974b13` after the
retained failed `4b6b744` run (20 passed/3 failed within this family).
The five existing
completion parents remain and retain their identity, unsafe-token, draft and
render assertions. The current c14 address map has 126 candidate zero addresses
in this family; this cohort does not claim all 126 are covered or reachable.
The full established workspace denominator and 100% reachable-production goal
remain unchanged.

## Recorded production findings and repair

The [initial scope/finding](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5948176467)
precedes edits. A lookup keyed only by target/incarnation/section can accept an
earlier model reply after model → tool → model recreates that tuple. Its cached
data also lacks same-incarnation account/run/current-inference context guards.
Completion now creates a unique request UUID and captures actual snapshot model,
inference/current-inference (including full account binding) and active run ID
before the unchanged asynchronous Controls request. Installation and menu
disclosure check current selected target, available route generation,
incarnation, command section and captured context. The UUID rejects older
lookups even if a later lookup has the same tuple. Prefix edits reuse the same
current lookup, retaining bounded single-request discovery.

The [additional modal finding](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5948236555)
records that existing App input owners were absent from periodic completion
discovery checks. Active configuration drafts, account modals and inference
pickers now suppress completion. They keep their existing input and authority
boundaries. No public wire contract, grant, provider, command authority, executor
or mutation path changes. The only Update changes carry this lookup UUID and
private typed context from the producer to its existing consumer.

Independent review additionally corrected the read-only wire contract:
Controls deliberately sends no incarnation pin. The producer uses
`voyage_observed` and checks the returned owner against the captured incarnation
before retaining any inventory. A same-session/new-owner response is refused
even while the catalogue still reports the old owner. The protocol stays intact.
Wrong-session response refusal is retained as a separate case.

## Meaningful journeys

The fixture reuses the maintained authenticated loopback `loopback_tests::Peer`
and real App factory. Actual requests can be held and answered out of order;
only exact synthetic read-only Controls, initial capabilities and the existing
picker's AccountModels read are permitted. No executing Vessel/Voyage, provider,
human credential/input, browser, service or global environment is accessed.

Assertions cover:

- Exact model/tool/terminal session/unpinned-incarnation/section/run requests, deduplicated
  pending discovery, literal TestBackend rendering and Tab editing only the
  unsent draft; no execution or inferred account selection.
- Model → tool → model held replies, full account tuple changes, provider/model
  and current inference changes, active-run replacement/finish, incarnation,
  selection, unavailable route and replaced route generation. Refusal is checked
  before installation and before displaying already-loaded stale metadata.
- Existing private account/inference/configuration-draft owners through actual
  App methods/input. Old live replies do not enter their composer. Account
  capability refusal and picker metadata are real synthetic protocol reads,
  not new enrollment, grants or provider work.
- Errors with a fixed private diagnostic marker never shown or put in history,
  exact wrong-session/new-owner public envelope refusal, no automatic retry, and explicit
  erase-space/reload with one fresh read identity.
- The actual ready-transport request deadline, followed by the late original
  reply: no metadata install, callback replay or duplicate read. The peer services
  actual Ping/Pong while withholding only that reply. Protocol request expiry is
  15 seconds (observed on its regular tick), before the unchanged outer25-second
  completion bound. No paused clock, timeout widening or dead-socket send is
  substituted; this parent does not claim execution of the outer-timeout branch.
- First-256 inventory admission, unsafe token/description sanitization, malformed
  inventory, current-prefix filtering, section reuse refusal, keyboard release/
  modifiers/cycling/dismissal, remote local-path suppression, typed destructive
  confirmation, ambiguous voyage identities, advertised inference choices and
  exact current-incarnation approval/question suggestions. Tab never submits.

Every journey checks canonical message role/text/index, revision, unsent draft/
caret, pending receipts, command/first-send/route jobs and OS-browser-effect
absence. Owned fixture socket termination is observed after explicit disconnect;
it establishes that transport's retirement, not a Voyage/browser/native cleanup
claim. No additional binary, paid provider or human clipboard is introduced.

## Verified local gate

Clean measured source `beb0e01be386ae896af74c2ea97f423b24974b13` passed final formatting and strict workspace all-target/all-feature Clippy. The complete workspace coverage run passed **2,804 passed/0 failed/8 ignored** across14 Cargo targets. The corrected audit retains17 exact current objects, all542 workspace mappings plus the retained standard-library mapping, and235 preserved foreign profiles. Line coverage is 106617/134951 (79.004231%). This is execution evidence, not full100% reachable-production or native proof. See committed coverage/latest.json and the parent publication/build record.

The actual ready-transport deadline and strict late original reply case passed;
this still does not claim execution of the unchanged outer25s timeout branch.

Actual failed `4b6b744` gate retained: full2801 passed/3 failed/8 ignored across
fourteen Cargo targets; Helm821 passed/3 failed. Three fixture
premises are corrected without production changes: account capability failures
render their bounded public notice, not internal error prose; ready transport
request expiry is observed while answering real heartbeat Ping/Pong, then its
late original reply is ignored on that still-owned socket; and the direct
completion suggestion handler edits the exact draft while full App Tab retains
the draft under existing pending-decision form ownership. Each decision-kind
journey has a fresh App, so form focus is not manually erased to force insertion.
All23 corrected parents now passed in the source-final full run; failed logs
and all assertions remain retained.

The first combined `1f6d73f` gate stopped at strict Clippy before tests/profile
mutation: inline LookupContext made Update 1,336 bytes. The event now boxes that
context and the consumer moves its unchanged value into the same acceptance
checks. No lint suppression, weakened fence or passing test claim is introduced.
The original failed diagnostic remains retained; corrected strict compilation
passed at4b6b744. Its three runtime fixture failures remain as described above;
source-final corrected execution and audited full coverage passed as recorded above.

The combined completion/transcript/service cohort was measured in the sole
bounded shared-target window. Optional focused reproduction selects existing
and new parents with `process_client::ui::completion::`; the new namespace is
`process_client::ui::completion::metadata_journey_tests`. Example using the
current installed coverage command (focused summary is mixed/unpublished):

```sh
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/verification-completion/focused-summary.json -j 1 \
  -- process_client::ui::completion::
```

Source-final formatting, strict Clippy, full current-object coverage, compact
history, normal Main publication and actual hosted artifact qualification
passed as recorded here and below. Keep
failed focused/full logs and do not loosen assertions to obtain a pass.
See [quality](../quality.md) and
[the address accounting distinction](coverage353-production-address-map.md).
The exact function-group reconciliation is a separate prepared cohort recorded
in [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5948146380)
and is now integrated with the maintained accounting guard; refresh its exact
address/function-group evidence after each new measurement.

## Published source and qualified hosted artifact

Normal Main publication is **`4c330e658b1f0eb5f2822d0f8b6d747a0a5fb60a`**.
The clean measured source remains **`beb0e01be386ae896af74c2ea97f423b24974b13`**;
these identities are distinct because publication includes the result documents
and compact coverage record.

[Hosted run36990358002](https://github.com/o-psi/helm.vessel.voyage/actions/runs/36990358002)
completed **SUCCESS**, produced rather than skipped. Actual checkout log,
artifact name, public target and BUILD all identify exact published `4c330e6`.
[Public nightly1.0.3-nightly.20261002.36990358002.1](https://github.com/o-psi/helm.vessel.voyage/releases/tag/nightly-1.0.3-nightly.20261002.36990358002.1)
contains the archive and checksum. Anonymous downloads verified45,666,156bytes,
SHA256 **`f5fed123ebf2335da6a62fed89e1b9708bb72a63bc75265e395c5b76ade2336c`**,
matching checksum and GitHub asset digest. Both asset digests, exact bounded
unique/no-link/no-path-escape membership, all four executable hashes/modes and
all123 browser asset hashes (including worker/guardian) passed. Verification was
exit0/1.649s/9.6MiB/zero swap under1GiB/120s. CI remained build-only.

[Original #401 repair acceptance closed with evidence](https://github.com/o-psi/helm.vessel.voyage/issues/401#issuecomment-5949504704);
[the release checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/375#issuecomment-5949519592)
retains the remaining scopes. No `4c330e6` native installation/worker-retention
claim is made: CT106 remains on separately qualified613 while its owned browser
fixtures are pending. CT129's transitioning/PID0/unknown lifetime remains an
explicit release disposition, not readiness, never-ran or whole-cleanup proof.
#333 actual browser acceptance, #353 full100% reachable production and #375 stable
publication remain open; this documentation update requires no new Rust
measurement or duplicate hosted build.
