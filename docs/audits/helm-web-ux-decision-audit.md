# Helm Web interaction decision audit

- **Inspected:** 2026-09-28, authenticated console at `/`.
- **Baseline:** [interaction inventory](helm-web-interaction-inventory.md).
- **Scope:** Helm Web controls and their enclosing flows; design recommendations, not implemented behavior.

The live review opened the voyage menu and Details, new-voyage Location and
Profiles, and the profile action menu. The prior inventory inspected the other
menus and the shared browser. No voyage, connection, profile, update, message,
website, or permission was changed. Conditional controls below are assessed from
the inventory and UI contract; they have not all been exercised end to end.
Private names, addresses, account identifiers, and transcript content are
omitted.

## Decision rules

**Need** means keep (needed for a user goal), contextual (needed only in a
relevant state), merge (same goal is exposed in multiple places), or move
(needed, but misplaced in the current flow). **Automation** means:

- **Auto-read:** bounded, read-only observation or reconciliation. Preserve the
  exact operation identity; never resubmit an uncertain effect.
- **Auto-local:** reversible presentation or local preparation without changing
  Voyage, Vessel, account, or website state.
- **Assist:** suggest, prefill, validate, or preview; the person submits.
- **Human:** require a deliberate user action. **Never auto** emphasizes an
  authority, privacy, irreversible, or external-effect boundary.

Automating an interface gesture is not a reason to erase a product boundary.
Helm remains a viewer/controller, the executing Voyage owns its run and browser,
and Vessel grants and Voyage policy still govern effects. In particular, do not
auto-send messages, expand access, approve tools or filesystem grants, enroll
accounts, install updates, replay uncertain commands, act on websites, or delete
history. See [Helm Web](../helm-web.md), [host browser](../host-browser.md), and
[remote updates](../remote-updates.md).

## What the live review exposed

1. The sidebar had **85 voyages**. Cards consumed most of the viewport, titles
   were truncated, and repeated **Suspended** labels provided little sorting
   value. One inspected card said Suspended while its latest run had completed;
   the process state and latest-run outcome need separate labels.
2. With no voyage selected, **New voyage** appeared twice while the composer
   remained visible but disabled. A single empty-state action would make the
   next step clearer.
3. **Details** displayed a very large raw JSON snapshot, including history and
   execution metadata. The normal path needs a concise status summary with
   technical data behind an explicit disclosure.
4. In new-voyage **Location**, **Vessel updates** surfaced a saved completed
   update that no longer matched the reviewed release. This maintenance state
   interrupted a creation flow and gave no immediate creation-related action.
5. **Profile** was briefly disabled while the catalogue loaded. Once loaded,
   three profiles were shown with a search field, action menus, and both header
   and footer Back controls. Loading, selection, and editing can be clearer
   without removing keyboard access.

These are observed UI facts, not measured task times or evidence that a proposed
change would improve completion rates. Recommendations require usability and
failure-path checks before implementation.

## Navigation and account

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| **New voyage** in sidebar and empty view | Merge | Human | Keep one prominent empty-state action; retain the sidebar action when a voyage is selected. Hide the disabled composer until there is a voyage or a genuine draft flow. |
| **Search voyages** / **No matching voyages** | Keep | Auto-local | With large lists, add filters for Vessel and lifecycle/status, show result count and a clear-query action, and preserve the query while navigating. Search should not imply it covers unloaded history. |
| **Open/Close voyage navigation** and mobile backdrop | Contextual | Auto-local | Keep on narrow screens; close after selecting a voyage and return focus to the conversation. Maintain an explicit reopen control. |
| **Voyage card** selection | Keep | Human | Make title, Vessel, last activity, process state, and latest-run outcome distinguishable. Use a readable title wrap or full-title tooltip rather than relying on truncation. Keep route identity stable. |
| **Voyage actions** menu | Keep | Human | Separate frequent actions, advanced lifecycle actions, and danger actions. Hide impossible actions rather than making every card expose nine choices. |
| **Vessel connections** count/popover | Keep | Auto-read | Show connected/total plus a concise reason for offline or pending states. Make **Manage Vessels** the clear entry point; avoid repeating status lists in several popovers. |
| **Reconnect Vessels** | Contextual | Auto-read | Reconnect saved grants with bounded backoff after transient drops. Show a manual retry only while disconnected or recovery is stalled; never repeat an uncertain command. |
| **Profile menu** | Keep | Human | Rename this to **Account and appearance** if it is not the Voyage profile picker. The current shared word “Profile” names two different concepts. |
| **Light / Dark / System** | Keep | Auto-local | Follow System by default, remember an explicit override, and preview the active choice. Do not silently replace a chosen mode. |
| **Sign out** | Keep | Human | Keep a persistent, plainly labeled action. If local unsent drafts exist, disclose that reloading/sign-out loses them before leaving. |

