# Right-side Actions and requests

Voyage Actions and runtime question/permission requests use the same rounded
border, horizontal padding, selected-row treatment, hover underline, muted disabled
controls and wrapped button layout. They stay beside the conversation rather than
replacing it. Helm requires at least 40 columns by 18 rows; the right panel uses
half the available width, capped at 64 columns.

## Mouse operation

- Open Actions with the voyage's **⋮** button. When voyage navigation is hidden,
  the conversation heading provides an **Actions** button instead.
- Click an action or access mode to open it. Use the visible **Open / Review /
  Confirm**, **Back / Close**, and **Up / Down** buttons; the mouse wheel scrolls
  the panel under the pointer. Wrapped descriptions and disabled reasons remain
  readable by scrolling, including in narrow panels.
- In a question or permission request, click a choice, then **Confirm**. Selecting
  **Allow once** never sends approval by itself. **Prev / Next** navigate multiple
  requests; leave the custom editor with **Back** first.
- Custom answers, names, branch labels and deletion confirmation have **Paste**
  and **Clear** controls. Paste reads text from the local Helm machine's clipboard,
  not the Vessel's machine. Type normally or use terminal text paste as alternatives.
  Clear replaces the field with empty text; it never submits it.
- Clipboard acquisition is bounded and uses the existing local clipboard policy
  and helper cleanup. Other key input, a mouse click or a resize cancels a pending
  panel paste. Results are rejected if the destination or its text has changed.
  Images/files are not inserted into these fields. An unavailable clipboard utility
  or desktop clipboard is reported without changing the field.

## Keyboard behavior and safety

Up/Down or Tab/Shift+Tab selects a menu entry or response choice. Enter opens or
confirms; PageUp/PageDown reads longer content. Existing editor cursor keys and
text limits remain: 256 bytes for action fields, 4096 bytes for custom answers.

Esc retains its original meaning:

- **Permission request:** deny.
- **Question:** skip.
- **Custom-answer editor:** back to choices, keeping the answer draft.
- **Actions subpage:** back; at the top level, close.

Mouse buttons use those same operations, not a separate submission path. Pending
or expired decisions cannot be answered. Action availability and the last rendered
target/incarnation are checked; deletion still requires the exact `DELETE` text
and reviewed conversation revision. Access review can scroll instead of demanding
a larger terminal, but **Confirm** stays disabled until its final lines are visible.
Owner-side access and command checks remain authoritative.

## Verification scope

Targeted Helm compilation and a temporary Linux executable probe check the shared
panel/button geometry and styling. Source review covers event routing, displayed
identities, pending/expiry guards, clipboard cancellation and existing response
paths. This is not a recreated automated interaction suite, live-provider evidence,
or native macOS/Windows clipboard verification.
