# Voyage host browser worker

`worker.mjs` is launched by the Voyage process behind `guardian.py` over private JSON-lines stdin/stdout. There is no page or HTTP control endpoint. The parent owns authentication, authorization, session policy and filtering human commands from model history. Each `{id,op,...}` receives `{id,ok,result}` or a bounded refusal. Durable receipts precede lifecycle and agent effects; read-only `mirror` and status are ephemeral. Request identity reuse with changed content refuses. An uncertain dispatched effect is never automatically replayed.

Trusted `init` supplies an absolute private state root, Chromium executable, public-web policy, explicit private origins and viewport dimensions. `open` starts one sandboxed task Chromium with a private Playwright pipe and isolated profile. The worker uses an origin/DNS-pinned proxy, route checks, blocked service workers and task-page WebRTC egress restrictions. These are defense in depth, not an OS sandbox. `close`/`shutdown` and the guardian must observe descendant/profile cleanup; unresolved cleanup retains the lock and receipt obligation.

The parent admits at most four viewers with `join`. `disconnect` removes a viewer; private mode does not implicitly return to agent mode. `control` changes agent/human/private authority, advances epochs, stops recording, clears retained controller downloads, and waits for old effects to settle. Browser/tab/document/viewport/control/capture epochs fence stale work. `input` requires one authorized controller and strictly consecutive per-viewer sequence. Dialog and stop commands use an interrupt lane. Agent actions retain fresh opaque element references and their own policy boundary.

`mirror` is a read-only viewer request with a cursor. The injected `mirror-source.mjs` lazily records rrweb snapshots and changes in the main document and up to eight child frames. It keeps bounded events, can take a new full snapshot, and exposes node IDs to the worker through Playwright. Child events are read through the private Playwright pipe, never posted to website parent scripts. The worker compresses bounded event arrays, inlines captured image/font/style resources from a bounded host cache and sends localized visual fallback images for visible canvas/video or unsupported top-level iframe elements. No page content is written to worker receipts. The worker resolves element IDs on the current page or a short-lived frame document for click, fill, select, wheel and upload, checking live visibility/editability before each effect. Downloads are capped and associated with their owner.

`status.mirror_formats` advertises the worker's optional `css_chunks_v1` encoding.
An omitted `mirror.format` retains the legacy gzip event array. Negotiated mirrors
return one atomic `gzip-chunks` reply containing an event array and a per-batch CSS
dictionary. Exact CSS is restored at recognized fields before replay; stylesheet
resource rewriting retains each document's URL base. Chunking changes neither
website execution authority nor viewer admission. Each decoded chunk is at most
512 KiB, with at most 16 chunks. Compact and expanded logical data share an 8 MiB
aggregate bound across the top document and child frames; private-pipe and outer
wire bounds remain 2.2 MB and 2.8 MB. Invalid or oversized data is refused rather
than pruned or partially replayed. A retained old worker does not acquire the new
format merely because the supervisor or client was upgraded.

Compression work is registered before its first asynchronous operation, bounded
and serialized per document, and fenced after each await. Stop observations retain
pending work and byte obligations until their completion is observed. Private
control, last-viewer disconnect and teardown clear retained CSS and discard stale
assemblies; they do not claim outstanding compression already finished.

The recorder combines its compact event dictionaries without expanding CSS for
each reader. Identical reads may reuse one encoded batch while the exact capture
generation and event sequence remain current. The worker retains at most 2.8 MB
of rewritten encoded batches, keyed by browser, tab, frame, capture and control
identity, document URL, asset revision and exact batch content. Capture/control
changes, recorder stop and asset changes invalidate this reuse. It never supplies
an old page after a privacy fence. A read-only `observation_unavailable` may be
retried within the original mirror deadline with fresh authority checks; an
`operation_timeout` remains an uncertain observation and is not retried.

The worker must never accept untrusted `config`, `policy`, executable paths or filesystem roots from an agent. Private/localhost browsing requires an explicit trusted origin grant even when public web access is enabled. CONNECT tunnels retain origin/address containment, not inspection of encrypted application messages. Browser profiles are temporary, never the user's ordinary browser profile. See the [public protocol](../../docs/host-browser-protocol.md) and [host-browser design](../../docs/host-browser.md).

Agent `inspect` accepts bounded control/text offsets and an observed child `frame`.
It returns visible role/label/state controls, fresh references, child frame bindings,
counts and explicit truncation/unsupported content. References expire on inspection,
DOM/control/document/tab/authority changes or after sixty seconds. The host compares
live control signatures as well as observed mutation versions; page state is untrusted.
Open shadow controls share one captured list with ordinary DOM controls, including
handle lookup and pagination. Each observed open shadow root has a mutation
observer; discovering a new root invalidates prior references. Hit testing follows
open shadow roots before accepting a target. Traversal examines at most 100000
elements and 32 open shadow levels. `controls_truncated` marks a partial count when
either bound is reached; `unsupported` states the limit. Child frames retain their
independent observed frame identity and existing frame bound.
An observation that exceeds the node budget withholds rendered text and effect
references instead of forcing layout of the entire oversized document. Its
`text_truncated` flag and `unsupported` entry state this limitation; the partial
control count does not assert that the rest of the page has no controls. The node
budget fixture uses a hidden 100001-node subtree so it measures traversal without
also requiring Chromium to format an enormous line of empty inline elements.
It also traps full-body rendered-text and control-geometry reads and requires that
neither trap runs after traversal truncates; hiding the subtree cannot conceal a
regression in the production bound.
`read` returns bounded element text, never input values. `diagnostics` returns error
counts only and clears them on control fences. Effects add typed `key`, native
`select`/`check`, `double_click`, reference-to-reference `drag`, and `history`
(back/forward/reload). These retain the existing journal, network policy and control
fences; they do not accept arbitrary evaluation/CDP or retry uncertain outcomes.

Concurrent `css_chunks_v1` reads with the exact same browser, context, page,
frame, document/control/capture epochs, capture generation, cursor and byte budget
share one in-flight recorder/CDP producer. At most four distinct producers are
retained. Each reader checks its own viewer/private authority before and after
waiting; encoded strings and chunk arrays are immutable. The first producer and
each caller retain their original deadlines, including the 750 ms browser
evaluation and 1 s CDP stages. No input is retried. Legacy mutable event arrays
remain independent. Completed producers are removed rather than cached here.
A late CDP call remains tracked through its actual session retirement even after
a caller times out; stop/privacy fencing cannot claim zero pending work early.
