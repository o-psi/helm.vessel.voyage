# Function-group and source-address accounting for #353

This reconciles the previously unresolved difference in the **unchanged**
`c14fdbfbeed25a9f6cb3da83eb0f7a2cdbe6de2f` measurement. It does not measure new
execution, classify every line as reachable, change exclusions, reduce the
denominator, or satisfy the 100% reachable-production objective. The
[scope checkpoint](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5948062035)
precedes isolated accounting preparation.

## Exact reconciliation

The dataset has 134,806 summary lines, 106,083 covered and **28,723 gap units**,
across 542 workspace files and one retained standard-library file. Its LCOV
`DA` inventory has 127,743 distinct mapped file/line addresses and **26,546 zero
addresses**. Both use the same archived 17 current objects, 440 own profiles,
unchanged dependency/test-source filters and preserved 235 foreign profiles.
Neither inventory replaces the other.

| Contribution | Gap units | Meaning |
| --- | ---: | --- |
| Instantiation-group maxima versus address union | +101 | A group uses the maximum mapped count and maximum covered count of any one compiled instantiation. The covered-address union can cover more addresses. |
| Overlapping function-group gaps | +2,103 | Different groups map the same address, including closures inside another function. A zero group can share its address with an executed group. Summary gaps retain each group. |
| File-combined counter shadowing | −27 | The file view has 27 zero addresses whose per-function union has a positive counter. An enclosing positive function does not establish execution of its narrower zero region. |
| **Total difference** | **+2,177** | **28,723 − 26,546 = 101 + 2,103 − 27** |

Merging instantiation addresses within each group gives **28,622** zero line
units. Union across groups gives **26,519** zero addresses. The file view adds
27 narrower zero addresses, giving **26,546**. The guard reconstructs **all
12,285 function groups and all 543 file summaries exactly**, checking each
file's equation. It also matches every JSON file-segment counter to every LCOV
`DA` counter exactly, not just covered/zero status. No residual accounting
difference remains. Reconciliation
does not fulfill behavioral coverage.

### Exact witnesses

- `helm/src/process_client/ui/previews/decode.rs`, definition **13:1**: seven
  `thumbnail` instantiations each map 58 lines; maximum covered count is 51.
  Their address union covers all 58, contributing seven summary gap units with
  no zero union address. These include production backend and test callbacks;
  decoding/cancellation assertions still need separate review.
- `vessel/src/process/access/connection.rs`, definition **85:24**: two
  `connected_with_authority` closure instantiations map 283 lines and cover
  139/37. Their union reduces this group's gap by seven. This does not establish
  every current-authority behavior or every compiled instantiation.
- `crates/voyage-protocol/src/host_browser.rs` **191** belongs to zero groups
  starting at **145:5** and **191:46**: two group gaps, one zero address.
  `crates/voyage-storage/src/descendant_cleanup.rs` **64–66** similarly have
  zero enclosing/closure groups. Duplication is not unreachability or cleanup.
- `helm/src/main.rs` **309–310, 329–333** show counter shadowing: the enclosing
  group at **22:7** has count 137 while the async group at **22:31** is zero.
  The file view selects zero. These addresses remain in the zero inventory.

Ignored output retains every contributing address, function symbol, definition,
per-instantiation zero-line list and per-file subtotal. Raw exports stay private
under ignored output; no runtime credentials or conversation payloads are used.

## LLVM counting contract

The measured LLVM **22.1.8** implementation establishes:

- [CoverageReport.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/tools/llvm-cov/CoverageReport.cpp#L414)
  adds instantiation-group summaries to produce file totals.
- [CoverageSummaryInfo.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/tools/llvm-cov/CoverageSummaryInfo.cpp#L106)
  merges function summaries; the
  [line merge](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/tools/llvm-cov/CoverageSummaryInfo.h#L78)
  takes independent maxima rather than the executed-address union.
- [CoverageMapping.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/lib/ProfileData/Coverage/CoverageMapping.cpp#L1068)
  groups functions by first mapped line/column and builds file segments from
  all functions, allowing the file view and per-function union to differ.
- [CoverageExporterLcov.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/tools/llvm-cov/CoverageExporterLcov.cpp#L78)
  emits `DA` from the combined file view, while `LF`/`LH` use summary totals.
  Counting `DA` entries need not equal `LF`.

## Repeatable guard

Use [the accounting guard](../../packaging/reconcile_coverage_lines.py) only on
previously audited, matching detailed JSON/LCOV. It invokes neither LLVM nor
Cargo and changes no source, profile or filter. It supports the observed
single-file, nonempty, code-region-only Rust mapping, refusing other kinds or
expansions. Each file must match LLVM summary and exact LCOV counters/inventory. Never
discard a file or accept partial reconstruction to make the equation work.

```sh
python3 -I -B packaging/reconcile_coverage_lines.py \
  --detailed /absolute/retained-report/detailed-current.json \
  --lcov /absolute/retained-map/audited-production.lcov \
  --source-root /absolute/measured-checkout \
  --output target/coverage-accounting/new-evidence.json
```

Existing evidence is never overwritten. Actual accounting run in an independent
user unit: **exit 0, 5.624 seconds, 280.5 MiB peak, zero swap**; limits 1 GiB,
no swap, 60 seconds. Only exports were read; shared target/live objects were
untouched. Input SHA256:

- JSON: `5e2f7acfcd5c19673ddfe9718f82c240b8d219423e419e501125809e5401612e`
- LCOV: `cc2017f7b7ea4212c88593c0073f6a6a77c0ea0c475473a997f194dd70510f52`

Retain original object/profile/source audit with these exports. Refresh address
and function-group evidence after the next measured Rust edit. This Python/docs
accounting delivery needs actual guard execution, source/command/path/link/diff
validation; it requires no new Rust measurement or hosted build.

## Substantive next ordinary families

Counts are candidate zero addresses, not proven reachable totals. File-prefix
classifications still need guards: `installer/src/remote/system.rs` has 460 zero
addresses but `root()` at 59 requires real/effective UID zero before system
effects. Its positive transaction is parked system-native scope; pure record/
protocol/refusal arms are ordinary candidates. Do not call all 460 ordinary or
remove the module from the denominator.

| Family and entry | Candidate addresses | Meaningful missing behavior to verify |
| --- | ---: | --- |
| Ordinary user updater `installer/src/remote/linux.rs::run` (1210), independent worker and reconcile | 422 | Separate operation/phase/worker lifetime, coordinator versus worker lease, changed gateway/activation/manifest refusal, exact unknown receipt retention, cleanup after observed exit. Most gaps are legacy bootstrap/apply; reconcile existing private/native recovery evidence before duplicating completed work. Test manager seams do not prove native independent updater death. |
| Connected Helm `ui/completion.rs::sync_completion` (52), update (129), menu/keyboard | 126 | Actual loopback Controls success/error/timeout; exact target/incarnation/section and active run ID; late foreign reply refusal; one request on unchanged metadata; draft/caret preserved with no menu-triggered command; remote local-path refusal. Five existing completion cases already cover basic navigation, context hiding, token safety, choices and path filtering; extend async/current-mode behavior. |
| Transcript `ui/transcript/layout.rs::build` (123), draw (509) | 169 | Actual TestBackend render with complete canonical text/order/indices, archived/recovering/error disclosure, revision-selected history, image MIME/parts, interrupted/action/final labels, clipping/selection/hit rectangles. Reuse existing render/viewport fixtures; no fabricated hits, restoration or replay effects. |
| Voyage `server/authorization.rs::authorize_parts`, UserFile authority | 126 mixed | Private grant ID/revision/principal/session/workspace, expiry/revocation/permission, connection/parent attenuation, account drift, queued dispatch recheck, exact receipt/no-effect assertions. Reuse existing account/participant/current-grant cohorts. Broker-positive protected peer authority stays distinct; no real-host grant or permission expansion. |

Completion plus transcript is a substantial ordinary presentation boundary with
no native/provider authority requirement. Updater/runtime authority are connected
execution surfaces requiring owner coordination and accepted/refused/unknown/
cleanup assertions. This note implements no runtime test and claims no new
passing behavior. Consult the [reachability map](coverage353-reachability-map.md),
[address map](coverage353-production-address-map.md) and current #353 owner record
before assigning overlapping source.

## Subsequent native local source

The same guard also passed the clean b315144 detailed export and matching LCOV:
543 files, 12,290 groups, 28,722 aggregate gaps and 26,551 zero addresses.
The entire 2,171 difference is **95 + 2,103 − 27**. All exact file summaries
and every file-segment/address counter agree; no scope changed. The retained
read-only run exited 0 in 5.469 seconds with 279.9 MiB peak and zero swap.
This refresh is accounting, not additional executed or reachable lines.