## Vessel connection flow

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| Saved connection list and **View details** | Keep | Auto-read | Put state, last successful contact, and a short failure reason on each card. Details should lead with connection and access consequences, then reveal address and ID for troubleshooting. |
| **Back**, **Done**, **Close Vessel connections**, **Back to Vessels** | Merge | Auto-local | Use one predictable header Back within the flow and one Close at its boundary; keep footer controls only when they advance or commit work. |
| **Add your first Vessel** empty state | Contextual | Human | Keep as the primary action only when empty, with one sentence about the prerequisite: a reachable, authenticated Vessel endpoint. |
| Offline-detail **Reconnect Vessels** | Contextual | Auto-read | Show connection-specific recovery and a bounded retry indicator rather than a global reconnect label in a single Vessel's details. |
| Pending-pairing **Check connection** | Contextual | Auto-read | Poll the exact pending pairing while the screen is open, with a deadline and status. Keep manual Check as recovery; never create a second pairing because a response was lost. |
| **Add Vessel** and **New invitation / Existing credential** | Keep; credential is advanced | Human | Lead with invitation. Explain that an existing credential is an import/replacement path and keep it under an advanced disclosure. Do not automatically pair or grant access. |
| **Name**, **Invitation JSON / Connection credential JSON** | Keep | Assist | Validate JSON shape and expiry locally, identify which field is wrong without echoing private data, and provide a safe host-side copy path. Keep private values masked from logs and voyage history. |
| **Connect Vessel** and add-form **Back** | Keep | Human | Show the exact target and resulting Web-account access before submission. Disable duplicate submission, then reconcile the exact receipt after timeout. Preserve the draft on failure. |
| **How do I get an invitation?**, **Full first-time setup guide**, **Back to adding** | Keep | Auto-local | Make a short in-product guide first and the full guide secondary. Preserve entered name/JSON when returning. State clearly that the host creates the invitation. |
| **Remove connection...**, **Keep connection**, **Remove [Vessel]** | Keep | Never auto | Label the action **Remove from Helm Web**. In confirmation, distinguish loss of this account's new access from revoking the underlying Vessel grant or stopping voyages. Keep the cancel path obvious. |
| Uncertain-change **Check status** | Contextual | Auto-read | Reconcile the saved connection state automatically with bounded attempts, then offer a visible manual refresh. Do not resubmit add/remove automatically. |

## Voyage action menu and lifecycle

Each action currently opens a fresh-state form. Fresh reads are valuable; a
generic **Confirm** and ever-present **Check pending receipt** make all actions
look equally risky and can obscure what will happen.

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| Open action form / fresh-state load | Keep | Auto-read | Fetch the current revision when opening, show a loading state, and explain changed state before enabling submission. Do not leave a generic disabled Confirm without a reason. |
| **Check pending receipt** | Contextual | Auto-read | Observe exact receipts in the background after submission. Surface this button only for a pending/uncertain result or stalled observation; never retry the effect. |
| **Close** and generic **Confirm** | Merge | Auto-local | Use action-specific verbs such as **Save name** or **Create branch**. Close is always available unless dismissing would hide an unresolved result; then explain how to find it later. |
| **Rename** / **Name** | Keep | Assist | Allow inline rename or a small dialog. Suggest a title from the first request only as a suggestion; preserve explicit user titles. |
| **Access mode** / Read only, Approval, Full access | Keep | Never auto | Show current effective mode, executing Vessel, and what the next change permits. Never auto-upgrade. Use one shared access editor with the composer selector instead of divergent flows. |
| **Archive / Restore** | Keep | Human | Show exactly **Archive** or **Restore** for the current state. Move archived voyages out of the default list but make them discoverable through a filter. Never archive solely because of age. |
| **Branch**, name, **Full conversation**, earlier point, **Load more branch points** | Keep | Assist | Preview the chosen message boundary, new identity, and the fact that workspace files are not copied or rolled back. Load earlier points on demand without resetting selection. Creation remains explicit. |
| **Cancel run** | Contextual | Human | Put **Stop run** beside the active-run status, with immediate feedback and eventual cleanup outcome. Hide the menu item while idle; avoid an extra generic confirmation for the same clear intent. |
| **Compact context**, **Recent messages to retain**, typed `COMPACT` | Contextual | Assist | Offer when context pressure is visible. Explain the actual keep-N behavior and which context is omitted; do not imply generated summarization. Use a reviewed action label rather than requiring a magic word for routine compaction. |
| **Clear conversation**, typed `CLEAR` | Keep | Never auto | Explain whether the voyage, workspace, and process remain. Keep strong confirmation and distinguish it visibly from Delete. |
| **Delete**, typed `DELETE` | Keep | Never auto | Keep a separate danger section and explicit identity review. State what is deleted, what cannot be recovered, and whether owned resources have observed cleanup. |
| **Details** | Keep as advanced | Auto-read | Replace raw JSON as the default with a concise lifecycle/run/receipt summary. Provide sanitized technical JSON behind a disclosure or download for diagnostics. |

