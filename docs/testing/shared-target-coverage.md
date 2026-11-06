# Current-workspace coverage objects (#251)

`packaging/coverage_artifacts.py` is an offline selection/export/audit guard, not
an alternative test runner. It never scans `target/debug/deps` for executables,
cleans caches, merges profiles, changes source or computes published percentages.
It retains **every current workspace executable**, including non-test executables
reported by Cargo, and requires a test artifact for every test-enabled member
target. Disabled test targets and custom build scripts are not required test
artifacts. Dependencies are not workspace test targets. Default features remain
Cargo's default; the manifest records resolved member features for review.

Use an exclusive shared-target window, the checkout-absolute forwarding wrapper
and the instrumentation environment from `cargo llvm-cov show-env --export-prefix`.
Do not run the selection Cargo command without that instrumentation. Preserve
foreign profiles. Select/merge only the current measurement's profile prefix;
never glob every shared profile. The ordinary full-workspace tests in `AGENTS.md`
remain mandatory. Retain the default reports as rejected evidence if their object
scan includes historical binaries; do not publish them as corrected results.

After the full test command has passed, with unchanged source and environment,
collect the complete current compiler-artifact stream (including cached artifacts):

```sh
# $evidence is a new ignored directory; $PWD is the measured checkout.
# All commands here run in the already-authorized exclusive Cargo window.
cargo metadata --no-deps --locked --format-version 1 > "$evidence/metadata.json"
cargo test --workspace --locked --no-run --message-format=json -j 2 \
  > "$evidence/artifacts.jsonl" 2> "$evidence/artifacts.stderr"
# Check the Cargo exit status before continuing.
python3 packaging/coverage_artifacts.py select --source-root "$PWD" \
  --metadata "$evidence/metadata.json" --cargo-json "$evidence/artifacts.jsonl" \
  --output "$evidence/objects.json"
```

Selection records SHA-256 for all selected executables, every Rust source file
outside target/Git metadata, Cargo manifests and lockfiles. Untracked Rust source
is included. It rejects failed/incomplete build streams, missing test targets,
missing member metadata, wrong checkout and missing executables. Preserve Git
commit/dirty state, tool versions, exact instrumentation environment (without
credentials), command and test counts alongside this evidence. A hash of source
bytes is not proof an executable was compiled from those bytes: validate compiler
instrumentation, mapping inventory and source identity before accepting caches.
The Cargo JSON and metadata must come from the same invocation scope/checkout.

Export the explicitly merged current profile over the selected objects. Supply
**exactly the existing cargo-llvm-cov default filename exclusion regex**, obtained
from that installed tool's verbose report command; retain the verbose command in
the evidence. Do not invent extra exclusions or omit inconvenient current source.
`$current_profdata` and `$default_regex` below are prerequisites, not guessed paths:

```sh
python3 packaging/coverage_artifacts.py export --source-root "$PWD" \
  --manifest "$evidence/objects.json" --llvm-cov "$LLVM_COV" \
  --profile "$current_profdata" --ignore-filename-regex "$default_regex" \
  --output "$evidence/current-export.json" --diagnostics "$evidence/export.stderr"
python3 packaging/coverage_artifacts.py audit --source-root "$PWD" \
  --manifest "$evidence/objects.json" --export "$evidence/current-export.json" \
  --diagnostics "$evidence/export.stderr" \
  --participation "$evidence/current-export.json.participation.json"
```

Export refuses to overwrite evidence and audits again after LLVM. Any LLVM
stderr, changed source/object, foreign mapping, unknown source, duplicate logical
file or empty mapping inventory is a refusal. Preserve diagnostics and investigate;
report generation is not a passed test. Summary totals come from this audited
export's `data[0].totals`, not the historical glob report. For HTML/LCOV use the
**same explicit object list, profile and exclusions**; retain and reconcile detailed
JSON/LCOV as described in `quality.md`.

The guard deliberately refuses reused foreign source paths, even byte-identical
ones. Recompile affected targets with current-checkout mapping rather than silently
normalizing another checkout's paths. Compare mapped source inventory/totals to
previous full-workspace evidence: missing cross-crate counters need investigation,
not denominator reduction. This guard cannot prove complete counters from a source
hash alone, or prove which test actually ran from a no-run stream. Retain the full
test log and verify every executed test executable belongs to `test_objects`.

## Verification and source-switch qualification

Run `python3 -m unittest discover -s packaging -p test_coverage_artifacts.py -v`.
Synthetic cases cover all-member target completeness, non-test executables, stale objects, incomplete Cargo streams, mutations, foreign
and duplicate mappings, diagnostics and checkout-switch refusal. They are not a
native Cargo/LLVM source-switch qualification.

In a serialized Cargo window, collect/instrument/test/export in main, then a clean
worktree sharing its target, then main again, with checkout-specific wrapper paths
and distinct current profile prefixes/evidence directories. Retain manifests,
Cargo logs and exports from all three. Each audit must accept only its checkout's
current source mappings, every member/test target and all executed binaries; old
manifests must reject replacement objects/source. Preserve caches and all foreign
raw profiles. Missing toolchains or coverage tools are a real gate: do not install
privileged dependencies, substitute a synthetic run or claim final-source coverage.

## Serialized host run budget

