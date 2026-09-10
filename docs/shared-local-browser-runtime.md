# Shared local browser: Voyage runtime contract

The browser executes beside Helm, not on the Voyage host. Voyage owns one normal
agent loop, policy, canonical history and durable intent/reply state. The `browser`
tool is registered at root-agent construction whenever the supervised runtime supplies
its broker, including before a local browser is shared. Presence is capability, not
authority: each invocation and claim checks live sharing. This allows sharing during
an active run. A configuration file cannot enable sharing. Subagents do not inherit the
browser tool. Local browser authorization remains independent of Voyage approval.

## Wire and fences

[`browser.rs`](../crates/voyage-protocol/src/browser.rs) defines the shared types.
The public `VoyageCommand::Browser` maps to the private `RuntimeCommand::Browser`;
both carry `BrowserOperation`. Public operations use the ordinary authenticated
command connection. This runtime introduces no HTTP browser endpoint or secondary
agent transport. Socket transport, local executor and companion are separate
components; their deployment checks are not established by this document.

All browser commands require current Execute authority. The authenticated principal
is assigned by Vessel/Voyage authorization, never by browser payloads. Bindings carry
session, process incarnation, optional run, browser, resource and executor UUIDs,
controller/capture epochs and lease expiry. An idle offer may have `run_id: null`;
every dispatched request has the actual current run. Offer is an assertion of
explicit **local sharing**, not a remote request to enable it. One executor is
admitted, subject to prior unresolved-effect cleanup. Lease maximum is 60 seconds.
Renewal is a same-mode Control operation in Shared, Human or Private, with the same
controller/capture epochs and a valid lease. It does not fence outstanding work or
erase captures. Actual transitions into Shared/Human/Private require strictly newer
controller and capture epochs. Expiry fences all three active modes. Disconnected/revoked
bindings require an explicit new offer rather than automatic lease resurrection.

After explicit local sharing consent, Helm first sends top-level
`VoyageCommand::PrepareBrowser` (`{"op":"prepare_browser"}`, no fields). It requires
Execute authority and follows/resumes the current owner through ordinary lifecycle
fences, including unresolved-cleanup refusal. It creates no agent, browser effect,
or sharing authority. The normal `VoyageReply.incarnation` supplies the fresh outer
and Offer incarnation. `PrepareBrowser` is deliberately not an observation served
by the suspended reader; it has no mutation UUID and needs no caller incarnation.
Send Offer immediately after Prepare (before ordinary idle suspension), never reuse
a pre-resume binding. Prepare must not be triggered before local sharing consent.

Browser operations are Offer, Pending, Claim, Result, Control, Receipt and Cleanup. Pending
is an immediate bounded fetch (1–16 requests, aggregate 3 MiB); use the same
full-duplex connection for polling. Claim commits Dispatched and a canonical
session-resource obligation **before** acknowledging permission to execute locally.
Only one request can be dispatched at a time. Claims recheck current run,
cancellation, controller/capture, sharing and execution authority. The local executor
must durably deduplicate the exact request and never execute an uncertain claim.

Tool actions: inspect, navigate, click, fill, scroll, tabs (list/open/select/close),
screenshot, upload_prepare, upload and download. Typed targets carry page and
observation UUIDs; Voyage checks retained observations and the local executor must
independently refuse stale targets. No JavaScript, CDP, shell, arbitrary local file
paths or browser credentials are part of this contract. Read-only mode permits
inspect, tabs list and screenshot only. For effects, Approval access mode asks the
Voyage approver with bounded cancellation/timeout; Unrestricted skips that ordinary
remote approval. Execution authority is rechecked after any await and before enqueue
and claim. Local confirmation is independent in every access mode and cannot be
substituted by a remote approval response.

## Privacy, persistence and uncertainty

The private canonical Journal stores browser intents, exact action hashes, command
identities, reply hashes, control and explicit unresolved states. Dispatched requests
atomically maintain existing `process_session_resources` rows (`browser_effect`),
so ordinary lifecycle readers retain cleanup obligations. Payloads are private;
resource projections contain only identity and cleanup metadata. Identity conflicts
refuse. Settled entries without a live waiter, pending cleanup or unconsumed capture
are compacted to metadata-only tombstones; action/upload payloads are discarded.
Original bindings, action hashes, final receipts and any result hash remain exact.
Cleanup retirement updates canonical resource rows atomically with the checkpoint.