## New voyage, profiles, accounts, and updates

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| New-voyage **Location**, **Profile**, **Cancel**, **Create voyage** | Keep | Assist | Present one review screen with the selected Vessel, workspace, account/model, and access defaults. Prefill only safe remembered choices; creation stays explicit and separate from sending the first message. |
| **Vessel** selector | Keep | Assist | Prefer the last usable Vessel and clearly flag offline choices. Never silently move an existing voyage to another Vessel. |
| **Workspace** field or selector | Keep | Assist | Offer known allowed workspaces; validate an owner-entered absolute path and explain errors without exposing private paths publicly. Do not create a directory as a side effect of selection. |
| Setup **Back**, **Done**, **Close settings** | Merge | Auto-local | Use Back for nested screens and one explicit close/cancel at the boundary. Keep unsaved editor values when moving between pickers. |
| **Vessel updates** within Location | Move | Auto-read | Put maintenance under Vessel details or a dedicated Updates entry. During voyage creation show only a blocking compatibility issue with a direct path to resolve it. |
| **Update source**: **Latest stable release / Latest completed development build** | Keep | Human | Default to stable; describe development risk and display installed, available, and reviewed versions. Never silently switch channels. |
| **Check and prepare update**, **Check update status** | Keep; status contextual | Auto-read | Check availability when opening Updates, but retain explicit preparation if it downloads/reviews an exact release. Reconcile a saved operation automatically; show why a completed review no longer matches the current installation. |
| **Update this Vessel**, **Not now**, **Continue setup** | Keep | Never auto for apply | Show exact source/version and expected service interruption at approval. After verified reconnect, offer Continue setup; do not auto-apply or treat a successful request as a verified installation. |
| **Search profiles** and select a profile | Keep; search contextual | Auto-local | With a few profiles, selection should be immediate; make search prominent only as lists grow. Show account availability before selection and preserve the chosen profile through setup. |
| **Create profile** | Keep | Human | Use a visible text label as well as the `+` affordance, especially in an empty list. Do not force a new profile when a suitable default exists. |
| Profile **Edit** and **Duplicate** | Keep | Human | Make Duplicate open a named copy for review; do not silently save a copy. Keep editing separate from choosing a profile for this voyage. |
| **Make default / Default profile** | Keep | Human | Show the effect on future voyage choices, not existing voyages; disable it for the current default with an explicit reason. |
| Profile **Delete** and confirmation | Keep | Never auto | State that existing voyages retain their settings and identify the exact profile. Keep a clear cancel action. |
| Editor **Profile name**, **Save profile**, **Cancel profile edit** | Keep | Assist | Validate uniqueness/readability before save; show unsaved changes on exit and retain the draft across account/model pickers. |
| **Provider account** search/select, **Reload accounts**, unavailable **Refresh sign-in** | Keep; reload contextual | Auto-read | Refresh the catalogue after successful enrollment or sign-in. Show account readiness and reason; use manual reload only for stale/error states. An expired binding refresh must stay scoped to that binding. |
| **Add ChatGPT account** and account name | Keep | Human | Explain that sign-in happens for the executing Vessel's account, then ask for a recognizable local label. Do not request credentials in the voyage composer. |
| **Continue with ChatGPT**, private code, **Open ChatGPT sign-in** | Keep | Never auto for sign-in | Make code handling private and time bounded; provide copy/open affordances and clear destination. Never put codes in transcript, logs, or a public receipt. |
| **I've signed in / Check sign-in**, **Cancel sign-in**, close enrollment | Keep; manual check contextual | Auto-read for status | Poll the exact enrollment with a deadline, then offer Check if observation stalls. Cancellation must report requested versus observed outcome; never restart an uncertain enrollment automatically. |
| **Model** search/select | Keep | Auto-local | Show model compatibility for the chosen account and mark the default. Explain when changing model resets reasoning or service selections. |
| **Reasoning** slider and **Service tier** selector | Keep | Assist | Use labeled discrete choices and an explicit provider-default option. Show whether each setting applies to the saved profile or the next run; do not imply cost or latency guarantees without provider evidence. |
| **Account usage** and **Refresh usage** | Keep as read-only | Auto-read | Fetch when the usage screen opens, timestamp the result, and reserve Refresh for stale/error cases. Do not poll usage continuously from the main console. |
| Existing-voyage **Apply profile** | Keep | Human | Compare current and proposed model/account/settings, state when the change takes effect, and retain the running voyage's independent execution. |
| Uncertain **Check status / Check creation** | Contextual | Auto-read | Reconcile the exact saved operation. If creation succeeded but navigation failed, open that identity; never create a second voyage automatically. |

