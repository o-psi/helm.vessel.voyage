# Reachability and evidence map for #353

## Evidence identity and unfinished objective

The objective remains **100% of reachable production behavior with meaningful
assertions**, in [#353](https://github.com/o-psi/helm.vessel.voyage/issues/353).
This map neither changes that objective nor excludes production source.

The last audited source for this map is
`c15ef79b6568616fc7c0d24350ef3defdd97ee67`: 2,568 passing tests, zero failures,
eight ignored; 101,885 / 132,836 lines (76.699840%), leaving **30,951 gaps**.
There are 539 workspace source files plus the retained standard-library mapping.
All 539 prior workspace files, 14 executed Cargo test targets and three
integration entry points remain represented by 17 distinct current objects.
The report merges 297 own profiles; 235 foreign profiles remained byte-identical.
The corrected detailed JSON and HTML have exactly the summary's totals and source
inventory. The mixed default report is not evidence for this map.

The coordinator retains the summary, detailed export, HTML, own profile manifest
and archived object manifest under ignored `target/coverage-report/v103-lint-final`.
The addresses below come from that corrected detailed export and matching source,
not from the earlier summary-only gap ledger. Counts are aggregate line totals;
zero-count regions and generic instantiations do not form a simple list of unique
source line numbers. The named entry points identify substantive missing journeys,
not a promise that executing each will cover every remaining defensive branch.

## Complete measured-area accounting

These rows are a **disjoint partition** of the entire report, including areas
whose completed test batches are not repeated here. “Direct” means only files
immediately in `voyage/src`. A directory's aggregate does not classify every line
as having the same authority or evidence requirement. Zero-gap areas remain in
their corresponding family. No remaining gap is discarded as difficult.

| Source family | Gaps | Measured lines | Files |
| --- | ---: | ---: | ---: |
| Vessel `process/` | 8,536 | 19,111 | 59 |
| Helm `process_client/` | 7,928 | 33,756 | 128 |
| Installer `system_install/`, `system_install.rs`, `system_preflight.rs` | 2,848 | 3,563 | 4 |
| Voyage direct | 2,270 | 13,237 | 41 |
| Voyage `tools/` | 1,191 | 7,865 | 31 |
| Helm outside `process_client/` | 1,123 | 4,318 | 21 |
| Installer `remote/` | 881 | 1,821 | 2 |
| Voyage `attachment/` | 865 | 12,458 | 65 |
| Voyage `server/` | 847 | 5,687 | 38 |
| Vessel outside `process/` | 676 | 1,810 | 7 |
| Voyage `github/` | 661 | 3,177 | 11 |
| Installer other | 627 | 2,820 | 24 |
| Voyage `provider/` | 396 | 5,747 | 17 |
| Voyage `extensions/` | 384 | 1,995 | 8 |
| `voyage-storage` | 248 | 643 | 3 |
| Voyage `subagent/` | 244 | 2,402 | 9 |
| `voyage-protocol` | 221 | 1,893 | 22 |
| Voyage `identity_helper/` | 152 | 930 | 3 |
| Voyage `policy_profile/` | 147 | 1,937 | 8 |
| Voyage `accounts/` | 123 | 1,053 | 4 |
| Voyage `build/` | 122 | 593 | 3 |
| Voyage `extension_sdk/` | 74 | 635 | 3 |
| Voyage `session/` | 68 | 570 | 2 |
| Voyage `agent/` | 65 | 948 | 7 |
| Voyage `workflow/` | 63 | 412 | 2 |
| Voyage `sandbox/` | 50 | 537 | 1 |
| Voyage `execution/` | 48 | 239 | 2 |
| Voyage `policy/` | 39 | 951 | 4 |
| Voyage `host_resources/` | 14 | 120 | 2 |
| Voyage `completion/` | 13 | 717 | 3 |
| Voyage `context/` | 9 | 367 | 1 |
| Voyage `model/` | 9 | 211 | 1 |
| Voyage `participant/` | 8 | 308 | 3 |
| Retained standard-library mapping | 1 | 5 | 1 |
| **Total** | **30,951** | **132,836** | **540** |

## Entry authority and required evidence

| Family and ordinary entry | Authority and local assertion surface | Separate native, external or fault evidence |
| --- | --- | --- |
| Helm connected UI, transport, draft, inspection and formatting (`App`, `Client::request`/`voyage`, `new_draft::launch`) | Current available route/session/incarnation; human-authored commands; private retained receipts. Owned scripted WebSockets, temporary files and headless frames can qualify envelopes, presentation, revocation and uncertainty without launching a voyage. | Actual event-loop input/terminal restoration, clipboard helper processes and viewer listener cleanup need owned Linux subprocesses. macOS/Windows behavior remains #363's v1.1 scope. |
| Helm plain terminal (`pending_prompt`, `attach`) | Current local policy, writable access, exclusive terminal ownership and explicit attachment. An owned PTY with a fixture `InteractiveTerminals` can qualify actual input, suffix preservation, resize, delivery failure and cleanup. | Descriptor `fcntl`/`write`/`poll`, raw-mode/discard/restore failures, slow output and kernel resize require real owned descriptors. Never use human terminal input. |
| Host browser viewer (`host_browser::Handle`, `run`) | Existing authenticated socket, exact host-browser binding, explicit viewer controls; Helm remains a viewer. Private listener/bootstrap/launcher lifetime is ordinary and testable with a scripted Vessel. | The local HTTP fixture does not establish Chromium execution, full-site mirrors, TLS, child frames, private-input fencing, canvas/video or media cost. Keep #333's browser qualification separate. |
| Opt-in local browser (`browser::runner::run`) | Independent local consent; helper control/capture epochs, original socket, exact dispatch receipts. Scripted helper and owned socket can qualify runner arbitration and shutdown. | Node/Chromium sandbox and real browser child observation require the installed local adapter and native fixture. This is not permission to migrate execution authority to Helm or to retire the separate consent boundary. |
| Vessel ordinary service, catalogue, access, notifications and updates (`Supervisor::request`, `connected`, `granted`) | Authenticate owner/connection/scoped grant, recheck current bindings and route to independent Voyage processes. Local catalogue, socket and child fixtures can qualify identity/refusal/receipt races. | Actual independently owned process readiness/death, systemd activation, OS resource failures and durable shutdown must be observed. Privileged bound journeys below cannot be inferred from ordinary service tests. |
| Vessel privileged identity/adoption/transition (`admin_execution`, `execution_transition`, `bound_*`, `guardian`, `identity_*`, `migration`) | Shared parsers/digest construction/refusals and immutable SQLite claims have ordinary local fixture surfaces. Do not call all 8,536 `process/` gaps privileged or unreachable. Protected positive paths require fixed real Root/cross-UID gates, provisioned identities and explicit owner rights. | #344/#346/#380's Root/system, adoption, migration, guardian/subreaper, cross-UID pipes, capability clearing and retirement are explicitly parked for v1.1. They remain in the denominator and are unfinished evidence, not skipped passes. |
| Installer system/adoption (`system_install::run`, `adoption::run`, `system_preflight::assess`) | Explicit account names, archive/schema/identity facts, exact review digest and retained operation metadata. Parser, path, bounds and snapshot consistency can be asserted locally. | Real UID/GID/groups, Root-private stores, system unit publication/readiness/reboot, opaque ownership transfer and rollback require disposable native Root Linux under the established v1.1 scope. |
| Installer ordinary acquisition/install/remote update (`install`, `remote::linux`, `source`, `legacy`) | Pinned archive/checksum/version, owner-approved exact plan, private receipts, previous pointer and account/process namespace. Temporary offline sources and service-manager fixtures qualify admission/failure claims. | Hosted acquisition/install, actual service activation/rollback and helper death around durable commit require owned native journeys. CT119/CT106 evidence belongs to those exact environments; synthetic data/service fixtures do not replace it. |
| Voyage execution/server/attachment/session/build | One process owns session/conversation/tools/cleanup. Current execution policy, actor, run/command IDs and canonical revision bind all admission and mutation. Existing ordinary owned process/scripted provider fixtures qualify local contracts. | Provider/network faults, filesystem durability, process death and descendant cleanup require explicit fault environments. Completed run/Goal/reconciliation/lifecycle/observation batches are retained; residual gaps do not authorize repeating their entire scope. |
| Voyage tools (`ToolContext`, `ToolRegistry`, filesystem/process/browser/Vessel tools) | Live registry and local roots/access/approval/cancellation/resource budget are authoritative. Temporary paths, scripted transports and owned commands qualify refused/accepted effects and exact receipts. | OS process-group/session ownership, inaccessible foreign-UID files, namespace/sandbox failure, browser workers and remote Vessel effects require controlled native/remote fixtures. Application policy is not an OS sandbox. |
| Voyage GitHub (`operator::execute`, `admin::execute`, `Service`) | Local admin recovery is attended/private-store only and needs no token or HTTP client. Remote service separately requires enabled capability, executing-machine token, current policy, exact object/actor/head and explicit write approval. | Scripted loopback service tests qualify transport contracts, not authenticated GitHub publication or real rate limits. No remote send is part of the prepared admin family. |
| Voyage extensions (`register`, `ExtensionTool::execute_output`, `Manager::shutdown`) | Required sandbox, supervised process scope, sealed reviewed package, current runtime policy, bounded read capability, reservation-before-spawn and observed cleanup. SDK/scripted launch and owned subprocess fixtures cover their stated contracts. | Real sealed executable spawn/isolation, inherited descriptors, PID/session identity, descendant/process death and filesystem-worker drain need owned native Linux. External provider access is not implicit. |
| Voyage provider/account/inference | Current executing-machine credentials/account capabilities and provider budget; deterministic protocol, retry, redaction and error classification can use existing scripted services. | Live OAuth/login, billed inference and service behavior require approved provider/budget; synthetic responses are never live success. Other-platform account privacy requires native #363 evidence. |
| Voyage policy/config/workflow/context/completion/protocol | Exact policy/selection/roots/actor/time/identity/usage contracts; temporary documents and explicit callbacks qualify deterministic logic. No policy fixture grants additional runtime rights. | Allocation exhaustion, arbitrary scheduler failure and unreproducible kernel errors need fault facilities. A structurally impossible arm needs specific construction proof rather than a blanket module classification. |
| Native private storage (`voyage-storage`, journal storage, execution scope/identity helpers) | Private-directory/file shape, ownership, link/bounds/schema refusal and scope freshness can be asserted on owned Linux files. Positive cross-UID/Root helpers retain their actual gates. | Linux protected storage/helper leases and native macOS/Windows private storage require their platform-specific environments; Linux coverage is not cross-platform security proof. |
| Participant/supervision/subagent families | Existing exact authority, child/task/assignment ownership, no replay and observed cleanup assertions remain authoritative. | Residual OS/process, remote participant and paid provider evidence is separate. Completed family batches are not being recreated by this prep. |

## Largest next ordinary families, with concrete missing journeys

The counts below are per-file gaps within the table above; they are not additional
gaps. Each named region is zero-count in the corrected detailed report. Existing
fixtures are listed so a successor can extend the same authority boundary.

| Source and gaps | Existing assertions | Concrete next owned assertion family |
| --- | --- | --- |
| `helm/src/plain_terminal.rs`: 278 / 469 | `plain_terminal_tests.rs` and `plain_terminal_final_tests.rs` cover byte/UTF-8 editing helpers, selection, rendering, typed cleanup and cancellation wrappers. | `pending_prompt` 179–321 and `attach` 592–653 are unexecuted production loops. Use a child with private HOME/XDG and owned PTY: real cursor/delete/Unicode input, capacity block/recovery, Ctrl-C/empty Ctrl-D, detached suffix preserved, direct terminal bytes delivered only once, resize/snapshot failure, event-channel closure, policy revocation and termios/fd restoration. Cover `discard_failed_attempt` 114–129 and actual output 404–447 with closed/full owned descriptors. |
| `helm/src/process_client/host_browser.rs`: 229 / 521 | Handler and adapter fixtures cover origin/Host/CSRF/token, one-use bootstrap, owner preparation, uncertainty and stale binding refusal. | `Handle` 48–109 and full `run` 335–457 are unexecuted. Start the actual private listener against an existing scripted duplex fixture; prove launcher mode/content, security headers, bootstrap consumption, control requests before/after connection, owner/socket change, stop/control closure, detach on original socket, server retirement and removal of only the owned launcher/directory. No Chromium is required for this Rust viewer lifetime. |
| `helm/src/process_client/browser/runner.rs`: 213 / 456 | Existing six tests cover consent booleans, epoch refusal, exact claim/result/cleanup, stale binding and prepare-heartbeat-offer ordering. | `run` 60–258 is unexecuted. An owned scripted Node helper must drive local private→shared→human changes, original-socket disconnect, helper exit, notification closure/replay-gap, pending/action arbitration and bounded action cancellation. Preserve dispatch evidence when shutdown is unknown. Existing dispatch residuals include remote receipt cancellation and withheld observations at 424–430 and 471–475. Real Chromium remains distinct. |
| `helm/src/process_client/ui/new_draft.rs`: 206 / 602 | Existing 18+ state/storage cases and launch/workspace fixtures cover unsent drafts, exact first-send recovery, actor/route/settings fences and retained receipts. | Extend only residual command and dispatch regions after tracing corrected zero regions to `create_on_route`/`send_draft`/`first_send_update`. Assert rejection keeps exact composer/image markers, malformed creation metadata cannot switch voyage and unknown original creation resolves instead of sending again. Do not repeat completed account/settings or lifecycle cohorts. |
| `helm/src/process_client/ui/paste.rs`: 166 / 448; `helm/src/clipboard.rs`: 170 / 375 | Existing private helper, path/policy, image and inline-paste cases exercise selected contracts. | The uncovered panel clipboard path 336–372 and image-send 496–558 need owned late-result/cancel/changed-destination/authority cases, preserving original unsent text and markers. Actual clipboard `read` 145–205/Linux helper 210–285 requires an isolated child with fake owned helper, stdin/stdout bounds, timeout and observed helper death; never query the human clipboard. |
| `helm/src/process_client/frontend.rs`: 162 / 317 | Synthetic validations refuse invalid prompt/workspace/catalogue and retain configured envelopes. | `open` 14–50 and `run` 147–219 plus follow paths 222–269 remain unexecuted. Use a real owning loopback Vessel/ordinary Voyage and retained launch file; prove canonical accepted workspace, original command identity, uncertainty resolution, receipt terminalization and cleanup with one provider request. The existing server fixture is the starting boundary, not a second embedded executor. |
| `voyage/src/extensions/runtime.rs`: 164 / 462 | Final/acceptance/boundary tests already cover SDK admission, read worker drain, confidentiality, counted output acceptance and owned broker child cleanup. | Production `register` 734–850 is almost entirely zero-count: sealed package catalogue, required sandbox/supervised scope, incomplete private-source provenance, capability/definition mapping and all-or-nothing collision behavior need a child with a genuine owned process scope and package fixture. Adapter native spawn failures and resource ledger release failures remain separate fault cases; do not infer them from mock launch success. |
| `voyage/src/github/operator.rs`: 170 / 361; `service.rs`: 124 / 555 | Operator argument/reference/file parsing and loopback Service publication tests already assert exact actor/head/approval/no replay. | Extend unexecuted operator dispatch glue using existing loopback `http_fixture`, private journal and original input-file authority: exact inspect/list/cancel/reconcile/dispose/forget calls, post-await policy cancellation and bounded output. Do not send to authenticated GitHub. |
| `voyage/src/github/admin.rs`: 141 / 141 | `store_tests.rs` qualify low-level SQLite transitions; no admin-glue test module existed at c15. | **Prepared here:** complete local command route family (below), using the existing explicit-directory `execute_inner` and counted `Approver`; no production seam, provider or remote client. Actual 900-second approval deadline and arbitrary filesystem durability remain explicit follow-through, not an “Expired” fixture being mislabeled timer evidence. |
| `voyage/src/tools/vessel.rs`: 150 / 709 | Existing action/schema/transport fixtures qualify selected local tools. | Trace zero regions in `perform` 781–807, 858–906 and 1186–1196 to settings/start/participant actions; extend exact local socket replies and changed/current grant/cancellation checks. Participant/subagent work already delivered is not repeated; real foreign Vessel execution remains separate. |

Other ordinary reachable gaps include transcript/sidebar geometry, completion,
preview worker lifetime, process notifications/access/HTTP duplex and private
config/tool errors. Their current totals remain in the complete area partition.
No short list is claimed to exhaust every uncovered assertion or to establish
100% merely by executing happy paths.

## Parked execution review finding and source proof

`helm/src/process_client/ui/execution.rs` has 217 / 248 gaps. The only three
current tests cover stale transport incarnation, lost current reply and generic
unconfirmed preparation display. `execution_arrived` 260–290 deserializes a
`SavedExecutionReview` and persists/display its IDs without cross-checking review
and receipt session/incarnation/command/review/digest consistency with the
selected target and retained operation. A fabricated reply from a scripted or
compromised peer can therefore replace the current UI's retained review. This is
an exact-receipt follow-up, not evidence that Root execution has occurred.

Entry proof: `ui/actions.rs` 157 accepts `/execution`; `transport.rs` 163 sends
`VesselCommand::Execution`. The ordinary owner service refuses that command at
`vessel/src/process/service.rs` 948. The authenticated connection dispatcher
`process/access/connection.rs` 107–140 checks Observe/Create/Execute/Decide or
transition Lifecycle/Cancel rights, full account-owner access and current grant
before Linux dispatch. Its capability list advertises `execution_identity` only
with a bound layout. `admin_execution.rs` 646 resolves a retained review through
`database_execution_reviews.rs` 19–26, which requires **both real and effective
UID 0** and a protected Root directory. The transition observation fallback uses
the same protected identity family. Ordinary shipped user-scope hosts cannot
legitimately return a positive saved administrator review through this path.

The parked [#344](https://github.com/o-psi/helm.vessel.voyage/issues/344),
[#346](https://github.com/o-psi/helm.vessel.voyage/issues/346) and
[#380](https://github.com/o-psi/helm.vessel.voyage/issues/380) v1.1 work must add
the precise UI review/receipt/target
fence before supported positive privileged review claims, together with negative
client tests and native Root evidence. Deterministic parser/storage refusal
tests are still feasible; the whole module is not structurally unreachable and
its lines stay measured. No privileged implementation is added by this prep.

## Source-only admin preparation and resumption

The isolated `coverage353-admin-prep` branch adds only an `#[cfg(test)]` module
declaration to `github/admin.rs` plus `admin_family_tests.rs`. Cases assert:

- Read-only List/Inspect/Audit without token/approval; exact scoped inspection,
  missing identity and bounded pagination.
- Unattended/cancelled/retired authority refusal before creating private storage.
- Unavailable owned store returns a sanitized failure, preserves an unrelated
  sentinel/original record and never requests approval.
- Exact approval snapshot for Cancel, uncertain Dispose and terminal Forget for
  Cancelled/Disposed/Published; audit evidence preserved and no send enabled.
- Wrong digest/state and every nonapproved outcome preserve original evidence;
  read-only mutation never requests approval.
- Cancellation, authority retirement and record change during approval recheck
  before mutation; a concurrent Sending record remains unknown and untouched.
- Confidential previews refuse before approval; local read output is redacted.
- ClearAudit uses exact exported digest, explicit approval and post-await snapshot;
  a changed audit preserves both old and concurrent evidence.

This preparation is **not compiled or executed** and has no new coverage result.
Only rustfmt, source/path review and `git diff --check` are completed. It neither
changes Cargo dependencies nor adds a hosted quality job. No provider/Root/browser
or main-checkout effect occurs. A successor must reconcile current main, review
this source, execute focused tests, correct concrete failures, run strict applicable
checks, then perform and audit full workspace coverage after final relevant edits.
Keep #353 open, retain the full denominator and publish the new compact summary
only after source/object/profile identity and meaningful outcomes are verified.
