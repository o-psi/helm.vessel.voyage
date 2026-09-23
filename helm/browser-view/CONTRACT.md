# Shared Voyage browser viewer

`viewer.mjs` and `viewer.css` are the single browser UI for Helm Web and the
native Helm loopback page. `mountBrowserViewer(root, options)` mounts the same
controls in either client. Helm presents and forwards typed operations; it does
not execute page actions or own the browser process. The mounted viewer holds no
credential store or persistent action journal.

`transport(operation)` sends one `HostBrowserOperation` over the authenticated
Vessel connection and returns `{status, value?}`. The adapter verifies the voyage
session and incarnation. `context()` supplies current `{incarnation, revision}`
for Start. A mount owns one attachment and one RTCPeerConnection at a time.
Unmounting or socket loss calls `disconnect()`/`dispose()`; it never closes the
Voyage browser or returns private control to the agent.

The viewer first reads status, starts an available stopped browser only when
opening the viewer, attaches, then requests a WebRTC offer. It answers over the
same authenticated socket after complete ICE gathering. The first decoded frame,
not an `ontrack` event alone, makes the browser interactive. A short ICE
disconnection is allowed to recover; an ended stream or missing frames shows a
recovery action. No page content or private input is logged or stored as model
history. Host supplied RTC configuration may include authorized TURN service;
there is no default STUN service or page supplied peer configuration.

The production UI gives the browser the larger share of desktop space, with
conversation beside it; the mobile browser occupies a full screen panel. Tabs,
address, Back/Forward, Reload/Stop, Fit/100% and private control are visible.
Capture, viewer disconnect and browser closure are secondary actions. Explicit
text composition supports paste and IME. Private control resizes the browser
viewport to the available stage. Website dialogs have immediate controls. The
viewer shows the last known status and a recovery action after an uncertain
effect; it never presents uncertainty as a stopped browser.

Status is authoritative: `{available, running, binding, mode, controller,
tabs, tab_details, page, viewport, dialog, agent_active, agent_action,
agent_cursor, input_sequence}`. Only the attached private controller may receive
private metadata. Every effect carries a new command UUID and the complete
observed binding. Input uses consecutive attachment local sequences. Queued
input is fenced by its original binding and rejected when that binding changes.
The dialog and Stop controls can interrupt a page action whose reply is blocked
by a native dialog or loading page. They retain consecutive input sequence and
ignore any older reply arriving after the interrupt. A timed out or rejected effect is not
replayed. A status read is the only automatic recovery operation.

The media peer is preserved through control and viewport changes for the same
browser/tab/attachment. The worker clears old pixels and excludes unauthorized
viewers before acknowledging a private takeover. A tab or browser identity
change replaces the peer. Page coordinates map decoded frame dimensions to the
authoritative browser CSS viewport; a receiver that adapts video resolution does
not change click targets.

`BrowserConnection` is the transport/media state machine, and `screenPoint`
maps input coordinates. `BrowserSession` and `videoPoint` remain export aliases
for existing adapters. `externalClose:true` lets a shell provide the panel close
button. Capture and annotation use the shared local frozen frame editor; draft
insertion requires a successful `onCapture(File)` response, and opening the
browser does not capture or submit an image.
