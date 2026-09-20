# Shared executing-host browser viewer adapter

Import `mountBrowserViewer(root, {transport, context, ...})` from `viewer.mjs` and
serve `viewer.css`. Framework-free DOM/ES modules, shared by Helm Web and the native
TUI's loopback static viewer. No browser companion, credential store, logging or
persistent journal. Native adapters must expose only their authenticated private
socket transport, not a general HTTP browser executor.

`transport(operation): Promise<{status, value?}>` sends exactly one
`HostBrowserOperation` over the already authenticated Vessel full-duplex socket.
Reject outer errors/unknown outcomes; do not retry effects automatically. The
adapter unwraps the Voyage response and verifies session/incarnation. Never log,
persist or pass SDP/input/private payloads to model-visible history.
`context()` returns `{incarnation, revision}` for Start. Recreate/dispose the viewer
on socket replacement, voyage change, incarnation change, or disconnect. Call
`disconnect()` immediately on transport loss, and `dispose()` when unmounting.

Status is `{available, running, binding, mode, controller, tabs}`. Binding is the
full protocol HostBrowserBinding; attachment_id is nil before Attach; the viewer supplies a fresh non-nil UUID
on Attach and uses the authoritative reply thereafter. Controller is an attachment UUID. Tabs are UUIDs, not page titles.
All effects carry fresh UUIDs; input uses consecutive attachment-local sequence.
Every input is fenced by the complete binding captured when queued; stale queued
input is dropped, never rebound. A bounded queue refuses/clears on slow transport.
Status is polled while mounted; capture/control/identity changes clear video and invalidate pending callbacks.
Document/viewport/tab input fences drop queued input without tearing down media. Remote mode is authoritative; UI does not grant itself control.

Viewer initiates RequestOffer. `value:{type:'offer',sdp}` is set as remote
description; createAnswer/setLocalDescription then wait for complete ICE gathering
before sending Answer. No trickle ICE. No default STUN. `rtcConfiguration` may be
supplied only by a trusted native/host adapter, never page content. ontrack attaches
a real MediaStream to a muted autoplay inline video. Connection failure clears it;
reconnect is explicit. Private mode clears previously displayed frames immediately
on transition; no automatic return to agent, automatic reconnect, or input replay.

Exports also include `BrowserSession` (transport/controller state without DOM),
`videoPoint` (letterbox-aware source coordinates). Mount returns the session plus
`dispose()`. Controls include Start/Connect, human/private/agent mode, navigation,
tabs, remote dialog accept/dismiss, viewport resize, pointer/wheel, physical keys,
and a transient IME text composer. No screenshots are substituted for video.

A remount on an existing attachment takes `status.input_sequence` as its starting
sequence; it never guesses zero. Explicit viewer detach/disconnect is best effort
and never resumes private mode or closes the browser. Host-authored
`value.rtc_configuration` in a RequestOffer reply overrides the local receiver
configuration; it is private signaling, not status, history, or model input.
