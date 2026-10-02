# Native host-browser adapter source cases (#353)

This batch prepares ordinary Helm coverage cases for the production Native
viewer adapter and TUI lifecycle. The maintained source modules are
`helm/src/process_client/host_browser_lifecycle_tests.rs` and
`helm/src/process_client/ui/browser_adapter_lifecycle_tests.rs`. The only changes
to production source files are their `cfg(all(test, unix))` module declarations.

## Scope and evidence

The 15 adapter cases start the actual `Handle::start` / Axum server and create
its actual one-use private launcher. Requests use loopback HTTP with no proxy or
redirect. The separate Vessel side is the existing scripted WebSocket `Peer`;
no real Vessel, independent Voyage, Chromium or provider is launched. These
cases assert the exact session, owner, command ID, revision, operation and
binding crossing that socket, rather than claiming synthetic replies prove a
production browser's effects.

The prepared cases cover:

- Private launcher ownership/modes, exact Host and Origin, one-use concurrent
  bootstrap, alive authorization, security headers, actual assets and the
  3 MiB router body limit before transport dispatch.
- Explicit Start preparation with the same command ID and observed revision,
  original/prepared incarnation acceptance, and refusal of arbitrary owners.
- Native control modes, explicit Close, a bounded queue, no implicit start/close
  without an attachment, and full exact-current-binding Detach on cleanup,
  including all owner/browser/attachment/tab identities and updated epochs.
- Nonclaim input serialization with explicit private control, claim input,
  dialog and navigation Stop interrupting the gate. Invalid/foreign intents
  never reach the socket.
- Socket retirement during pending Start, read-only mirror refusal, unknown
  HTTP input/native control outcomes, and a dropped HTTP observer followed by
  an original late reply: no effect replay or automatic replacement connection.
- Actual local server retirement, launcher and unique directory removal,
  Handle Drop, idempotent finish and missing-socket startup refusal. Cleanup
  checks do not convert an unknown remote effect into success.

The nine App cases exercise the actual browser commands, input handling and
target lifetime using the same maintained synthetic App and loopback Peer.
They cover pre-effect open guards; same-target reuse; panel swallowing Paste /
Enter until one Esc; retained drafts; F6 reuse; background panel updates that
preserve selection and other panels; owner/lifecycle retirement; route-scoped
viewer cancellation; explicit detach completion before a new launcher; and
the four-viewer bound including actual pending Handle cleanup: a real fourth
viewer emits Detach, its reply is held while three active viewers plus that
retiring resource refuse another viewer, and admission resumes only after the
original reply and observed launcher removal / Handle.finish retirement.
The relative opener-path
case refuses before OS process creation. Valid desktop openers are never run.

Every created adapter directory is the production code's unique `host-view-*`
directory under the normal executing user's state root. Tests retain their
exact launcher paths and assert those files/directories are removed; they do
not enumerate or alter other state, change HOME, read real credentials or
relax filesystem/runtime checks. HTTP, peer observation and cleanup have
explicit finite deadlines. The application runtime's 15-second bound stays
unchanged; the dropped HTTP observer is an intentionally shorter fixture-side
observation, with no transport replay.
Successful cleanup assertions establish observed removal of their owned
resources. On an assertion panic, fixture Drop only requests stop/disconnect;
that cancellation is not positive failure-cleanup evidence.

## Measurement and remaining obligations

Prepared against source `182b188664bd150321758f1c6ed19838f952a1f1`; the two
production modules are unchanged in measured source
`db3d358e2a51db488c850be978853e18a4063a14`. That source's corrected #353 map
identifies 205 uncovered adapter addresses and 109 TUI browser addresses.
Those counts describe the input map, not a coverage gain. Source formatting
and diff checks are complete. Tests and workspace coverage await the next
coordinated gate; do not describe these prepared cases as passing.

The standalone `run_connected` CLI / actual desktop opener path and deployed
public WSS, TLS, browser/site/media and client-cost qualification are separate
obligations. The real #333 production driver still requires an existing
legitimate Native public credential and the authorized deployed fixture run.
No lines or targets are excluded from coverage, no platform is classified as
unreachable by these tests, and loopback results do not qualify native
macOS/Windows or an actual website/browser process.


## Observed shutdown correction from the first full gate

Frozen source `78ff047c63114ae972b1e6aa5868ba957814d5b8` failed six Helm
parents. Four App helpers omitted the required Snapshot `model` fixture field.
The mirror-refusal and queued-control cases exposed a production lifetime race:
requested Handle cancellation also completes Axum graceful shutdown, so an
unbiased select could treat that healthy completion as unexpected. The cleanup
tail also skipped inspecting a server result that was already finished.

[Recorded defect](https://github.com/o-psi/helm.vessel.voyage/issues/353#issuecomment-5947330710).
The correction retains any selected join/I/O completion exactly once, accepts
only requested healthy completion in the select branch, and inspects even an
already-finished server during cleanup. Bounded explicit abort retirement is
distinct from graceful completion; unsolicited completion or I/O/join errors
remain errors. Original socket, full Detach binding, private path removal, queue
and unknown-effect/no-replay assertions remain unchanged. Two prepared boundary
cases use actual Tokio completions to cover healthy/cancelled/failed outcomes.
These changes are source-only; focused and fresh full verification are pending.
The failed full log/report remains retained and is not a passing measurement.
