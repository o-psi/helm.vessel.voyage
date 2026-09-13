# #272 — X14 browser / X15 operator and workflow: bounded offline v2 evidence

Recorded 2026-09-13 UTC. Scope: the `ux272/docs` checkout; no Rust/Cargo edits,
builds, commits, publication, paid provider, human data, or Chromium execution.
Existing documentation worker changes and `private_terminal.py` were untouched.
Issue acceptance was supplied by the coordinator; the parent read #272 successfully.
No issue update was performed by this worker; publication belongs to the parent.

## Result

- **X14 partial PASS:** two bounded Node source-function probes passed against the
  frozen parent helper, including consent outcomes, duplicate receipts, withheld
  duplicate content, and pre-admission refusals. This is not a browser journey or
  end-to-end Helm–Vessel transport verification.
- **X15 source-only:** inspected existing concrete operator/workflow fixtures and
  implementation boundaries. No operator or workflow execution PASS is claimed.
  Only the v2 `helm workflow --help` CLI check ran (exit 0).
- No product bug reproduced by these checks. Unexercised behavior remains unknown.
- The coordinator subsequently paused all product-binary executions for an
  inspection-only Helm rebuild. At that point this worker had **no active product
  binary processes**. No product binary was started after the pause. Nothing here
  establishes v3 validation. The coordinator's private-terminal result is not
  incorporated as independently observed evidence.

## Artifact identity

Checkout HEAD observed: `b2d935c86f8795613c946abc779da5beedbd3cc3`.
The supplied parent `target/ux272/build-manifest-v2.json` identifies
`b2d935c86f8795613c946abc779da5beedbd3cc3+dirty`, fingerprint
`f7081d0abf055198de6b3865af0bbac430b9d97f4c73f1ee86c8aa559b9d11a4`.
Before the rebuild pause, `sha256sum` of the parent `target/debug` binaries matched
all three manifest values:

| Binary | SHA-256 |
| --- | --- |
| helm | `a376657a42160f7ef213e2a064a924c1217ee51412834b42bb6e0702afe46f78` |
| vessel | `42313601f087cbf91c512c613e4a6f05c06d703e491dbd96537479f43372e8ce` |
| voyage | `9bdd7c4483efbeadb32e3731b7efda3ff8e3919bf259105ddbfbd84da0ebd5cf` |

Hash matching is identity evidence, not test coverage. The checkout's baseline
sources are not asserted to equal all dirty parent sources compiled into v2.
Node version: `v26.8.2`; Python available: `3.14.7` (no Python fixture executed).

## X14: actual commands and output

Commands ran from `/home/psi/voyage/.voyage-worktrees/ux272/docs`:

```sh
timeout 10s node /home/psi/voyage/helm/browser/outcomes-probe.mjs
timeout 10s node helm/browser/ux272-offline-boundaries.mjs /home/psi/voyage/helm/browser/helper.mjs
```

Both exited 0; exact output:

```text
PASS: local consent matrix, bounded expiry, wire reasons and unknown-state precedence
PASS: 6 duplicate receipt states with withheld content; 6 conflicting identities; 5 pre-admission refusals; no dispatch
```

The existing probe extracts actual `confirmation`, `localReason`, and `wireResult`
functions into a Node VM. It checks approved/denied/expired/invalidated/cancelled
consent, absent-controller unavailability, prompt removal, ten reason/state
combinations, identity fields, null images, and unresolved precedence. Timer expiry
is invoked through a stub callback, not measured wall-clock browser timing. Its
consent/fence wiring checks are source regex assertions, not live controller events.

The new [focused fixture](../../helm/browser/ux272-offline-boundaries.mjs) extracts
actual `action` and result mapping functions. Across queued, dispatched, unknown,
completed, cancelled-before-dispatch, and refused receipts it verifies:

- Identical duplicate returns retain request/action identity without re-entering
  authority or dispatch; completed does not imply replayed content.
- Wire output has null image/file/page/observation and fixed
  `duplicate_content_withheld` text; a synthetic receipt canary is absent.
- Changed action digests reject as `request_id_conflict` for all six states.
- Stale authority, stale capture, required binding missing, invalid action digest,
  and full queue reject before receipt creation; durable admission and dispatch
  are fail-if-called stubs.