On the current 4-CPU/8-GiB host use `-j 2`, not the generic `-j 8`
example. Instrumentation can substantially enlarge object/profile/report storage.
Check free space before every cohort and after compilation; stop before exhaustion
and retain existing evidence. Do not clean another cohort to obtain room. Run one
cohort at a time with a two-hour compile/test deadline and a ten-minute LLVM export
deadline (enforced by the export guard). A timeout is failed/incomplete admission,
not passing tests; terminate the owned Cargo tree and observe cleanup before
releasing the slot. Request a new budget rather than silently shortening tests.

The installed cargo-llvm-cov 0.9.1 supports `show-env --sh`; `--export-prefix`
is a deprecated alias. Capture this environment without compiling, but set both
shared target variables and the checkout wrapper before capture: otherwise it
selects the issue checkout's own `target`, not the coordinator's shared target.
Use matching LLVM tools from the selected Rust toolchain, not distribution LLVM
merely because it exists. The 0.9.1 report implementation adds test/example/bench,
Cargo/Rust home, target/build directory, registry and rustc defaults plus any
configured vendor paths. Capture the actual verbose command after the test run;
do not hardcode a regex inferred from those categories. Its JSON export passes
one `-object` argument per object, as the guard does. Its default object discovery
scans executables and is intentionally not the authority for this shared target.

For each cohort preserve a rejected/default verbose export separately, then
explicit-selection export/audit. The full qualification remains main/worktree/main
with all workspace tests; a tiny fixture, metadata-only selection, or successful
synthetic checkout refusal does not replace those three native cohorts. Final
integrated-source coverage is separately owned by the release coordinator.

## Completeness guard revisions

Source fingerprints now conservatively cover every Git-tracked and nonignored
untracked file, including SQL, prompts, JavaScript/CSS and other embedded assets.
Inventory uses ordinary Git in normal clones and linked worktrees. In the primary
workspace with reserved `.git`, it uses the existing `scripts/local-git` wrapper
when available, otherwise explicit `.local-git/worktree.git` and checkout arguments.
It verifies the selected checkout root and never initializes, replaces or repairs
Git metadata. Missing source inputs remain a refusal, including tracked deletions.
External symlink inputs refuse. Ignored/generated inputs and external build inputs
still require native compiler/build-script provenance review; Git source hashes
alone never qualify them. All ordinary default-feature bin targets from metadata
must have non-test executable artifacts, independently of complete test artifacts.
Feature-gated binaries need an explicit matching feature policy; default scope is
not expanded implicitly. Preserve their metadata in the qualification record.

Export now runs an additional identical-profile/exclusion LLVM export for **each**
selected object. Empty mappings, diagnostics, missing object participation, or a
combined file inventory different from the union refuse publication. Per-object
exports and their stderr are retained beside the combined export; the participation
JSON is derived evidence, not a substitute for those exports. Offline audit must
supply that participation record. This proves object/file participation, not that
all cross-crate counters or ignored/generated source inputs are complete. Native
three-cohort provenance/inventory comparison remains a mandatory gate.

### Native input provenance evidence required before acceptance

For each current Cargo executable, retain its matching compiler dep-info (`.d`)
and Cargo fingerprint/build-script records in the cohort evidence. Resolve all
compiler source dependencies against the measured checkout: each must be present
in the conservative source manifest with unchanged bytes. Any ignored, generated,
external or unresolved dependency refuses qualification until its producer,
content digest and generation command/environment are captured and reviewed.
Absence of dep-info is not an empty input set. Build-script rerun-if-changed and
rerun-if-env-changed records must be reviewed too; do not publish environment
values containing credentials. Archive actual nonsecret compiler configuration,
not an assertion that source hashes suffice. This provenance review remains an
explicit native gate, not an automated claim from this offline guard.

No workspace Cargo custom-build target was observed in the no-build metadata
inspection for this source. `voyage/src/build.rs` is a Rust module, not proof of a
Cargo build script. Real inputs observed include Vessel database SQL and Helm
embedded browser files. Dependencies can still have generators. A native cohort
must validate the actual instrumented dependency graph and all current objects;
metadata alone does not establish compiled provenance or cross-crate counters.
Duplicate source files within a per-object export now refuse; repeated files
across distinct objects are expected generic instantiations, not automatically
foreign maps. Compare function/region inventories in the full union against the
current compiler set and retained prior full report, investigating counter loss.

### Read-only producer acquisition command

After the supervisor's frozen-source instrumented compilation, acquire retained
Cargo producer identity evidence without another Cargo invocation:

```sh
python3 packaging/coverage_provenance.py \
  --fingerprint-directory "$CARGO_TARGET_DIR/debug/.fingerprint" \
  --output "$evidence/fingerprint-producers.json"
```

The output is exclusively created. Unknown/ambiguous dependency fingerprint
identities and cycles refuse; marker and JSON hashes are retained and revalidated.
This reads Cargo's retained output, not a dependency cache-cleaning operation.
Scope acquisition to the actual compiler-artifact set before qualification: a
shared directory may contain historical producers with colliding identities,
which the conservative acquisition refuses rather than guessing.

The command is currently a **producer graph collector**, not complete executable
input evidence acquisition. `export --input-provenance` additionally needs an
`objects` record for each selected executable: exact object hash, matching
compiler dep-info path/hash and fingerprint path/hash. These bindings must come
from current compiler artifact/invocation evidence, not filename guesses.
Generated, ignored or external dep-info inputs still refuse. Registry lock and
configuration qualification and automatic object/producer binding remain
unfinished acceptance gates. Do not use the collector output alone to claim a
passing workspace export or native source-switch qualification.