## Conversation, composer, and decisions

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| Scroll transcript / load older history | Keep | Auto-read | Load near the top without moving the reading anchor; show that history is loading and whether the beginning has been reached. |
| **Jump to latest** | Contextual | Auto-local | Show only when the reader is away from the end; do not force-scroll while they are reading older content. |
| **Show older actions / Hide older actions**, tool details, previews, reasoning disclosures | Keep as advanced | Auto-local | Default to concise result cards; reveal arguments, output, and provenance progressively. Distinguish provisional tool activity from saved outcomes and keep private data sanitized. |
| **Read complete message**, **Load more output** | Contextual | Auto-read | State how much remains and preserve scroll position. Avoid a long raw-data wall in the main transcript. |
| Links in messages | Keep | Human | Show a recognizable destination and an external-site cue; do not open links or transmit conversation content automatically. |
| Message field, **Send**, Enter, Shift+Enter | Keep | Human | Label the exact effect before submission, preserve draft on failed/uncertain admission, and show a clear multiline hint. Sending remains explicit even when Enter is a shortcut. |
| **Steer** during an active run | Keep | Human | Call it **Send to current run** with run state and delivery feedback. If the run ends before admission, retain the draft; do not retarget or replay it silently. |
| Composer **Cancel run** | Contextual | Human | Keep beside active status with immediate “stop requested” and later “stopped/cleanup observed” states. Make it the primary stop route, not a hidden action-menu form. |
| **Attach pictures**, paste/drop, previews, remove | Keep | Auto-local preparation | Show count/size/type and conversion outcome before send, with remove/cancel. Never upload just because a file was selected, pasted, or dropped. |
| Composer **Location / Profile** | Merge | Human | Use a single **Voyage settings** entry for an existing voyage and make fixed versus editable values obvious. Do not present a workspace picker if relocation is unsupported. |
| Composer **Voyage access mode** | Merge | Never auto | Use the same reviewed access flow as the voyage action menu; show current effective policy and host authority. Do not auto-elevate to make a requested tool work. |
| Pending **Approve / Deny** | Keep | Never auto | Present the exact tool, host, scope, and expiry with a clear decision outcome. Timeout, denial, and cancellation must have different labels. |
| Question options, **Custom answer**, **Send custom answer**, **Skip question** | Keep | Human | Keep choices short, allow editing before submit, and explain that the answer enters conversation history. Skip should not be styled as a denial. |
| **Grant access for this run / Deny access** | Keep | Never auto | State path, read/write scope, host, and run lifetime; keep the grant limited to that run. No inference from a message should authorize broader filesystem access. |
| **Check receipts** | Contextual | Auto-read | Reconcile exact pending commands automatically with bounded reads. Show manual Check if observation stalls and expose the last known state without suggesting the command should be retried. |

## Shared browser and website control

