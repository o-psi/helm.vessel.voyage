# Exact uncovered address accounting for #353

The acceptance target is **100% reachable production lines** with meaningful
assertions. The full established Rust measurement/denominator stays intact.
An aggregate coverage gap count is not automatically a count of executable,
reachable, unasserted production source addresses.

## Audited address inventory

The independently audited `cb929ff47383bb49f82e553a291f097268873eae` measurement
contains 30,186 LLVM line-gap units. Exporting LCOV from its **archived exact 17
objects and own-only profdata** yields **27,948 distinct zero-count file/line
addresses** across 541 workspace files plus the retained standard-library source.
No live Cargo object, raw profile or exclusion was changed by this read-only
export. The original JSON/HTML/summary remain authoritative for aggregate totals.

**2,238 gap units are not yet reconciled to distinct LCOV zero-count addresses.**
Macro/generic/instantiation and aggregate LLVM line accounting requires further
region-level reconciliation; this difference is not dropped, called covered or
subtracted from the 100% goal. The prior 993 and 2072 inventories are preserved separately. Refresh this map after every
subsequent audited source delivery before reporting a current address or
reachable-production count.

The full address inventory, source text, demangled function owners and selected
entry/guard proofs remain ignored under
`target/coverage-report/coverage353-production-map`: `audited-production.lcov`,
`uncovered-addresses.json`, `classification-summary.json` and exact archived-object
command. Detailed coverage/address exports are not published as the compact
coverage history. They are independently reconstructible from the coordinator's
retained source/object/own-profile audit and LLVM LCOV export.

| Address classification | Distinct zero addresses | Meaning |
| --- | ---: | --- |
| Ordinary executable candidates awaiting assertion/guard review | 16,505 | Module/entry surfaces support ordinary fixtures; not a proof that every line is reachable. |
| Native privileged/staged mixed source awaiting exact guard review | 8,246 | Root/system/identity families include ordinary validators plus protected positive effects; no blanket unreachable classification. |
| Owned native/fault fixture candidates | 1,638 | Terminal/clipboard/process/filesystem/browser/extension boundaries need actual owned resources or controlled faults. |
| Ordinary protocol fixture plus external success evidence | 753 | Provider/GitHub protocol/error logic is locally testable; authenticated external success needs separate authority/budget. |
| Ordinary serve native fixture/guard review | 219 | Actual `serve(None)`, local authenticated HTTP/stream and graceful lifetime; Root/gateway arms separately guarded. |
| Privileged UI staged positive with ordinary parser/refusal/storage | 214 | Ordinary owner transport refuses Execution; positive saved review requires protected Root identity. |
| Ordinary notification contracts/courier | 66 | Public authenticated notification operations and bounded metadata courier; no decision/run admission. |
| Proven current-protocol unreachable budget positive branch | 84 | Constant unknown typed command cannot construct a successful response; exact proof below. |
| Detected inline test/helper addresses | 222 | Demangled `tests`/`*_tests`/test-support owners; retained in the measured denominator. |
| Retained standard-library mapping | 1 | Existing known mapping; no production reachability claim. |
| **All exact zero addresses** | **27,948** | **Full inventory retained.** |

The 27,641 remaining production **candidate** addresses after the detected inline
test/standard-library/current-protocol categories are not a verified reachable
production count. Every “candidate” or “mixed” row retains an explicit review
obligation. A function-owner name is useful evidence for inline tests, but source
cfg/module review remains necessary for unnamed/generated helpers. Unknown/fault
paths are not relabeled unreachable merely because fixtures are hard.

## Concrete entry and structural proofs

`notifications.rs` 845–847 attempts to decode the constant
`{"op":"budget_events",...}` into RuntimeCommand, whose current enum has no such
variant. Its supported refusal test confirms this. Therefore the `Ok(command)`
forward path 849–860 and successful response parsing/page path 889–890/895–975
cannot execute through this construction. Current behavior records unavailable
budget delivery/backoff; it cannot infer totals or fall back to another command.
Those 84 zero addresses stay in the measurement. Implementing a new accounting
wire capability is separate feature scope, not a coverage exclusion or fabricated
success fixture.

`service.rs` `serve_configured` is executable with an ordinary user and no gateway.
It initializes private management state, launches actual independent Voyages,
serves authenticated localhost transport and owns metadata tasks. UID0/protected
root and configured system-gateway arms need their separately provisioned native
authority; classifying the whole file as Root/unreachable would be incorrect.
The new ordinary service/courier cohort targets that actual lifetime.

`ui/execution.rs` positive review presentation lacks the exact review/receipt/
target consistency checks identified in the earlier reachability map. Ordinary
service dispatch explicitly refuses Execution; connected positive review loading
requires real/effective UID0 and protected storage. Keep ordinary parser/refusal
fixtures and the parked #344/#346/#380 positive-native/fence obligations distinct,
without removing either from the denominator.

## Largest substantive next families

- **Ordinary Vessel service/courier and access:** actual private service startup/
  shutdown, authenticated routing, changed recipient/source after IPC, durable
  cursor/receipt uncertainty and cleanup. Current prepared cohort covers service/
  courier; connected/scoped access remains a separate next owner.
- **Helm viewer lifetime and UI glue:** full host-browser launcher/listener/
  detach lifecycle (205 distinct zero addresses in the sampled export), image/
  preview/completion/viewport job lifetime and current-target disclosure gates.
  Do not repeat completed account/lifecycle/observation families blindly.
- **Ordinary transport/frontdoor:** `process_http.rs` and `duplex.rs` have actual
  authenticated connection/stream, scoped recheck/cancellation/resource admission
  paths. Coordinate ownership before extending active browser/duplex work.
- **Executable extension registration:** sealed package/private provenance,
  all-or-nothing definitions, current owner/policy and actual owned spawn/drain;
  SDK mock launch success cannot establish native adapter cleanup.
- **Installer ordinary/forward recovery:** current supported entry, complete
  pinned raw/semantic receipts, service definition/current namespace and retained
  failures. Critical fixes/native evidence belong to the active delivery owner;
  system/adoption/cross-UID positives remain explicitly parked, not passed.

Future passes must refresh this exact map, trace each selected address to its
executable entry/authority/OS/provider/fault requirement and meaningful assertion,
then reconcile residual aggregate regions. Neither an increasing percentage nor
this classification scaffolding completes #353's unchanged full target.

## Classification reproduction and limits

Use the retained corrected LLVM export arguments, replace only object paths by
their hash-verified archived copies, retain the exact own profdata and original
exclusions, remove `-summary-only` and select `-format=lcov`. For each source file,
retain all `DA:line,count` entries with count zero. Resolve its LLVM function-region
file indices/line spans and demangle names with the matching `llvm-cxxfilt`; test
ownership requires a containing `tests`/`*_tests`/test-support module, not a method
merely named `test` or a word ending with that substring. Mixed owners remain
production candidates. Source-family labels are triage hypotheses pending exact
entry/guard/assertion review, except the named independently checked guard proofs.

The verified cohort changes no Rust production behavior beyond cfg(test)
declarations; current scope and all remaining obligations stay measured. This map export/analysis ran only against retained archives in
separate bounded user units; it did not compile, execute a provider/host operation,
write live profiles, remove an object or alter the full denominator. The
classification does not supply a verified total of reachable production lines.
