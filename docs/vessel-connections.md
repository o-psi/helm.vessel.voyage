# Connecting to Vessels inside Helm

Helm's **Vessels** panel manages human connections independently of conversation
text and model-driven coordination. A connection identifies where work executes;
it is not a conversation, a provider account, or permission to run as root.

## Open and use the manager

Start Helm normally. Click **Vessels**, press **Ctrl+G**, or enter `/vessels`.
The control remains available in narrow layouts where the conversation sidebar
is hidden. The panel supports keyboard selection and mouse input; Escape returns
to the original conversation or draft.

The panel provides:

- **Add HTTPS + pairing** for an owner-approved workspace connection.
- **Import existing access file** for an existing credential, including restricted
  conversation sharing.
- Connect/Retry, Disconnect, friendly-name changes and automatic reconnect.
- Renew/Replace access, Forget, and recovery of retained connection records.
- New conversation on the selected Vessel and filters for the conversation list.

Saved connections are loaded by normal interactive Helm startup. Remote discovery
runs in the background; an unavailable remote does not prevent local work. Explicit
CLI subcommands remain single-target and do not implicitly fan out to saved Vessels.

The executing Vessel is shown beside the conversation/draft. New remote
conversations use a workspace selected from that Vessel's approved list. A local
working directory is never silently substituted for a remote workspace. Choosing
a workspace creates only a private draft; the first send retains the existing
separate, deduplicated create-and-submit workflow.

## Pair a workspace connection

1. Open **Vessels → Add HTTPS + pairing** and enter the HTTPS origin, such as
   `https://vessel.example.com`.
2. The panel displays this Helm installation's non-secret **principal UUID**.
   Give it to the executing account's owner so the invitation is bound to the
   intended principal.
3. The owner issues a short-lived invitation on the Vessel host, approving exact
   workspace directories and rights. The local administrative command is:

   ```sh
   vessel pair-invite \
     --directory /absolute/private/vessel-state \
     --endpoint https://vessel.example.com \
     --principal HELM_PRINCIPAL_UUID \
     --workspace /absolute/approved-project \
     --rights catalogue,create,observe,history,execute,steer,decide,cancel \
     --output /absolute/private/invitation.json
   ```

   Repeat `--workspace` for additional approved directories. Run this as the
   account that owns the supervisor state, not as a different administrative
   account. The output parent must be private and owned; the file must be new.
   The command prints the output path, not the secret.
4. Transfer the invitation through a private authenticated channel. Its private
   JSON contains `invitation_id` and `code`; enter them as
   `invitation_id.code` in the panel's masked invitation field. Do not put the
   code in a conversation, command argument, issue, or log.
5. Validate and review the authenticated Vessel identity, workspace scope, rights
   and expiry. Choose a friendly name and whether to reconnect automatically,
   then explicitly save the reviewed access.

The principal is public metadata, not a password. The invitation is secret,
one-time and principal-bound. Invitations default to ten minutes and cannot exceed
fifteen minutes (`--ttl-seconds`). A redeemed connection grant lasts thirty days.
Failed attempts are bounded. After an uncertain response, use the panel's pending
pairing recovery; Helm retains the original command identity instead of issuing a
second redemption. Successfully redeemed invitations can recover that original
credential after invitation expiry, but not after grant revocation or expiry.

The owner's default rights are `catalogue,create,observe,history,execute`.
The example above additionally allows steering, human decisions and cancellation.
`decide` is needed to answer runtime questions/approvals. Add `lifecycle` only when
archive/delete and process lifecycle controls are intended; add `terminal` only
when private terminal attachment is intended. Terminal input also requires
`execute`. These permissions never bypass the runtime's tool policy, workspace
limits or local account permissions.

Pairing uses the executing host's configuration and provider credentials. It does
not permit arbitrary configuration-file paths, administrator policy changes,
participant-session takeover or provider-token copying. Unknown provider readiness
is reported as unconfirmed; actual account/model availability requires host setup
and, separately, an explicitly authorized inference check.

## Import existing conversation access

Use **Import existing access file** and select the private credential path. Helm
validates and copies the exact credential into its own private storage before
saving the reviewed connection, so the original download can subsequently be
moved without breaking the saved connection.

Legacy credentials remain **Shared conversation** access. Only the granted
session is available; importing does not create workspace-wide authority. Obtain a
new owner-approved pairing invitation for additional conversations. Existing
startup `--access-file` routes and their drafts can be explicitly imported without
changing their original delivery identities. Old SSH drafts are preserved but
never automatically mapped to HTTPS.

## Disconnect, replace and recover

**Disconnect** stops this Helm's observation. It does not stop or cancel accepted
remote work. Reconnect retrieves state and resolves retained command receipts; it
never replays a prompt or private terminal input automatically.
If first creation or submission never reached admission, supported servers can
durably close the original command ID and restore editable text. If creation is
confirmed but the first turn was never attempted, press Enter to continue
explicitly. Unconfirmed legacy admission remains fenced rather than guessed.

**Renew/Replace** reviews new access as a distinct connection. It cannot silently
change the principal, endpoint, scope or credentials associated with an old pending
operation. The old recovery records remain available. A changed Vessel identity
requires explicit new review, not automatic repinning.

**Forget** is local connection management, not server revocation or conversation
deletion. Helm retains private recovery material and does not claim that unfinished
deliveries disappeared. Use retained-connection recovery to restore the original
identity when needed. Server revocation is an explicit executing-owner operation:

```sh
vessel revoke-connection \
  --directory /absolute/private/vessel-state \
  --grant GRANT_UUID --expected-revision OBSERVED_REVISION \
  --command-id NEW_COMMAND_UUID
```

Retain that command ID and exact arguments if its outcome is uncertain. Revocation
and expiry invalidate subsequent admissions and the existing runtime authority
watcher; cleanup still requires actual observation. Disconnecting is deliberately
different from revoking execution authority.

## Storage and trust boundaries

The manager uses the private `helm-connections` directory alongside the local
Vessel directory (normally `~/.local/state/voyage/helm-connections`). Its versioned
address book, private immutable credentials, pending pairing records and recovery
records are separate from provider configuration. Updates use locks, per-record
revision checks and atomic writes; unsafe ownership, permissions or symlinks are
refused rather than silently repaired.

Human connections do not automatically populate `[vessel.remotes]` or grant a
model coordination authority. That configuration belongs to the executing host
and remains a separate explicit decision.

Use trusted HTTPS. Credentials in URLs, HTTP redirects and certificate-verification
bypasses are not supported. Literal-loopback HTTP remains a development exception,
not a LAN transport. With Cloudflare Tunnel, TLS can terminate at Cloudflare and
forward to the Vessel gateway's private loopback HTTP listener. An interactive
Cloudflare Access login page is not a supported Helm authentication method; the
Vessel credential authenticates API access. Preserve POST command traffic and
unbuffered SSE at `/v1/vessel/command` and `/v1/vessel/events`; pairing also needs
`/v1/vessel/pair` and `/v1/vessel/pair/capabilities`.

Vessel and native private-storage support remain Linux-focused. A build or local
fixture is not evidence for native macOS/Windows operation or arbitrary reverse
proxy configurations. See the implementation's recorded verification for actual
platform and deployment evidence.
