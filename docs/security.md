# Authority and security

This guide distinguishes requirements of the [target architecture](architecture.md)
from the [current implementation](current-state.md). Configuration and commands for
the existing binaries are in [configuration](configuration.md) and
[operations](operations.md).

## Target authority boundary

Helm presents actions and decisions; it does not execute tools or supply execution
authority. Vessel authenticates clients and supervises voyage processes within
locally configured limits. Each voyage process enforces the applicable local
policy before providers, tools, subprocesses, subagents and disclosure boundaries.
A supervisor's routing decision is not a substitute for runtime admission.

Authority is the intersection of the authenticated principal's capabilities,
current session access, accepted Vessel membership, local grants, policy ceilings,
resource limits and any required exact-action approval. Discovery, enrollment,
feature negotiation and saved configuration drafts confer none of these grants
by themselves. A remote client cannot supply a path or provider credential to
replace a machine's locally accepted execution binding.

Use private local IPC with OS-user and endpoint ownership checks. Authenticate
runtime incarnation, not just PID or socket name. Network control uses authenticated,
versioned and bounded messages over protected transport. The inter-Vessel topology
still needs implementation design; exposing an unauthenticated runtime task port
is not a shortcut. Helm remains a client in that design.

## Credentials and disclosure

Provider credentials belong to the machine and runtime performing the work.
Vessel may arrange protected local configuration for its child voyage processes;
credentials must not be centralized in another Vessel, forwarded between machines,
put in supervisor metadata or delivered to the model. Reconnecting from another
Helm does not change the voyage's provider account or billing boundary.

Separate provider credentials, client authentication, enrollment and session-sharing
grants. Use protected local credential storage or named environment bindings.
Never put secrets in URLs, command arguments, published fixtures or documentation.
Treat stored provider continuation as private local runtime data. Native transports
and the optional external compatibility bridge remain distinct capabilities.

Authorize catalogue, history, event replay, decisions and mutation separately as
needed. Do not reveal existing private sessions merely because a client connects
to a Vessel. A participant receives only the context permitted by the voyage's
sharing policy and the local grant. Notification and diagnostic payloads should
contain the least metadata required for their purpose.

Revocation must stop new authorized reads, publication and dispatch at a defined
local ordering point. Already delivered bytes and already dispatched effects
cannot be recalled. Expose pending remote enforcement and cleanup honestly. Do not
claim retention guarantees for backups or external copies that are not enforced.

## Connected account authority

The Linux process route authenticates a private per-service bearer token and accepts
it only on a literal-loopback HTTP listener. The discovery record must remain an
owned private regular file under the owned private Vessel directory. Browser Origin
requests, redirects and non-loopback endpoints are rejected. SSE subscriptions are
bounded and carry only durable invalidations; canonical history requires its
separate right. Vessel forwards with a session/incarnation-bound runtime secret and
does not keep the canonical transcript. The SSH compatibility adapter invokes the
remote account's local client with noninteractive authentication and agent
forwarding disabled. This provides the remote account's local authority, not an
enrolled per-session grant.

Pending runtime decisions have exact targeting, durable receipts, single-response
semantics and bounded expiry. Local policy remains the execution ceiling. Scoped process grants bind principal, workspace, session, rights, revision and
expiry; current grant/enrollment state is checked at dispatch and during execution.
Private credential files are separate from provider keys. History and responder
authority are distinct rights; see [process access](process-access.md). Shared
OS accounts and arbitrary code running as that user remain within the cooperating
process trust boundary.

## Current controls

Execution checks live in the voyage process. All Helm session execution entrypoints
reach that supervised owner. Native providers do not require
the Codex executable. API-key and subscription credentials use distinct configured
transports; selecting a model transport does not grant tool authority.

The current access modes are read-only, approval and unrestricted. Unrestricted
removes ordinary approval prompts but retains roots, explicit command denials and
special exact-review publication requirements. Unattended work follows the
configured unattended policy and cannot silently assume interactive approval.
The current dedicated remote worker has additional capability restrictions; see
[operations](operations.md).

Filesystem tools check canonical allowed roots. Subprocesses use the configured
filtered environment. Linux policy resolution checks the administrator ceiling;
do not infer equivalent enforcement on another platform. Profile escalation requires
fresh exact review, and a saved conversation cannot restore an old policy grant.
These remain application controls when `[sandbox].mode = "off"` (the default).
Optional required isolation adds a Linux x86_64 bubblewrap boundary at subprocess
launch in Voyage, independently of approvals. It uses pinned root mounts,
namespaces, seccomp, filtered environments and inherited resource limits, and
refuses launch when setup fails. See [configuration](configuration.md#optional-linux-process-isolation)
for the explicit network grants, per-process and same-UID limit scopes, separate
compatibility-bridge grants and local diagnostic probe. Native HTTP provider
requests are not tool subprocesses and retain their executing-host credentials.
This implementation does not establish macOS or Windows containment, an aggregate
descendant budget, or an endpoint network allowlist.
New subprocess launches narrow their pinned mounts when the current dispatch is
read-only. Changing access does not retroactively remount an already-running
process; stop it before relying on a narrower OS boundary for that process.

The current worker connects outbound to Vessel. Do not expose a new inbound Helm
task port or forward its provider credentials. Enrollment/presence alone do not
share sessions or start execution. The remote HTTP surface requires an explicitly
started dedicated worker and current authorization. Its connection-loss and
withdrawal behavior must remain intact until deliberately replaced by the new
supervision contract.

## Untrusted content and private terminal input

Repository content, remote responses, tool results and retrieved documents are
untrusted input. They cannot override configured authority or become runtime system
instructions. The live tool registry defines available model tools; do not infer
provider-host tools or hidden capabilities.

Preserve canonical conversation text while sanitizing it for terminal presentation.
Keep raw subprocess/provider diagnostics out of full-screen TUI output. Direct
human PTY keystrokes remain private terminal input and must not enter conversation,
model-visible tool records or general event replay. Approval paste must not count
as approval; cancellation must remain bound to the intended run.

## Recovery and operational limits

Keep private storage owner-only and reject unexpected symlinks or incompatible
storage identities where the implementation requires it. Current native Windows
private-storage primitives do not establish native validation of all execution or
service paths. Local filesystem locks assume their documented cooperating-process
boundary; they are not protection against arbitrary code running as the same user.

A process death or failed cleanup leaves an obligation, not permission to retry an
external effect. Preserve attribution when a human attests cleanup. Restoring a
backup must not revive stale runtime ownership or revoked grants. Inspect current
authority and live processes before resuming work.

Automated security and regression suites are currently absent. The [quality
checks](quality.md) only establish their stated build, analysis and packaging
results. Behavioral, native-platform and deployment evidence must be recorded
separately; never represent a skipped check as passing.