Exact bounds per voyage:

- 128 full request entries and 8192 retired request identities. New enqueue stops
  when either bound is reached. Unknown cleanup is never retired.
- 256 full command reply cache entries, with other accepted command IDs reduced to
  exact principal/operation SHA-256 tombstones. UUID ordering chooses cache retention,
  not recency. No identity tombstone is evicted.
- 65,536 ordinary command admissions; 256 additional identities are reserved only for
  commands that positively settle an outstanding cleanup obligation (65,792 total).
  New effects stop at the ordinary bound. Repeated non-settling cleanup cannot consume
  that reserve. At one renewal every two seconds, the ordinary budget represents
  about 36.4 hours **minus other commands**, not an unlimited lease lifetime.
- The complete serialized browser checkpoint remains bounded to 16 MiB; large active
  transfer payloads can reach this bound earlier. Pending fetch is at most 3 MiB;
  observations retain at most 128 page identities.

A replay of an exact retired command is explicitly refused as **reply retired**;
changed reuse is an identity conflict. Neither dispatches anything. Read the original
request Receipt, never retry the effect under a fresh identity. A retired request
continues answering Receipt/Cleanup under its exact original binding; a duplicate
Result must match its retained result hash. After cleanup-only retirement without a
result hash, a late result is refused rather than inventing website success.
Capacity exhaustion is explicit bounded refusal, never unsafe eviction. A new voyage
has a new receipt scope. These are improved finite bounds, not unbounded storage.

Cancellation, timeout, private takeover, lease loss and process restart fence pending
work. Dispatched work becomes Unresolved; none of these events establishes effect
success or observed cleanup. Old queued/captured content is withheld across privacy
or controller fences. A late result for an exactly retained dispatched request may
settle cleanup, but stale capture is not stored or shown to the model. Result hashes
remain exact. Results are consumed once; repeated commands return receipts, not
repeated captures. Run completion uses existing bounded managed-resource cleanup.
Unexpired shared/human/private offers prevent idle suspension; unresolved effects
continue blocking after lease expiry.

Recovery addresses the **current** outer `VoyageRequest.incarnation` and retains the
**original** inner request binding. Receipt, Cleanup and late Result accept an old
inner run/incarnation only for the exact retained request and current authorized
principal. Offer/Claim never do. `Cleanup { observed: true }` is an authenticated
local-executor cleanup attestation: it does not turn an unknown effect into success.
For a claim whose response was lost, local absence-of-dispatch evidence may clear
cleanup while the effect receipt remains Unresolved. A restarted owner does not
claim the former process survived. Ordinary explicit process recovery remains
necessary when the owner itself is gone.

## Transfers and outputs

The model's `upload_prepare { path }` resolves a Voyage-policy-readable regular
file and, after remote policy authorization (approval in Approval mode), reads at
most 2 MiB. Configured secrets
are withheld. The wire action carries transfer UUID, display name, MIME and base64,
never the remote path. The local executor independently approves/stages it and
returns a grant identifier. `upload { target, element, grant_id }` is a separate
locally authorized browser effect. Download requires an explicit local disclosure;
it returns a bounded file blob, not an instruction to write/open/execute it.

Result image/file blobs are mutually exclusive and limited to 2 MiB. Only an
explicit screenshot may return an image; only download may return a file. Images
are decoded and checked for actual raster MIME before retention, then use existing
ToolOutput/Image session artifacts. Files use ToolOutput/Resource artifacts, not
automatic workspace writes. Provider image hydration/capability support is separate.

## Verification scope

Package checking and targeted synthetic broker probes establish Rust construction
and selected durable/failure behavior only. They do not establish a live provider,
authenticated deployed end-to-end browser session, macOS/Windows behavior or local
browser sandbox/security. Parent integration must check the local executor, Helm
manager and duplex transport together. No broad removed test suite is recreated.