The digest, authority, normalization, persistence and dispatch dependencies are
stubbed: this does **not** validate cryptographic hashing, successful admission,
durable replay after restart, browser authority transitions, DOM capture filtering,
upload/download consent, process cleanup or actual website side effects.
`probe.mjs` and `cleanup-probe.mjs` were deliberately not run by this worker.
The coordinator subsequently reported one parent `cleanup-probe.mjs` attempt
ending in a bounded initialization timeout and no remaining helper/Chromium
processes. That is **reported failed/blocked**, not PASS or independently observed
cleanup evidence here. Missing dependencies were suspected, not established. No
browser installation or public-web fallback was attempted.

Source identities observed before the second probe:

| Parent source | SHA-256 |
| --- | --- |
| `helm/browser/helper.mjs` | `7d57193119c46c19aa65ef8805520583ea279446f45fb056d1804cb3db7dea9a` |
| `helm/browser/outcomes-probe.mjs` | `1f0ffa03b570bffb9490c8915406e595398491a7af07462066c1e222893d8346` |

## X15: concrete existing fixtures, source review only

[ui_journeys.py](../../voyage/tests/ui_journeys.py), lines 502–536, contains an
idle F8 `read_file` journey with typed path, explicit review, canonical output,
and unchanged loopback provider request counts. Lines 576–607 contain a saved
workflow journey: repository definition review, trust action, masked optional
synthetic secret, nonsecret preview, one submission, and absence of the synthetic
secret from retained files. Those assertions were read, **not run**. This is an
older multi-journey fixture; it was not promoted to focused current UI evidence.

[operator.rs](../../voyage/src/attachment/runtime/operator.rs), `execute_operator`,
uses a durable run owner and phase-specific authored public errors, checks
cancellation between phases, redacts successful output, and avoids overwriting a
terminal outcome on later failure. This is source inspection, not cancellation,
exact-command deduplication, cleanup, or no-inference runtime proof.

[workflow.rs](../../voyage/src/workflow.rs), `prepare`, checks exact repository
trust and rechecks the selected definition digest after attended collection.
[workflow/secrets.rs](../../voyage/src/workflow/secrets.rs), `RunBindings::resolve`,
checks accepted run identity, bounded reference count, known names, and duplicate
references; its public debug representation is hidden. These source boundaries do
not establish secret non-persistence or shell output suppression by execution.

Only this bounded CLI command executed before the pause:

```sh
timeout 10s /home/psi/voyage/target/debug/helm workflow --help
```

Exit 0. It listed `list`, `inspect`, `validate`, `preview`, `run`, and `help`, with
`--user-directory`, `--json`, policy, config, provider, workspace and access options.
This did not select or run a workflow or launch a provider.

### Executable package feasibility

[executable_packages.py](../../voyage/tests/executable_packages.py) imports
`Fixture`/`wait_for` from [delivery_recovery.py](../../voyage/tests/delivery_recovery.py),
not `tests/voyage_fixture.py`. The latter remains in this older checkout but is
absent in frozen parent source; it was neither restored nor used.

The package fixture requires explicit static Go `conformance`, `read`, and
`transform` binaries in addition to Helm/Vessel/Voyage. Historical candidates were
found under the parent `target/issue-63-evidence/`, but are not pinned by v2's
manifest and were not executed. Its default sandbox-required scope would need
actual native isolation evidence. Its base fixture starts a synthetic loopback
provider, redirects HOME/XDG, and allocates evidence using `tempfile.mkdtemp`;
any future admitted execution must constrain temporary output to the permitted
checkout and verify artifact identity and cleanup. It is broader than this bounded
source review. No package install, service change, sandbox modification, or fixture
restoration was attempted.

Reviewed fixture SHA-256 values:

| Checkout fixture | SHA-256 |
| --- | --- |
| `voyage/tests/ui_journeys.py` | `ce80eba2a8e08b11d90ef2bb5b85566d6625fee8a5288afe02a14d28954da780` |
| `voyage/tests/executable_packages.py` | `c38596b4d16559916a85368721722aaeec8203d8a74dbfa651da1019ae7d62d9` |
| `voyage/tests/delivery_recovery.py` | `c6ea495d4c90b7921140731a9f1c72a0effefba53df6bbffa27a830212e62a14` |

## Remaining limitations

Full X14 browser journeys require separately admitted Chromium execution and no
human data. Full X15 needs focused current-binary operator/workflow execution,
including failure/cancellation, secret transport/privacy and cleanup observations.
Neither gate is closed by this partial offline evidence. No native macOS/Windows
claim, live-provider claim, or Rust coverage measurement is made.
