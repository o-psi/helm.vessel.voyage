# Share a local browser with a Voyage

The browser runs **on the computer running Helm**, even when its Vessel and Voyage
are remote. Websites use that computer's network. The local companion and the
agent operate one Chromium session, not two browsers following the same URL.

Helm sends browser observations and receives browser actions through its existing
[full-duplex Vessel socket](duplex-transport.md). The local browser adapter has no
Vessel connection, remote browser endpoint, pairing flow or agent loop. Its only
control connection is private local stdio to Helm. The companion's display and
human input remain local; they do not make a trip through the remote server.

## Setup on the Helm computer

Install Node.js 24 or newer, npm and Chromium locally. Then explicitly run:

```sh
helm browser setup
helm browser status
```

Setup installs the bundled lockfile-pinned `playwright-core` dependency using npm
with lifecycle scripts disabled. It does not download a browser, connect to a
provider or share anything. Assets are versioned by their bundled content digest
under the private `helm-browser` state directory. Existing altered assets are
refused, not silently replaced. Updating Helm can require another explicit setup
for its new asset version; old profiles/evidence are not deleted.

Chromium must support its sandbox. Helm does not fall back to `--no-sandbox`.
If Chromium is not discoverable on the local PATH, set `HELM_BROWSER_CHROMIUM` to
its absolute executable path in Helm's local environment. This setting never
comes from a remote tool argument. Current desktop/process evidence is Linux;
native macOS/Windows behavior has not been qualified.

## Open and share

1. Connect Helm to the intended Vessel and select an existing voyage.
2. Press **F6** or enter **`/browser`**. Helm creates a dedicated local browser
   resource, initially private, and opens its local companion.
3. If the desktop opener is unavailable, open the private **local launcher file**
   shown in the Browser panel. It carries a one-use local bootstrap internally;
   do not publish or send the launcher to the model.
4. In the companion, review the target voyage, add the exact website origins you
   intend to use, and navigate. Private-network access is a separate explicit
   choice. There are no default origin grants.
5. Choose **Share / Return to agent** only after reviewing the disclosure: the
   entire shared browser, including signed-in page text and requested screenshots,
   can become remote Voyage history and model input.
6. Send the browser task in Helm. Agent effects still need the companion's local
   confirmation. Remote policy can impose additional approval or refuse them.

For a separate human browser frontend associated with an existing voyage:

```sh
helm connect --access-file /absolute/private/connection.json browser SESSION_UUID
# Or select a local Vessel:
helm connect --directory /absolute/private/vessel browser SESSION_UUID
```

That frontend also uses one authenticated socket for its commands and browser
traffic. It is not a browser bridge or a second connection belonging to the TUI.
Do not start it alongside the TUI for the same browser expecting merged controller
ownership. Each explicitly opened browser has its own executor and local consent.

A browser belongs to its selected voyage and connection. Switching conversations
never redirects it. At most four browser resources can be open in one TUI. Other
Helm clients cannot silently inherit the resource or steal its local controller.
A second local companion tab must wait for or explicitly release the current
controller; closing a tab is not return-to-agent consent.

## Human and private control

- **Take over** fences queued agent actions and new observations immediately at
  the local adapter. Already-dispatched actions can have effects; wait for its
  quiescent human-input state rather than assuming a click was undone.
- **Private** suspends agent text/DOM, screenshots and disclosure. Local human
  viewing/input remains available. Traces, HAR, video and console/network capture
  are not enabled. Private input never enters the model's tool arguments.
- **Return to agent** is a local companion action, not a remote command or chat
  approval. It advances control/capture epochs and requires fresh element
  observations. Old queued actions and element references cannot resume.

TUI controls:

| Command | Action |
| --- | --- |
| `/browser` or `/browser open` | Open the selected voyage's local companion/panel. |
| `/browser status` | Show local resource state without capturing the page. |
| `/browser takeover` | Request immediate local human control. |
| `/browser private` | Request the local privacy fence. |
| `/browser close` | Close the owned local browser, not the Voyage. |
| `/browser reconcile` | Submit retained positive local cleanup evidence; never repeat browser effects. |

