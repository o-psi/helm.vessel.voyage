# Voyage-owned host browser

Issue [#333](https://github.com/o-psi/helm.vessel.voyage/issues/333) tracks the shared browser. Voyage starts and owns one sandboxed Chromium instance for its session on the Vessel host. Vessel supervises the Voyage process and carries authenticated Helm operations; Helm Web and native Helm use the same viewer. Disconnecting Helm does not end the browser or the voyage. The older opt-in local browser retains its separate consent boundary for compatibility, but its `browser` agent tool is no longer offered.

## Live page path

The task browser runs the website. A bounded rrweb recorder in its page produces a full DOM snapshot and subsequent DOM changes. The worker retrieves events through Playwright's private pipe, inlines captured image/font/style resources, compresses the event batch and returns it through the authenticated Helm–Vessel command connection. Cross-origin child frames have separate recorders read through that same private pipe; the worker never uses rrweb's parent-page `postMessage` mode, which would expose child-frame content to the site's parent scripts. Helm reconstructs the page and mirrored child frames in script-free, sandboxed iframes with a restrictive content security policy. No website JavaScript runs in Helm. The viewer reads changes with a cursor; a lost cursor requests a full snapshot. The worker stops recording when no viewer remains and clears the recorders and retained downloads on control changes.

Workers that advertise `css_chunks_v1` can transport a larger bounded logical snapshot by deduplicating captured CSS and returning all gzip chunks atomically. Helm reconstructs the complete captured CSS before replay; it does not remove page structure or styles to make a snapshot fit. Each decoded chunk is at most 512 KiB, with at most 16 chunks and an 8 MiB total bound on compact and expanded logical event data across the top document and child frames. The existing private-pipe and outer wire-reply bounds remain. A client validates the complete top document and child-frame reply before replay or cursor advancement. Legacy workers keep the original gzip format. Capability comes from the voyage's actual retained worker, so upgrading Core alone does not enable the new format for an existing immutable launch configuration. Pages exceeding these limits still produce a bounded observation failure; this does not establish universal website fidelity.

Clicks, form fills, selects and wheel events carry the recorder's element ID back to the task browser. Cross-origin frame actions additionally carry a short-lived frame ID, changed on frame navigation. The worker resolves both IDs against the current page and checks live authority, visibility and input sequence before using Playwright. Keyboard, navigation, tabs, resize and dialogs remain typed operations. Canvas and video areas receive bounded, localized screenshots at most once per second; clicks on those areas use a relative position inside the named element. An iframe that cannot be mirrored can also receive that localized fallback. This fallback can show changing content at its bounded sample rate; it does not reproduce video cadence or audio, or guarantee interaction inside an unsupported frame. Native file selection uploads at most 2 MiB through the authenticated operation; completed host downloads are offered only to their controller and are size bounded.

This replaces the whole-page frame transport and its second browser process. It does not make arbitrary websites uniformly faithful: script-generated media, deeply nested or more than eight child frames, CSS resources that exceed the worker's asset budget, transformed iframe layouts and browser-specific rendering may differ. A child snapshot over its per-frame budget falls back to an image when its host is in the top document. Qualify a target site with an actual Helm viewer before claiming parity. The source contract is [the worker](../voyage/browser/CONTRACT.md), [public protocol](host-browser-protocol.md) and [shared viewer](../helm/browser-view/CONTRACT.md).

## Authority and privacy

The `HostBrowserOperation` schema is strict. The authenticated socket and process grant identify the principal; an operation's attachment ID is never authority by itself. Status, attachment and live page reads require Observe. Starting, controlling, typing and closing require Execute. Input must carry the current incarnation/browser/tab/document/viewport/controller/capture binding and a consecutive sequence. Ordinary page and navigation input can claim human control in that same operation: the worker fences pending agent effects and applies the original action only while its target remains valid. Private control is a separate explicit choice before entering secrets. It excludes other viewers and never automatically returns to the agent on disconnect. Unconfirmed effects are never replayed automatically; receipts preserve exact identities without private page content.

During native viewer startup, catalogue updates can arrive before the response
that confirms a resumed Voyage owner. Helm briefly defers cancellation for an
incarnation mismatch while an unbound preparation is in flight. This bounded
wait never adopts an owner from the catalogue or extends command deadlines.
Only the verified preparation response and revision permit rebinding the same,
explicitly undispatched command. Once a browser has been bound, owner mismatch
still cancels the viewer immediately; deletion, archive and socket loss also
retain their cancellation behavior.

The worker retains a private Chromium profile, an origin/DNS-pinned proxy and explicit grants for private destinations. Browser flags and application policy are defense in depth, not an OS egress sandbox. Browser resources and descendants require observed cleanup. Human input and page snapshots stay out of model-visible history, ordinary logs and durable receipts. The executing host can see the website and its credentials. Native macOS/Windows behavior and broad public-site security are not established by Linux loopback checks.

## Implementation and verification

### Recovering retained browser capacity on Linux

A browser slot survives a process crash because stopping the Voyage process does not prove its Chromium descendants stopped. On Linux, when capacity is full, Voyage now checks slots belonging to sibling sessions under the same Vessel directory. It reclaims a slot automatically only when the prior session's startup, execution and guardian locks are free, its registration and private worker lock identify the exact owner, the worker PID is absent, and the browser guardian receipt positively confirms descendant and temporary-profile cleanup. Recovery records an audit and retains the old worker lock. It does not reconcile uncertain website effects. Missing, malformed, or incomplete evidence keeps the slot reserved.

Linux browser admission starts with four slots and allows up to sixteen on hosts with spare memory. Each extra slot requires two GiB of available memory beyond a four GiB reserve; a finite service cgroup memory limit further reduces that headroom. Existing live browsers are never evicted just to make room. On a constrained or fully occupied host, admission still refuses rather than discarding another voyage's browser state.

For older or otherwise unmatched slots that lack this automatic proof, inspect the suspended session, its `journal/host-browser/worker.lock` PID and all browser descendants on the executing host. Once cleanup is independently observed, run `voyage host-resources browser-capacity SESSION_UUID --session-dir SESSION_DIRECTORY --confirm SESSION_UUID --observed-no-descendants --reason 'observed worker and browser descendants absent'`. The command holds the session startup and guardian locks, verifies its suspended and cleanup-observed records and dead worker PID, records an audit, retains the old worker lock under a recovery name, and reclaims only that session's slots. Never run it for an active session or use an absent PID alone as proof that descendants stopped. The CLI only supports Linux process evidence.

The Linux worker uses `voyage/browser/worker.mjs`, `mirror-source.mjs`, the vendored rrweb 2.1.6 runtime and its license, and `guardian.py`. The vendored runtime must expose `globalThis.rrweb` in both a classic script and a Vite-imported module; otherwise the Web viewer receives mirror data but cannot replay it. The shared viewer is `helm/browser-view/viewer.mjs` and `viewer.css`, loaded by both Helm clients. `packaging/browser_assets.py` inventories every shipped browser asset. The exact local checks and their scope are in [quality](quality.md). The GitHub issue holds the delivered commit, build artifact and remaining qualification evidence.

### Read-only mirror failure diagnostics

Mirror reads expose a fixed diagnostic code for page size and bounded observation
failures: `page_too_large`, `mirror_limit`, `operation_timeout`, `capture_fenced`,
`capture_busy`, `recorder_disabled`, `viewer_missing`, `observation_unavailable`
and `worker_error`.
The boundary emits only its own fixed message; worker text and state details remain
withheld. Other codes keep the existing worker-reply classification and generic
fallback. Input and lifecycle commands
keep their existing receipt and unknown-outcome handling. A mirror failure does not
authorize repeating an interaction or establish successful capture or cleanup.

### Agent inspection and interaction (#379)

The `host_browser` tool uses the same Voyage-owned browser and authority fences.
`inspect` accepts optional `offset`/`limit` for controls (default 64, maximum 128)
and `text_offset` for 16,384-character text pages. Counts, next offsets, truncation
and unsupported content are explicit. Offsets index all candidate controls,
including hidden controls, so a page may contain fewer visible elements than its
limit. Choose a smaller limit when the tool output budget is small. Each inspection
replaces previous element references. The observation reports roles, labels,
native selected/checked states and at most 64 select options.

Inspection reports at most 32 short-lived child-frame IDs. Passing an observed
`frame` ID reads that same- or cross-origin child through the worker's private
Playwright pipe. Frame navigation, document/tab/control changes, DOM mutations,
changed control properties and a sixty-second reference lifetime invalidate targets.
Reinspect after dynamic changes; unsupported or stale targets refuse before action.
Closed shadow roots are not inspected. Canvas/video use screenshot evidence.

`key` targets an observed reference and accepts ordinary keys and modifier chords;
`select` accepts a native option value, and `check` sets a native checkbox/radio
state (radio unchecking is refused). `double_click` and `drag` use observed source
and target references. `history` supports back, forward and reload. These are
effects governed by the existing access mode, approval and network policy.
`read` returns paged text of an observed element; it does not expose input values
or arbitrary HTML/evaluation. `diagnostics` reports bounded console-error and
page-error counts for the current tab since its last control fence. Log text is
withheld because websites can print secrets; private/human activity is excluded.

Operations retain exact receipts; an uncertain effect is never retried automatically.
An unavailable observation can be retried independently of a prior effect. The
worker's content-free receipts never contain element text, form input or log text.
These operations do not provide arbitrary JavaScript, CDP or filesystem access.


### Private zero-viewer observation

The worker writes bounded owner-only `capture-observation.json` metadata at
attachment/disconnection/capture-stop transitions, never per mirror poll. It
contains only worker/browser/source identity, a sequence/time, browser liveness
and zero-viewer/recorder-stop observations. No page, URL, input, frame topology or
credential enters this diagnostic, public status or canonical receipts. It is
not an execution grant, cleanup receipt or evidence of website effects.

A successful stop observation requires actual source-recorder stop acknowledgments
with inactive recording and empty pending capture, or an observed absent recorder
in that frame. Failed, detached or inaccessible frames keep it unconfirmed. The
source-created recorder closure and its global slot are immutable; repeating the
init script retains that closure. A concurrent capture/attachment change cannot
promote an old stop acknowledgement. A joining viewer first invalidates prior
idle evidence; inability to publish that invalidation is an existing
`observation_unavailable` pre-effect refusal, not permission to reuse stale proof.

The maintained qualification host probe checks this private producer metadata
against actual worker PID/start/browser and the independently pinned source,
private file/directory identities and retained slot for a ten-second interval.
The client disconnects normally without closing the browser; later reconnect
uses the existing page's status-recovery control, same browser and a fresh
attachment. The extra idle interval is separate from the nine active cost windows.
Source contracts and Linux qualification remain distinct; native evidence is
required before claiming this observation passed on a deployment.

Recorder start permission is disabled until the worker enables a monotonic
capture generation over its existing private pipe. A stop invalidates earlier
permits; exposed page drain cannot reactivate recording without that private
capability. Stop proof follows bounded retirement of prior mirror work; unresolved
work keeps observation unconfirmed. The idle host probe independently observes
the actual Chromium root/program/profile and guardian, rather than trusting a
Node-owned liveness boolean. Native reconnect decisions use fresh Status before
reattaching without Start.