| Interaction point | Need | Automation | UX decision |
| --- | --- | --- | --- |
| **Browser** viewer open | Keep | Human | Show browser activity and current control owner before opening. Opening a viewer is observation; it must not claim website control. |
| **Expand browser / Show chat** | Keep | Auto-local | Remember panel size locally and keep a visible return to chat. On narrow screens, restore focus when the panel closes. |
| **Close browser viewer / Close viewer** | Keep | Human | Label this **Close panel** and state that the task browser keeps running. Return focus to the Browser toggle. |
| **Disconnect viewer** | Contextual | Human | Explain why someone would disconnect the stream while keeping the panel open; otherwise merge with Close panel. Do not imply this closes the task browser. |
| **Use browser** | Keep | Human | Show a persistent **Agent controls / You control / Viewing** indicator and a clear handoff. A first website click may claim control only when that is visibly explained. |
| **Browse privately / Finish private browsing** | Keep | Never auto | Rename to **Private control** with an explanation that agent-visible page/input recording is fenced while active; avoid implying anonymous/incognito browsing. Never exit private control automatically on disconnect. |
| **Continue agent** | Keep | Human | Show what control returns to the agent and whether private input has ended. Require an explicit handback; no automatic resume after inactivity. |
| **View at actual size / Fit page in view** | Keep | Auto-local | Fit by default when space is tight, show the current zoom mode, and preserve a one-click actual-size option. |
| **More browser options** | Keep | Auto-local | Put rare lifecycle controls here, but keep current controller/private state visible outside the menu. |
| **Close browser** | Keep | Never auto | Rename to **Close task browser** and confirm the effect on tabs/downloads and the active run. Closing the panel is a separate action. |
| Completed downloads | Contextual | Human | Show a visible download count/notification and an explicit save action; do not silently write files to the person's computer. |
| **Browser tabs**: select / close / **New tab** | Keep | Human | Display the active tab, title, and loading state. Warn before closing a tab with an unresolved website dialog or private input. |
| **Browser navigation**: **Back / Forward / Reload / Stop loading** | Keep | Human | Match familiar browser behavior and disable unavailable history actions. A reload can repeat website behavior, so never trigger it as generic recovery. |
| **Website address**, **Go to address**, welcome **Enter a website address** | Keep | Human | Distinguish URL navigation from search if search is supported; validate input locally and make the destination visible before navigating. |
| Mirrored page click, right click, select, keyboard, scroll | Keep | Human | Show control ownership and a clear fidelity warning only when a page surface falls back to a visual mirror. Never let Helm run website scripts or infer consent from observation. |
| Website file input | Contextual | Human | Show the selected file, target site, and size limit before upload. A file chooser selection alone must not transmit the file. |
| **Text for browser / Send text to browser** | Contextual | Human | Use when mirrored input cannot reliably take text. Label the destination field/site and keep the text out of conversation history. |
| Website dialog response / **Accept dialog / Dismiss dialog** | Contextual | Human | Surface the dialog's site and message, preserve prompt text privately, and block conflicting website input until resolved. |
| **Check browser status / Start browser** | Contextual | Auto-read for status; Human for start | Poll bounded status after disconnect/failure, show whether the host is unavailable or capacity-limited, and keep Start explicit. Do not restart a browser while its prior effects or cleanup are uncertain. |

## Flow priorities

1. **P0 — status and recovery clarity.** Separate process suspension from last
   run outcome; replace raw Details JSON with a readable summary; make receipt
   checks contextual and automatically read-only; distinguish stop requested
   from cleanup observed. This addresses the most visible ambiguity without
   changing execution ownership.
2. **P0 — authority and privacy language.** Unify access review, clarify
   connection removal versus grant revocation, and rename browser private
   control and close actions. Keep approval, updates, sign-in, deletion, and
   browser handoff under explicit human control.
3. **P1 — voyage finding and creation.** Group/filter the large voyage list,
   separate archived items, reduce duplicate empty-state controls, and make
   loading/defaults clear. Move Vessel maintenance out of Location.
4. **P1 — everyday run loop.** Expose Stop near run status, explain active-run
   steering, retain failed/uncertain drafts, and progressively disclose tool
   output. Coordinate with the open [drafts issue](https://github.com/o-psi/helm.vessel.voyage/issues/309).
5. **P2 — deep setup and browser polish.** Simplify profile navigation,
   refresh read-only usage/status on entry, improve download discoverability,
   and explain visual fallback. Coordinate browser changes with
   [#333](https://github.com/o-psi/helm.vessel.voyage/issues/333).

## Validate before changing the UI

- Run short observed tasks with new and experienced users: find an old voyage,
  create one, recover an offline Vessel, change a profile, stop a run, inspect a
  failed action, and hand a browser to/from the agent. Record misclicks,
  comprehension, time, and whether the intended identity was acted on.
- Exercise offline, stale revision, lost response, expired sign-in, update
  mismatch, browser-capacity, and interrupted-run states with disposable
  fixtures. Verify that automatic reads stay bounded and no uncertain effect is
  replayed.
- Check keyboard, screen-reader, narrow-layout, focus return, and long-content
  behavior for the proposed navigation and dialogs. Do not infer native TUI or
  other-platform quality from this Web audit.
- Turn accepted recommendations into scoped implementation issues with exact
  acceptance criteria. This audit does not close the earlier
  [CLI UX audit](https://github.com/o-psi/helm.vessel.voyage/issues/271) or its
  [implementation follow-up](https://github.com/o-psi/helm.vessel.voyage/issues/272).
