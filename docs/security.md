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
does not keep the canonical transcript. Remote Helm connections use scoped HTTPS
grants; there is no SSH account-authority transport.

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

The previous broad security and regression suites were removed. Focused checks
only establish their recorded scope; see [quality](quality.md) and the audit below.
Behavioral, native-platform and deployment evidence must be recorded separately;
never represent a skipped check as passing.


## First-release audit: 2026-09-08

**Release decision: not yet approved for unrestricted public deployment.** Five
confirmed findings are remediated below, with targeted Linux evidence. Public
deployment, exact release artifacts and native-platform verification remain open
release gates. Completion of this
source review is not a claim that the entire application or an installed service is
free of vulnerabilities. Track completion in
[#198](https://github.com/o-psi/voyage/issues/198).

### Scope and threat model

The baseline is `92b3c66327bdd052f0673f3398fcd39372bb8591`, with the separately
recorded security patch. Unrelated removal of `scripts/` and the former general
fixtures was preserved. The review covered the Linux Vessel HTTP gateway,
enrollment and attachment transports, operator routes, pairing and scoped grant
routing, corresponding runtime authorization, Helm's Vessel clients, provider
credential storage/configuration, installer service defaults and locked dependency
advisories. This was source review plus targeted local adverse-input experiments,
not exhaustive fuzzing or an independent penetration test.

Attackers considered: unauthenticated internet clients; clients with narrow or
revoked grants; hostile browser origins; a configured HTTP proxy; and unsafe
existing credential files. The local service account and its executing-host
configuration remain trusted. Arbitrary code already running as that account is
outside the cooperating-process isolation boundary. Public clients cannot be
trusted with local supervisor bearer credentials.

### Findings and remediation

| ID | Severity | Finding | Status |
| --- | --- | --- | --- |
| VSA-01 | High confidentiality impact; requires a configured proxy | Plaintext authenticated loopback requests could follow `HTTP_PROXY`/`ALL_PROXY` when `NO_PROXY` did not exclude loopback. | Fixed; isolated actual Helm and Vessel verification passed. |
| VSA-02 | Medium availability | Incomplete request bodies could indefinitely occupy all 64 process-gateway request slots. Other handlers also collected bodies before their own authorization without one shared receive deadline. | Fixed; reproduced before and verified after. Edge connection/rate limits remain required. |
| VSA-03 | Medium availability | Anonymous pairing failures exhausted a shared persisted budget also required for legitimate redemption and lost-response recovery. | Fixed; targeted real-function verification passed. |
| VSA-04 | Medium; host configuration prerequisite | Authenticated native provider endpoints can be configured as non-loopback HTTP. | Fixed: [#199](https://github.com/o-psi/voyage/issues/199); config and request-path refusals plus synthetic proxy checks passed. |
| VSA-05 | Medium local credential-storage hardening | OAuth cache reads accept existing unsafe files and are unbounded; non-Unix permission handling does not establish ACL protection. | Fixed on verified Linux paths: [#200](https://github.com/o-psi/voyage/issues/200); 15 synthetic CLI cases passed. Native platform evidence remains separate. |
| VSA-06 | Release verification gate | The actual public TLS proxy, denial-of-service controls, administrative endpoint exposure and upstream isolation have not been verified by this audit. | Open under #198. |

**VSA-01.** Reqwest enables environment/system proxies by default. Its proxy matcher
only honors configured exclusions; a literal loopback destination alone does not
exclude proxying. That sent the local bearer and request payload, potentially
including a scoped bearer, to the proxy. A leaked local bearer does not by itself
make the loopback listener remotely reachable, but the disclosure violates the
private transport boundary. Exclusively local clients now use `no_proxy()`.
Mixed local/remote clients disable proxies for literal-loopback endpoints while
retaining remote HTTPS proxy support. Relevant code:
[Helm local transport](https://github.com/o-psi/voyage/blob/main/helm/src/process_client/local.rs),
[scoped access](https://github.com/o-psi/voyage/blob/main/helm/src/process_client/access.rs),
[operator discovery](https://github.com/o-psi/voyage/blob/main/helm/src/voyage_client.rs), and
[Vessel forwarding](https://github.com/o-psi/voyage/blob/main/vessel/src/process/exchange.rs).

**VSA-02.** The baseline accepted 64 incomplete JSON bodies; a separate malformed
pairing request changed from HTTP 422 to 503 and remained unavailable after an
11-second wait. None of the stalled requests received a response. The new
[HTTP receive boundary](https://github.com/o-psi/voyage/blob/main/vessel/src/http_boundary.rs) applies to the complete
merged gateway router, admits at most 64 in-flight requests and allows ten seconds
total to collect a bounded body. It retains route-specific limits: 4 MiB for
process commands/events, 4 KiB for pairing and enrollment inspection, 16 KiB for
enrollment proofs/administration, and 256 KiB otherwise. Expired body collection
returns 408 before dispatch; oversized bodies return 413. The deadline does not
wrap an admitted command or an SSE response stream. Existing downstream command
receipts and uncertain-delivery behavior are unchanged. All gateway responses are
non-cacheable, and request tracing uses route templates rather than raw URL paths.

This is not a connection-level or distributed denial-of-service defense. A
sustained attacker can still compete for bounded capacity; incomplete TCP headers,
TLS negotiation and slow downstream readers need proxy/network controls. Request
capacity is separate from the existing 64-stream SSE bound and bounded WebSocket
transport. Vessel's deliberately uncapped authorized voyage creation is unchanged
(#157); grant creation rights only to principals trusted with the host's resource
and provider-use budget.

**VSA-03.** The shared 120-attempt/minute counter was consumed before invitation
lookup or secret verification. Anonymous clients could keep all valid redemptions
and exact retries blocked. [Pairing](https://github.com/o-psi/voyage/blob/main/vessel/src/process/pairing.rs) now applies
that counter to anonymous failures; a matching secret, principal and origin can
proceed despite its exhaustion. Eight incorrect attempts against a known
invitation still lock that invitation. Expiry, Vessel identity, command identity,
current grant state and revocation checks remain in force. The invitation ID and
code should both be distributed privately. This change does not replace edge rate
limiting or bound total disk/network cost under continuous traffic.

**VSA-04.** [Configuration validation](https://github.com/o-psi/voyage/blob/main/voyage/src/config.rs)
and the provider factory now reject non-loopback HTTP before credentials are
loaded. Each native transport checks again before its request so a direct provider
constructor cannot bypass validation. HTTPS and literal IPv4/IPv6 loopback HTTP
remain supported; userinfo, fragments, controls and malformed endpoints are
rejected without echoing the URL. Existing endpoint paths/query behavior and
redirect refusal remain. Loopback native provider requests use a cached proxy-free
client; remote HTTPS retains its configured proxy behavior. Checks covered all
four native provider families, including OAuth model, response, device and token
exchange entrypoints. No live account was used.

**VSA-05.** [OAuth storage](https://github.com/o-psi/voyage/blob/main/voyage/src/provider/chatgpt_oauth/storage.rs)
now reuses the existing native private-directory primitives: descriptor-relative
Unix operations and the existing Windows ACL implementation. Reads and imports
are limited to 64 KiB; loaded tokens must have valid required fields. Unsafe
existing directories/files, symlinks, hardlinks and non-regular files are rejected.
Save uses a lock and atomic private replacement instead of chmod-based repair;
errors omit credential contents. Logout atomically publishes a JSON `null`
tombstone, and status treats it as logged out. It does not delete the cache path or
revoke already issued upstream tokens. Login/import after logout remains supported.
These paths passed the Linux hostile-file fixture; native Windows/macOS behavior
is not established by that result.

### Verified controls and limits of the evidence

Source tracing found no concrete privilege escalation in the reviewed scoped
paths. Vessel denies nested grant wrappers and privileged configuration operations;
the owning runtime independently checks principal, rights, scope, expiry,
revocation and parent connection authority. Authorization is checked around queued
dispatch and before replies; active execution polls authority and requests
cancellation when it fails. This is not proof that already dispatched effects have
been undone. Terminal authority requires both terminal and execution rights.
Public event projection carries invalidation metadata rather than canonical text.

Enrollment uses signed, origin-bound proofs and bounded replay records. Its
credential is separate from process execution grants. Remote clients reject
redirects and require trusted HTTPS, with explicit literal-loopback development
exceptions. No TLS certificate-verification bypass was found in these paths.
The installer provisions an ordinary-user `local-serve` service with `UMask=0077`,
not a public HTTP listener. The default application policy is not OS isolation;
optional Linux subprocess isolation and its limitations remain as documented above.

Recorded local evidence lives under `.local-git/evidence/security-audit-198/`;
private synthetic fixture directories are named in the result records. Evidence
contains no live provider credentials. These one-off probes do not recreate the
removed general test suite:

- Locked development build of Vessel, Helm, Voyage and the protocol crate passed.
  The final build included concurrent uncommitted Helm UI changes and reported an
  unused-import warning in that unrelated work; it is not a clean release artifact.
- Strict Clippy for final Vessel and Voyage, all targets/features, passed. Earlier
  strict Helm Clippy passed before subsequent unrelated UI edits. Changed security
  files passed formatting and diff checks; full release validation remains separate.
- Before/after actual gateway probe: 64 stalled bodies held capacity before the
  patch; all 64 returned 408 after the patch, and normal rejection responses
  resumed. Foreign Origin, oversized pairing, unauthenticated operator access,
  health and unknown-route checks retained their expected statuses.
- Actual Helm and Vessel proxy fixture: local authenticated requests reached only
  the fake local supervisor; the fake configured proxy received zero requests.
  The fixture used synthetic local/scoped credentials and checked both layers.
- Stream/dispatch probe: a command completed after 11 seconds, and metadata SSE
  continued beyond 11 seconds before a synthetic supervisor revocation response
  ended the stream. Foreign Origin and duplicate authorization were rejected before
  forwarding. This checks gateway handling, not actual grant-store revocation.
- Pairing real-function probe: known-invitation lockout, anonymous rate limiting,
  valid redemption under exhausted anonymous budget, identical credential recovery,
  changed-command refusal and post-revocation refusal all passed.
- Provider URL probe: 60 provider/config URL cases plus defaults, no-auth
  compatibility, factory refusal and direct unsafe transport calls passed. Unsafe
  calls returned before I/O. All four native provider model clients sent synthetic
  credentials only to the fake loopback receiver, with zero fake-proxy requests.
- Actual Vessel auth CLI: 15 case groups covered private creation/import,
  overwrite refusal, logout/reimport, unsafe parent/file modes, cache and parent
  symlinks, hardlinks, FIFOs, directories, oversized files, malformed/incomplete
  tokens and unsafe import sources. No credential marker appeared in diagnostics.
  Wrong-owner rejection is source-traced; no other-user ownership fixture was run.

No live model usage, production load test, certificate deployment check, native
macOS/Windows verification or full UI acceptance was performed. Packaging and the
full quality runner were unavailable in this checkout because their scripts were
already deleted; they are not reported as passing. Existing archive artifacts were
preserved. The report stays in this already-listed security guide rather than
introducing an unlisted packaged document.

### Dependency advisory check

The [OSV version query API](https://google.github.io/osv.dev/post-v1-querybatch/)
was queried for all 412 registry package/version entries in the inspected lockfile.
The scan was refreshed after concurrent UI work added dependencies. The exact
lockfile SHA-256 and query timestamp are retained in `osv-results.json`.
Only [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141.html)
was returned: an informational unmaintained-package notice for `bincode 1.3.3`,
reachable through `syntect 5.3.0` in Helm. It is not reported as an exploitable
Vessel vulnerability; there is no patched version identified by that notice.
No other known advisory was returned for the queried versions. This does not
establish absence of unknown flaws, vendored/native vulnerabilities or later
advisories. Repeat the scan against the actual release lockfile.

### Public deployment acceptance still required

Before opening a release deployment to untrusted traffic, record the hostname,
proxy/tunnel implementation and versions, executing-host OS, exact installed binary
hashes and these checks with synthetic clients:

1. The public endpoint presents a valid trusted certificate and uses TLS 1.2 or
   newer. Plain HTTP must not accept credential-bearing API traffic. The Vessel
   HTTP listener and private supervisor are unreachable from the internet and LAN;
   only the intended same-host proxy/tunnel can reach the public gateway upstream.
2. The edge enforces bounded connection counts, header/body receive deadlines,
   body/header sizes and per-source rate limits. Trusted client-address handling
   must not accept arbitrary forwarded headers. Use separate administration limits
   or network restrictions so an anonymous flood cannot prevent operator recovery.
3. Allowlist only the required routes. Ordinary paired Helm connections need the
   four `/v1/vessel/command`, `/events`, `/pair` and `/pair/capabilities` routes
   under that prefix. Enrollment, compatibility workers, coordination, diagnostics,
   `/ui`, `/metrics` and readiness endpoints should be exposed only when required
   and with their appropriate authentication/network restrictions.
4. Verify pairing, wrong credentials, duplicate authentication headers, foreign
   origins, rights/scope refusal and revocation through the actual proxy. Confirm
   command-body limits and streaming updates beyond receive deadlines; disable
   response buffering/caching for SSE. Dropped replies must preserve exact retry
   identities and may not trigger automatic replay of uncertain effects.
5. Keep secrets out of proxy access logs, URL paths/queries and body capture. Use a
   randomly generated operator secret in a private service environment, scoped
   expiring credentials for clients, and current provider credentials only on the
   execution host. Treat a TLS-terminating tunnel operator as a trusted party able
   to observe plaintext requests; TLS does not hide credentials from that terminator.
6. Ship the verified fixes from #198, #199 and #200, and limit native support claims
   to verified platforms. Run the restored/replaced release packaging workflow separately and
   check the exact artifacts to be shipped. Do not substitute development-binary
   checks for release-artifact or deployed-service verification.

These requirements follow the separate access-control and transport obligations in
[OWASP REST security](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)
and the layered resource protections in
[OWASP denial-of-service guidance](https://cheatsheetseries.owasp.org/cheatsheets/Denial_of_Service_Cheat_Sheet.html).
They are acceptance criteria, not assertions that an existing proxy already meets
them. The audit requested deployment details; no target-specific sign-off is implied
without that evidence.
