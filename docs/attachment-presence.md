# Foreground attachment presence

Vessel can accept authenticated outbound Helm connections and report their
presence. Session browsing, remote execution, remote approvals and event replay
are separate capabilities and are not enabled by this mode.

Configure enrollment using [the attachment CLI guide](attachment-cli.md). Vessel
mounts `/v2/attachment` when `--attachment-directory` and `--public-origin` are
provided. Its listener must be a literal loopback address behind a TLS proxy on
the same host. Configure the proxy to support WebSocket upgrades. The canonical
HTTPS origin is authoritative; forwarded host headers do not change it. The
existing operator credential is required for enrollment administration and the
operator console. `--allow-insecure-loopback` permits HTTP only for explicit
literal loopback development origins.

After enrolling, start one foreground connection:

```sh
helm attachment --directory /absolute/path/to/identity connect
```

For an HTTP loopback development enrollment, also pass
`--allow-insecure-loopback`. The command uses the origin and identity already
bound in the private enrollment directory. It never loads provider configuration,
starts an agent, reads a session, opens an inbound Helm port, or prompts for input.
A second process cannot borrow the same exclusive enrollment identity.

On authentication, stdout receives one JSON record containing
`event: "attachment_connected"`, `mode: "presence_only"`, and opaque machine and
connection IDs. This record describes that connection's establishment, not future
availability or task acceptance. The process maintains heartbeats while running.
Ctrl-C, SIGTERM or SIGHUP requests bounded connection shutdown and exits with status
130. Enrollment remains unchanged. A network failure or server revocation exits
nonzero; run `connect` again to make a fresh authenticated attempt. No old command
queue, lease, or effect is replayed. This foreground command does not automatically
reconnect or install a service.

`helm attachment status` inspects local enrollment, not live server presence.
Stop the foreground process before using `detach`, `rotate` or `revoke`, which
need its exclusive identity lease. Offline detach prevents a fresh connection but
does not claim server revocation. A server-side revocation can terminate a live
connection while its offline local enrollment record is still unchanged.

The operator-authenticated `/v1/diagnostics` response reports `presence_only` and a
bounded `connections` array containing machine, owner, connection and epoch IDs.
Each entry is checked against its current authorization and heartbeat lease when
observed; it is not an authorization grant or a guarantee that the peer remains
connected. No session data, frame payload, credentials or human terminal input is
included. The public metrics endpoint reports only whether presence is configured.
With enrollment disabled, the attachment route is absent and diagnostics report
no connections. Remote execution remains unavailable unless separately enabled
with Vessel `--remote-execution` and Helm [remote-worker](remote-sessions.md).
That opt-in server reports `outbound_managed_sessions`; individual presence-only
connections still cannot receive execution commands.

The default presence server negotiates an empty event-feature set and has no application
frame consumer. Unexpected application frames close their connection without
acknowledging work. Server shutdown closes socket admission, pending authentication
and registered connections. Lease checks enforce expiry directly, even if a
background monitor has not run. The transport retains its finite handshake,
queue, socket and write limits described in [the transport contract](attachment-transport.md).
A blocked initial output pipe has a two-second reporting budget; the output
worker never owns enrollment or its lease.

The Linux system fixture exercises actual binaries through disabled/enabled
routing, enrollment, lease renewal, exclusive identity use, explicit reconnection,
server restart, signal shutdown, blocked output, server revocation and offline
detach. Independent library regressions cover stale leases, revocation, owner
mismatches, generation isolation and shutdown racing with authentication. TLS
proxy deployment and native macOS/Windows behavior are not claimed by those local
fixtures. Dedicated foreground managed dispatch is documented separately in
[remote sessions](remote-sessions.md); broader sharing and services remain open
under #9, #10, #78 and #79.
