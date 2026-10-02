# Browser opening intent and reconnect

This correction belongs to [#333](https://github.com/o-psi/helm.vessel.voyage/issues/333)
and the [v1.0.3 release record](https://github.com/o-psi/helm.vessel.voyage/issues/375).
The shared viewer now accepts `startOnConnect`, defaulting to `true` for existing
clients. A client supplying `false` observes browser status and attaches to a
running browser, but does not start a stopped browser. An explicit
`connect({start:true})`, including the existing **Start browser** action, still
requires the current owner/revision and a fresh command identity.

Helm Web consumes one selected-voyage opening intent before asynchronous viewer
connection. Incidental React remounts on socket renewal, owner-incarnation change,
connection recovery or viewport replacement receive no new start intent. Closing
the dock detaches the viewer and preserves a running Voyage-owned browser. A new
explicit dock opening may start a stopped browser once. The Web shared viewer
snapshot must match this core asset when delivering the coordinated change.

The known source path is distinct from attribution of a particular native event.
The old Web mount effect remounted on socket and owner changes, and the old shared
viewer connected with `start:true` on every mount. A status observation of a cleanly
suspended Voyage may prepare a new owner incarnation without starting a browser;
a subsequent mounted viewer could then issue Start. Runtime status itself does
not call browser start. Native receipt identity and chronology remain necessary
to establish which path caused an observed replacement browser.

`voyage/tests/host_browser_viewer_start.test.mjs` supplies five source contracts:
observation-only connection to a stopped browser, attachment to an existing
browser, explicit Start with exact fences, preserved default/native opening, and
disposal during an outstanding status read refusing a late Start. These use the
real shared connection state machine with an owned scripted transport and no
browser, provider, network or process effects. Web's React regression drives
actual dock controls through close, socket/owner remount, disconnect/reconnect,
explicit Start and fresh opening, while requiring no additional Start during
incidental transitions.

Source preparation is not passing evidence. Root owns the coordinated local
checks and subsequent hosted asset/build, Web deployment, artifact identity and
native qualification. This correction neither changes browser process authority
nor proves the complete browser release gate.
