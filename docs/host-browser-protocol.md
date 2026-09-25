# Host browser protocol

The source of truth is [host_browser.rs](../crates/voyage-protocol/src/host_browser.rs). This is the Voyage-owned browser on the Vessel host, separate from the opt-in local Browser executor. Both Helm clients send `VoyageCommand::HostBrowser` over their authenticated duplex Vessel socket. Vessel attaches private `HostBrowserSocket` provenance and the existing process grant; public JSON cannot choose a socket or principal. The runtime checks grant rights and revocation on every operation.

`HostBrowserBinding` carries `incarnation`, `browser_id`, `attachment_id`, `tab_id` and document, viewport, controller and capture epochs. All IDs must be non-nil for a bound operation and all epochs positive. Status may return a nil attachment ID before Attach. IDs are fences, never credentials. Socket loss delivers a private cleanup notification, invalidates its attachments and does not resume agent control.

| Operation | Right | Effect |
| --- | --- | --- |
| `status` | Observe | Read bounded browser status. |
| `start` | Execute | Launch with exact incarnation and expected revision. |
| `attach`, `detach` | Observe | Admit or remove one viewer on its authenticated socket. |
| `mirror {binding,since}` | Observe | Read compressed DOM events and cursor; no mutation ID or durable page content. |
| `control {binding,mode}` | Execute | Select agent, human or private authority with a control fence. |
| `input {binding,sequence,input}` | Execute | Apply one typed human action with a consecutive attachment sequence. |
| `receipt` | Observe | Query an exact command identity without cached secret payload. |
| `close` | Execute | Close the supervised browser and observe cleanup. |

Input variants are `history`, `click`, `surface_click`, `fill`, `select`, `wheel`, `upload`, `download`, `key`, `text`, `scroll`, `resize`, `dialog`, `navigate` and `tab`. Element actions use a positive recorder node ID. A surface click uses integer x/y fractions from 0 to 10,000 within a canvas/video/iframe element. Text is at most 16 KiB, uploads and returned downloads at most 2 MiB, navigation HTTP(S) only, and resizing 320–3840 by 240–2160. Structural validation is followed by live worker checks for authority, network policy and element state.

The mirror reply contains `{encoding:"gzip",data_base64,cursor,reset,latest,visuals}`. The gzip body is a bounded rrweb event array. `reset` requires a full snapshot; subsequent reads use `since=cursor`. A stale cursor, page change or control change resynchronizes instead of replaying input. Visual fallback items carry an element ID, position, dimensions and bounded JPEG bytes. The task page has no direct Helm channel. The replay iframe has no website script execution or network authority.

Effect IDs have exact deduplication. Input uses a bounded live receipt ledger plus runtime tombstones. A timed out or disconnected effect can be unknown, never safe to retry automatically. The worker's private protocol is documented in [its contract](../voyage/browser/CONTRACT.md).