The local bootstrap secret is not placed in process arguments: Helm opens an
owned private launcher file, which redirects to the local companion. Its secret
is consumed into a private local browser session and removed from the address bar.
Do not copy the companion's private session files or cookies into a conversation.

## Files and credentials

Local uploads use the companion's explicit file picker and grant. A remote
workspace upload first passes Voyage's allowed-root/read and disclosure policy,
then transfers bounded bytes over the same socket to locally reviewed staging.
The subsequent website upload needs its own target/local confirmation. Remote
paths are never interpreted as local filesystem paths.

Downloads remain untrusted in private local staging. Saving locally and disclosing
to Voyage are separate choices. No download is automatically opened or executed,
and a remote result is an immutable session artifact rather than a caller-chosen
filesystem write. File/raster transfers are limited to 2 MiB each. The executing Voyage's configured
`max_output_bytes` (including encoded result overhead) and artifact-store budgets
can impose a lower limit; sharing cannot override them. Large transfers
fail explicitly rather than silently truncate. Staging is removed on clean close;
profiles and minimal dispatch receipts are retained.

Browser cookies and login entry stay local. Provider credentials remain on the
executing Voyage host. **Local browser placement is not a promise that page data
stays local:** permitted observations cross to Voyage and its configured provider.
Privacy cannot recall already delivered data, hide credentials from the website
receiving them, or sanitize all later authenticated page content automatically.

The [visual tool-result contract](visual-tool-results.md) preserves tool-call
provenance and validates actual pixels. Responses, native ChatGPT OAuth and
Anthropic have image-bearing tool encodings; text-only Chat tool output is
explicitly unsupported. Selected model capabilities and request-wide image bounds
still apply. No automatic provider/account switch occurs.

## Disconnect, failures and recovery

A share is pinned to one socket identity and loss generation. Any socket loss,
credential failure or explicit disconnect fences it; a successful reconnect does
not restore sharing. The local watchdog also fences after missed liveness. These
are bounded detection mechanisms, not instantaneous detection of an undetectable
network partition. The remote Voyage stays alive and may perform other permitted
work.

A request UUID and payload digest are recorded before dispatch. Duplicate requests
never repeat an effect. Lost replies, cancelled in-flight actions and crashes can
leave an **unknown effect**. A website does not participate in Helm's receipt
transaction: exactly-once purchases/submissions cannot be promised.

Close the browser, then use `/browser reconcile` for evidence from this local
installation, or `helm connect --access-file /absolute/private/connection.json browser SESSION_UUID --reconcile`. It can report that no local dispatch occurred or that the owned
adapter/browser positively closed. This resolves resource cleanup, **not whether
a website transaction succeeded**. Records without positive evidence remain
unresolved. An orphan profile lock is not silently removed; an unavailable Voyage
owner still needs the normal evidence-based process recovery. No new browser,
new action ID or fresh run is used to conceal old uncertainty.

## Security and support limits

This is narrowly scoped local browser execution in Helm, not a general remote
shell or agent runtime. Raw JavaScript, CDP, arbitrary local paths, browser flags,
cookie export and provider credentials are not browser tool operations. External
web text/images are untrusted content, never policy instructions or approval.

Chromium sandboxing and the local origin/network gate complement application
policy; they do not make the cooperating same-user process boundary an OS-level
security boundary against arbitrary local code. The browser proxy checks actual
resolved destinations, blocks unapproved private-network access, and disables
unsupported WebSocket/service-worker/UDP paths rather than claiming a general
network sandbox. This can make some websites unavailable. Native browser UI,
extensions, camera/microphone, arbitrary desktop automation and personal-profile
attachment are not granted by sharing a dedicated browser.

Profile/cache budgets are application-enforced with bounded observations; in-flight
writes can overshoot. They are not hard filesystem quotas. Local frame rate,
connections, tabs, action/prompt deadlines, receipts and artifact storage are
bounded. Keep disk/resource-limit refusals visible and preserve their evidence.
See the [adapter contract](../helm/browser/README.md),
[runtime binding contract](shared-local-browser-runtime.md) and
[security deployment gates](security.md#first-release-audit-2026-09-08).

Source/offline checks do not certify live model quality, arbitrary sites, native
platforms or the actual remote TLS proxy. Those require their own recorded evidence.
