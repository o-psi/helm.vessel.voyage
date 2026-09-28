# Helm Web interaction inventory

This inventory records the authenticated Helm Web console at `/` as inspected on
2026-09-28. It maps the controls a person can reach from the voyage list,
connection manager, setup screens, conversation, and shared browser. Repeated
voyage cards use one interaction pattern, so they are described once. Machine
names, voyage titles, endpoints, and raw snapshots are omitted.

Menus and forms were opened without submitting changes. Controls marked
**conditional** appear only for the relevant run, account, update, browser, or
recovery state; their presence in the UI implementation was checked, but their
final actions were not exercised. This is an interaction inventory, not a
functional pass for every outcome. See the [Helm Web overview](../helm-web.md)
for the console's ownership and connection model.

For necessity, safe automation, and flow improvements for these controls, see
the [interaction decision audit](helm-web-ux-decision-audit.md).

## Navigation and account menu

- **New voyage** appears in the sidebar and the empty conversation view.
- **Search voyages** filters voyage cards by voyage or Vessel name and updates
  the displayed list. An empty result shows **No matching voyages**.
- On narrow screens, **Open voyage navigation** reveals the sidebar;
  **Close voyage navigation** or its backdrop dismisses it.
- Selecting a **voyage card** opens its conversation. Its adjacent **Voyage
  actions** control opens a separate action menu.
- **Vessel connections** shows the connection count and status list, with
  **Manage Vessels** as the deeper entry point.
- **Reconnect Vessels** retries the saved Vessel connections.
- The **Profile menu** offers **Light**, **Dark**, or **System** appearance,
  **Vessel connections**, and **Sign out**.

## Manage Vessels

- The overview lists saved connections and pending pairings. **View details**
  opens a connection's state, address, and Vessel ID. **Back**, **Done**, and
  **Close Vessel connections** leave the current screen.
- When no connection exists, **Add your first Vessel** opens the add screen.
  An offline connection's details can offer **Reconnect Vessels**.
- A pending pairing has **Check connection** to reconcile the original pairing
  attempt. One check during this audit returned an unconfirmed result; that
  observation is not a general pairing failure.
- **Remove connection...** opens **Keep connection** and **Remove [Vessel]**.
  The details screen explains that removal stops new access through this Web
  account without stopping voyages or revoking another client's grant.
- **Add Vessel** offers **New invitation** and **Existing credential**. Both
  methods have a **Name** field, an **Invitation JSON** or **Connection
  credential JSON** field, **Back**, and **Connect Vessel**.
- **How do I get an invitation?** opens host-side instructions, a **Full
  first-time setup guide** link, and **Back to adding**. Connection details
  end with **Back to Vessels**.
- A **Check status** recovery action is conditional on an uncertain connection
  change. It reads the saved state before another attempt.

## Voyage actions

Every voyage card has this menu. Each action opens a form that reads fresh
voyage state and includes **Check pending receipt** and **Close**. The final
button is **Confirm** unless a more specific label applies.

| Action | Interaction points |
| --- | --- |
| Rename | Edit **Name**, then **Confirm**. |
| Access mode | Select **Read only**, **Approval**, or **Full access**, then **Confirm**. |
| Archive / Restore | Use **Archive** or **Restore**, according to lifecycle state. |
| Branch | Enter **Name**; choose **Full conversation** or an earlier branch point; use **Load more branch points** when available; **Confirm**. |
| Cancel run | **Confirm** is available for an active run and disabled when no run is active. |
| Compact context | Set **Recent messages to retain**; type `COMPACT`; **Confirm**. |
| Clear conversation | Type `CLEAR`; **Confirm**. |
| Delete | Type `DELETE`; **Confirm**. |
| Details | Read the public voyage snapshot, including process, run, conversation, and receipt fields. |

## New voyage and profile setup

- **New voyage** opens **Location**, **Profile**, **Cancel**, and **Create
  voyage**. Creation prepares the voyage; the first message starts a run.
- **Location** offers a Vessel selector and workspace field for a new voyage.
  In an existing voyage, the Vessel and workspace are displayed but fixed.
  **Back**, **Done**, and **Close settings** navigate or dismiss the setup view.
