# Shared local browser: Voyage runtime contract

The browser executes beside Helm, not on the Voyage host. Voyage owns one normal
agent loop, policy, canonical history and durable intent/reply state. The `browser`
tool is registered at root-agent construction only when a browser has been explicitly
shared. A configuration file cannot enable sharing. Subagents do not inherit the
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
Renewal is a Shared Control operation; human/private transitions and returns to
Shared require strictly newer controller and capture epochs. Disconnected/revoked
bindings require an explicit new offer rather than automatic lease resurrection.

Operations are Offer, Pending, Claim, Result, Control, Receipt and Cleanup. Pending
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
inspect, tabs list and screenshot only. Every effect asks the Voyage approver,
bounded by tool timeout; local confirmation is separate and cannot be substituted
by a remote approval response.

## Privacy, persistence and uncertainty

The private canonical Journal stores browser intents, exact action hashes, command
identities, reply hashes, control and explicit unresolved states. Dispatched requests
atomically maintain existing `process_session_resources` rows (`browser_effect`),
so ordinary lifecycle readers retain cleanup obligations. Payloads are private;
resource projections contain only identity and cleanup metadata. Identity conflicts
refuse. Receipts are not evicted to make uncertain effects repeatable: bounds are
128 retained requests, 4096 command receipts, 16 MiB browser checkpoint. Capacity
exhaustion is explicit refusal. A new voyage has a new receipt scope.

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
file and, after remote disclosure approval, reads at most 2 MiB. Configured secrets
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
