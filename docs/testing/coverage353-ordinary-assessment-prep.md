# Prepared ordinary read-only system assessment (#353)

## Verified local result

Clean measured source `e10603830e17d3428f8154140ff0b983f20fd337` passed
formatting, strict workspace Clippy and the full workspace run: **2936 passed,
0 failed, 8 ignored** across 15 Cargo targets. The combined cohort has 73 new
semantic parents (18 Helm courier, 21 Supervisor status/context, 18 extension/
UserFile, 16 ordinary assessment), plus three inert default child entries.
Nested child summaries are retained separately and are not added to parent totals.

The audited 18 current objects retain all prior production mappings. Corrected
production coverage is **108676/135727 lines (80.069551%)**, +1453 covered lines
and +752 measured lines versus `1d49636`. All 235 foreign profiles are unchanged;
511 current profiles, summary/detail/HTML and full JSON/LCOV accounting agree.
The full reachable-production target remains unmet.

The earlier `43901f9` run also passed 2936/0/8, but its report counted two new
test-only files whose names lacked the standard `_tests.rs` suffix. That report
is preserved and rejected. The filenames and path declarations were corrected,
and the complete measurement repeated using unchanged standard exclusions.
No production source was excluded and no totals were manually subtracted.

Pre-test failures and their logs are retained: error-propagation syntax, an
unavailable `tempfile` reference replaced by the maintained installer fixture,
and an inventory type alias required by Clippy. Assertions and production
refusal behavior were preserved.

This is locally verified source. Normal Main publication and an actual hosted
build/archive follow-through remain pending. It establishes none of
the separate browser, native platform, Root installation or stable release gates.

This cohort was originally prepared without compilation or execution on
`coverage353-ordinary-assessment`, based on
`c3f997f8ac1d1d1c821cb7a6a5b05f84f2ca795e`. The
[objective and boundaries](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5951565846)
were recorded before edits. Preparation made no Cargo/test/target/profile/main
mutation or production host operation.

## Current entry point and retained scope

Linux installer `main::run` dispatches `system-assess` before installation,
adoption or the interactive wizard. It requires explicit distinct account names;
no login/SUDO identity supplies CLI authority. `system_preflight::assess` performs
NSS metadata, bounded `/proc` reads, filesystem metadata/statvfs and fixed bounded
`systemctl --system --no-pager show` observations. It never starts, enables,
restarts, adopts, installs or writes a service. Its report always retains
`ready_to_install=false` and the incomplete-qualification/review blockers.

The retained audited BEB LCOV has 274 zero-execution candidate input addresses in
`installer/src/system_preflight.rs`. These are selection inputs, not a proved
reachable count or promised gain. No source, denominator or deferred
Root/system/adoption/platform exclusion changes. The existing three tests cover
basic argument/protected-path/map helpers; the new sixteen semantic parents and
one inert explicitly selected self-BIN child cover the whole ordinary diagnostic
and its observation/refusal contract.

Only `system_preflight.rs` gains a test declaration, alongside new
`system_preflight_owned_assessment_tests.rs` and this runbook. No release binary
or browser-asset validation exists in this module; the cohort does not invent or
invoke that separate responsibility.

## Actual assertions

| Family | Observation and refusal oracle |
| --- | --- |
| Whole actual CLI | An env-cleared, current installer BIN test child takes explicit `system-assess` arguments, returns without entering the wizard and prints parseable actual JSON with false installation readiness. Private fixture inventory and its existing startup lease remain unchanged/free. |
| Whole assessment | Ordinary caller + explicit Root account produces a Root-account refusal, not Root eligibility. Each failed/unavailable probe has its corresponding retained blocker. The reversed account selection reports actual ordinary execution metadata while remaining unready; installation-presence flags are metadata only. |
| Explicit arguments and NSS | Malformed/reordered/missing/same-account requests refuse before assessment. Current nonsecret NSS UID/GID/home/group metadata matches the ordinary runner; Root and embedded-NUL requests refuse. No account is created or changed. |
| Kernel observation | Actual bounded UID/GID map and CapEff observations agree with current kernel representations. A full mapping observation is not Root authority, namespace activation or permission to install. |
| Owned bounded probes | Exact UTF-8 file reads trim only the observation. Oversize, invalid UTF-8, missing and directory sources remain unavailable without rewriting the private fixture. |
| Protected path and mount inventory | Missing descendants report absent and use their existing ancestor's actual statvfs result without creating a root. Exact file/directory expectations, independent writable modes, hardlinks and symlinks refuse without chmod, unlink, replacement or content changes. Ordinary-owned fixture paths cannot masquerade as Root-controlled inventory. |
| Home/service/error reporting | Home shape checks do not follow a linked home or initialize missing state. Fixed service metadata is either bounded observed output or retained unavailable evidence. Probe errors cannot invoke a success predicate or lose their requirement/blocker. |

Private inventory snapshots include every owned relative path, UID/GID, mode,
inode, link count and file bytes/symlink target. They do not claim a global host
snapshot, global absence or historical process cleanup. The source reads only
nonsecret account/service metadata; no credential or home contents are loaded by
assessment. Fixtures contain synthetic bytes only.

## Host observation and process limits

These are ordinary-user Linux tests. The whole assessment/current-identity cases
refuse a Root test runner rather than manufacture a passing privileged fixture.
They do not change UID/GID, accounts, namespace maps, mount policy or permissions
outside owned temporary fixtures.

`service_state` calls the real bounded command runner directly; the installer
fixture's user-manager seam does **not** intercept it. Gate execution may make
actual fixed read-only systemctl SHOW queries. Actual host manager availability
is not assumed: failure remains an unavailable blocker, never a fake positive
manager or native activation claim. The production runner has a ten-second child
bound and bounded output. No command in this cohort requests a manager mutation.

The CLI parent tracks/reaps its exact current self-BIN child with a thirty-second
bound, private log and null stdin. Its environment is cleared except explicit
private HOME/XDG paths, PATH and the retained LLVM profile pattern. A timeout
kills/reaps the owned direct child and fails qualification. No extra normal
binary/object, provider, real human credential, native production acceptance or
comprehensive historical descendant proof is introduced.

## One coordinated gate

Integrate only after independent source review, alongside the complete extension
and courier source cohorts. Run the exact filter in the gate owner's private
bounded ordinary-user unit with one compiler job and existing instrumentation:

```sh
CARGO_LLVM_COV_TARGET_DIR="$PWD/target" \
LLVM_COV=/usr/bin/llvm-cov LLVM_PROFDATA=/usr/bin/llvm-profdata \
RUSTC_WORKSPACE_WRAPPER="$PWD/packaging/rustc-workspace-forward" \
RUST_TEST_THREADS=2 \
cargo llvm-cov --workspace --locked --no-clean --json --summary-only \
  --output-path target/coverage-report/ordinary-assessment-focused-summary.json -j1 \
  -- system_preflight::owned_assessment_tests
```

The focused summary remains ignored/unpublished and can include retained mixed
objects. Do not combine `--no-clean` and `--no-report`. Resolve concrete failures,
then perform required formatting/strict Clippy and fresh full workspace coverage
after the final relevant edit. Preserve previous/foreign evidence, audit all
current object/source attribution and publish the compact summary with normal
main delivery and actual hosted archive follow-through. This cohort does not
close #353, the milestone or deferred/native system-install acceptance.