- **Vessel updates** under Location offers **Latest stable release** or
  **Latest completed development build**, **Check and prepare update**, and
  **Check update status**. A prepared update can expose **Update this Vessel**
  and **Not now**; a verified completed update can expose **Continue setup**.
  The preparation and application steps were not run during this audit.
- **Profiles** has search and selection. **Create profile** opens a blank
  editor. Each saved profile's menu has **Edit**, **Duplicate**, **Make
  default** (disabled for the current default), and **Delete**. Delete opens a
  separate confirmation screen.
- The **profile editor** has **Profile name**, **Provider account**, **Model**,
  **Reasoning & service**, **Account usage**, **Cancel profile edit**, and
  **Save profile**.
- **Provider account** has account search and selection, **Add ChatGPT
  account**, and **Reload accounts**. An unavailable account can expose a
  refresh action.
- **Model** has search and selection. **Reasoning & service** has a reasoning
  slider and service-tier selector. **Account usage** has **Refresh usage**.
- **Add ChatGPT account** starts with an account name and **Continue with
  ChatGPT**. Conditional sign-in steps include a private code, **Open ChatGPT
  sign-in**, **I've signed in** or **Check sign-in**, **Cancel sign-in**, and
  closing the enrollment view.
- An existing voyage uses the same profile screens and ends with **Apply
  profile**. Uncertain profile or creation operations can expose **Check
  status** or **Check creation**.

## Conversation and composer

- The transcript scrolls through canonical history and loads older history
  near its top. **Jump to latest** returns to the newest content.
- **Show older actions** and **Hide older actions** expand or collapse older
  tool calls. Each tool action can expand to show its arguments and result.
  Tool previews and reasoning disclosures can also expand when present.
- Conditional long-content controls include **Read complete message** and
  **Load more output**. Links in messages remain independently clickable.
- The message field supports **Send** while idle and **Steer** during an active
  run. Enter submits; Shift+Enter inserts a line. **Cancel run** appears during
  an active run.
- **Attach pictures** opens a multiple-image picker. Pictures can also be
  pasted or dropped into the composer, previewed, and removed before sending.
- **Location** and **Profile** reopen voyage setup. **Voyage access mode**
  offers **Read only**, **Approval**, and **Full access** when an access change
  is available.
- Conditional pending decisions can offer **Approve** or **Deny**; question
  options, a **Custom answer**, **Send custom answer**, and **Skip question**;
  or scoped filesystem **Grant access for this run** and **Deny access**.
  A receipt notice can offer **Check receipts**.

## Shared browser

**Browser** opens the selected voyage's viewer. One inspected voyage reported
that its host could not start a browser and offered **Check browser status**.
Another exposed a live viewer with a blank tab. Browser availability therefore
depends on the selected voyage and its executing host.

- **Expand browser** changes to **Show chat** when expanded. **Close browser
  viewer** detaches this view while leaving the voyage browser running.
- **Browse privately** changes to **Finish private browsing** while private
  control is active. **View at actual size** changes to **Fit page in view**.
- **More browser options** contains **Use browser**, **Disconnect viewer**,
  and **Close browser** when their control states permit them. **Continue
  agent** appears after a person takes browser control. Completed downloads
  appear in this menu when available.
- **Browser tabs** support selecting a tab, closing a tab when allowed, and
  creating a **New tab**.
- **Browser navigation** has **Back**, **Forward**, **Reload** or **Stop
  loading**, **Website address**, and **Go to address**. The welcome view has
  **Enter a website address** to focus the address field.
- With human control, the mirrored website supports page clicks, right clicks,
  text entry, selection, keyboard input, scrolling, and a website file input.
  A separate
  **Text for browser** field can show **Send text to browser**.
- A website dialog can expose **Website dialog response**, **Accept dialog**,
  and **Dismiss dialog**. Recovery states can expose **Check browser status**
  or **Start browser**.

## Audit boundary

This audit opened menus and read forms, including the browser viewer. It did
not create or modify voyages, change access, send a message, enroll an account,
install an update, remove a connection, delete history, or act on a website.
Controls that depend on those states are listed as conditional interaction
points, not verified end-to-end behavior.
