# Canonical design lessons from the inspiration apps

Status: consolidated design reference, based on the 13 source snapshots downloaded
on 2026-09-07. These are recommended principles for future design decisions, not
a claim that Helm implements them. [Architecture](architecture.md) governs component
ownership; [current state](current-state.md) describes implemented behavior;
[UX readiness](ux-readiness.md) governs interaction acceptance.

“Canonical” here means one consolidated vocabulary and decision framework for this
collection. It does not mean the upstream authors endorse this synthesis or that
every app follows every rule.

## Central lesson

A useful terminal interface keeps four things clear: what the user is looking at,
what they can do next, what will be affected, and what actually happened.
Responsiveness, previews, layout, search, help and visual polish are valuable when
they make those answers easier to see.

The collection offers three complementary strengths:

- **Orientation and action:** Yazi, Joshuto, xplr, GitUI, Rainfrog, Television and
  Atuin connect an identifiable object to a small set of relevant actions.
- **Observation and interpretation:** bottom, Trippy and scope-tui make changing
  data inspectable through purpose-built views and controls.
- **Communication and consequence:** Oatmeal gives conversation a readable shape,
  Gitlogue makes change visible over time, and Caligula makes a consequential
  operation understandable from selection through verification.

These categories describe the lessons being extracted, not exclusive app types.

## Evidence and limits

The review inspected upstream READMEs, selected usage/configuration documents and
representative UI, input or state code. The source register below pins references
to each downloaded commit so the document remains useful without the local clones.
No apps were built or run for this review; no comparative performance measurements,
accessibility tests or user studies were conducted. Statements about mechanisms are
source observations; their benefits and the design rules are reasoned synthesis.
The original list's superlatives are not adopted as measured findings.

Coverage is all 13 downloaded repositories, including Joshuto and xplr separately.
Chess-tui, Minesweep, 2048, wordl, maze visualizers, ratthew and Plastic were mentioned
in the original request but were not in the downloaded collection and are not
represented as reviewed here.

## What each app contributes

| App | Observed mechanism | Extracted lesson | Transfer limit |
| --- | --- | --- | --- |
| **Yazi** | Async I/O and managed tasks; configurable parent/current/preview proportions; preview limits and terminal image fallbacks. [Yazi overview][yazi-readme], [defaults][yazi-detail] | Keep navigation responsive while nearby context and previews help identify the target. Bound secondary work. | A rich preview may depend on terminal support and external helpers. Do not make an image the only way to identify an object. |
| **bottom** | Focused widgets, expansion, freeze, time-series graphs and process tables; mouse and keyboard widget selection. [Overview][bottom-readme], [usage][bottom-detail] | Support overview, focused inspection and a stable view of changing data as explicit user choices. | A dashboard is appropriate for simultaneous measurements; it can crowd out a primary reading or writing task. |
| **GitUI** | Contextual command help, async Git operations, file/hunk/line actions; help consumes input while open. [Overview][gitui-readme], [help implementation][gitui-detail] | Bring operations close to the selected object and show the commands that apply in the active context. | Its keyboard-only positioning is not evidence that mouse affordances are unnecessary for Helm. |
| **Rainfrog** | Schema/table browsing, editor, query history/favorites and results; explicit focus variants for these surfaces and popups. [Overview][rainfrog-readme], [focus model][rainfrog-detail] | Keep the object, draft operation and result distinguishable while making movement between them direct. | SQL execution is consequential. A convenient editor or query history does not establish that a query is safe to execute. |
| **Television** | Named channels connect a source, preview and actions; help uses merged configuration and groups actions into ordered sections. [Overview][television-readme], [help renderer][television-detail] | Reuse one search interaction across domains, but keep the domain and available actions visible. Generate help from actual configuration. | A channel can invoke external commands. A familiar picker must not hide the authority or effects of its actions. |
| **Atuin** | History includes directory, session, time, duration and exit status; scoped search; Tab returns a selection without execution while Enter follows the acceptance setting. [Overview][atuin-readme], [keybindings][atuin-detail] | Retrieval becomes more useful when context disambiguates matches and recalling an item is distinct from executing it. | History may be sensitive; collecting or exposing more context requires a product-specific privacy decision. |
| **Oatmeal** | Chat bubbles, slash commands and editor integration; the bubble renderer treats code blocks separately and assigns them numbers. [Overview][oatmeal-readme], [bubble renderer][oatmeal-detail] | Make conversational roles and reusable outputs easy to distinguish, with a direct path from reading to using an answer. | Bubbles consume width. Decorative rendering must not alter copied code or become a substitute for faithful text. |
| **Joshuto** | Parent/current/preview columns, collapsible preview, size limits and optional icons; a searchable, sortable keybinding help table. [Defaults][joshuto-readme], [help widget][joshuto-detail] | Familiar spatial navigation and searchable help can work together: a user can learn commands without leaving the task. | Ranger/Vim familiarity is learned expertise, not universal intuition. Essential navigation needs visible guidance. |
| **xplr** | Configurable modal keybindings and layouts, including help and selection panels; integration with command-line utilities. [Overview][xplr-readme], [modes][xplr-detail], [layouts][xplr-layout] | Keep interaction concepts composable and make the active mode legible wherever it changes key meaning. | Extensive customization increases learning and configuration burden. A coherent default remains a product responsibility. |
| **Trippy** | Hop-oriented diagnostics with alternate chart/map views; the body explicitly chooses error, no-data or data views. [Overview][trippy-readme], [body renderer][trippy-detail] | Use domain-specific representations and give absence of data a different presentation from a failed observation. | A network topology or map is meaningful in network diagnosis; the same graphic may add little to a conversation. |
| **scope-tui** | Waveform/spectrum/vector modes; pause, scale changes and reset; documented resolution and interpolation limitations. [Usage and precision][scope-tui-readme], [input/render loop][scope-tui-detail] | Make a visualization an inspectable instrument: users need control of time, scale and interpretation. | Attractive curves can imply more precision than the data or terminal supports. Display controls are not execution controls. |
| **Gitlogue** | Commit replay with pause, line/change stepping and previous/next commit controls; manual stepping enters a paused state. [Overview][gitlogue-readme], [playback implementation][gitlogue-detail] | Animation can explain sequence when the viewer controls its pace and can inspect individual changes. | Simulated typing is a storytelling device. It would misrepresent live agent progress if presented as actual execution. |
| **Caligula** | Disk identity and confirmation, input validation, writing and post-write verification; exit handling differs while unfinished. [Overview][caligula-readme], [UI state][caligula-detail] | Design the complete consequential workflow, including target review, progress, verification and interruption. | Caligula's exit semantics belong to disk imaging. Helm detach must continue to leave voyages running. |

## Canonical principles

### 1. Give the screen a primary object and a primary task

Yazi and Joshuto organize navigation around the current directory and entry.
Rainfrog separates database objects, query input and results. Oatmeal centers the
conversation. The general rule is to allocate space according to the user's
current task, then place supporting context nearby.

For Helm, reading and composing a conversation should retain the largest useful
region during ordinary chat. A voyage list and action details support that task.
A monitoring view can legitimately use a different allocation.

**Review question:** Can the user identify the current object, their input
destination and the next action without interpreting internal identifiers?

### 2. Treat responsiveness as an interaction contract

Yazi and GitUI explicitly pursue asynchronous work; Television presents an
interactive search loop. The transferable principle is that expensive data work
should not prevent navigation, cancellation requests or visible input feedback.

Separate immediate acknowledgment from an eventual result. Keep partial, loading,
failed and stale states distinguishable. Bound preview work, discard obsolete
responses and prevent a delayed response from appearing under a newly selected
target. Those last requirements are synthesis for an asynchronous interface, not
claims verified across every app.

For Helm, a slow Vessel or long tool result should not stop draft editing or voyage
navigation. An acknowledged click must not be styled as completed work.

**Review question:** While a dependency is slow, does the interface remain usable
and keep the identity and freshness of the displayed result clear?

### 3. Let users inspect before they commit

Yazi's previews and Television's source/preview/action model help users evaluate
a candidate before opening or acting on it. Atuin makes returning a command for
editing distinct from accepting it for execution. Caligula adds target review
where a mistake has serious consequences.

Use selection to reveal information. Require a distinct, clearly labeled action
for effects. A preview must identify the same target that the eventual action
uses, including scope that is easy to miss in a truncated label.

For Helm, an approval should expose the target voyage, executing host, intended
operation and relevant arguments. Selecting a historical message should not
silently submit it as a new turn.

**Review question:** Can the user inspect and back out without causing the effect?

### 4. Teach commands where they become useful

GitUI exposes contextual commands; Television groups help generated from active
configuration; Joshuto lets users search its keybinding table. Together these
suggest a progression: visible primary actions, short contextual hints, searchable
full help, then configurable expert shortcuts.

Help should use the active bindings and capabilities. Where a relevant action is
unavailable, explain the requirement instead of offering a control that silently
does nothing. Do not require opening a help screen to discover every primary task.

For Helm, visible Actions and command discovery should agree on names, targets
and availability. Local key hints cannot advertise runtime tools that the owner
does not expose.

**Review question:** Can a new user discover the action, and can a practiced user
invoke the same action quickly?

### 5. Make focus, selection and modes explicit

bottom distinguishes widget focus from movement within a widget. Rainfrog models
focus for its editor, history, results and popups. xplr makes modes a first-class
configuration concept. Atuin illustrates that Escape can mean different things
in different input modes.

Use a small consistent interaction grammar within the product. A modal surface
must own its input; closing it should return to a predictable location. Hover,
keyboard focus, selected item and a destructive action target are distinct states
even if some share styling.

For Helm, typing into a question answer, composer or private terminal must have
an unmistakable destination. Changing voyage cannot silently retarget a pending
confirmation.

**Review question:** Can the user predict what the next key does and which object
it affects, including after a popup, resize or background update?

### 6. Use search to reduce navigation, with visible scope

Television applies a common search pattern to named data sources. Atuin makes
context a useful discriminator among similar history entries. Joshuto applies
search to help itself.

Show what is being searched, what is matched and enough context to distinguish
similar entries. Preserve a route back to the prior view and draft. Explain
whether a search covers loaded items, all retained history or a filtered subset.
“No matches,” “still loading” and “could not search” need different messages.

For Helm, a future voyage picker could show name, host and recent activity together.
Conversation search must accurately describe the portion of history it covers.

**Review question:** Does the user know the search domain and why a result is the
right target, without opening several lookalikes?

### 7. Let density change with attention

bottom offers widget expansion; Joshuto can collapse previews; scope-tui can hide
its UI. The common lesson is to support broad orientation and detailed inspection
without trying to show every surface at maximum detail simultaneously.

Give summaries an obvious drill-down path and make expansion reversible. On
narrow terminals, prioritize the task, its identity and required controls; expose
secondary surfaces through reachable navigation. A hidden panel must not make its
essential actions unavailable.

For Helm, a long code block or approval detail may need more space than the default
layout provides. Expansion should preserve the user's place and draft.

**Review question:** Can the full task still be completed when the normal
multi-pane arrangement no longer fits?

### 8. Make live information inspectable

bottom supports freezing and time-range adjustment. scope-tui supports pausing
and scale changes. Gitlogue supports explicit stepping through history. Each
allows the viewer to stop chasing moving content.

Keep “follow live” distinct from “inspect earlier information.” Preserve a reading
anchor while new material arrives, visibly indicate that new content exists, and
offer a direct return to the live edge. Label units, time windows and aggregation
where a chart would otherwise be ambiguous.

For Helm, scrolling back should hold the reading position while a voyage keeps
working. Pausing the display must never imply that a run has been paused.

**Review question:** Can the user examine one detail without losing it to updates
or confusing observation controls with execution controls?

### 9. Use visual hierarchy to encode meaning

Oatmeal distinguishes conversational content and code; bottom gives measurements
dedicated widgets; Trippy uses views suited to network questions. Yazi's terminal
fallbacks and Joshuto's optional icons also show that rich decoration need not be
a prerequisite for basic function.

Choose spacing, headings, alignment, labels and a restrained set of semantic
colors before adding borders or ornaments. Pair color with text or another cue.
Preserve meaningful whitespace and exact copyable content even when display
wrapping or highlighting changes its presentation.

For Helm, user messages, assistant answers, tool activity and decisions should be
distinguishable at a glance. Long activity sequences can have summaries without
erasing their results or making them inaccessible.

**Review question:** If decorative color and icons are unavailable, can the user
still identify roles, state, selection and actionable controls?

### 10. Design progress through the final outcome

Caligula makes verification part of the workflow, and distinguishes unfinished
work in exit handling. Trippy separates errors and no-data from ordinary results.
These are more useful models than a generic spinner followed by disappearance.

Describe the actual phase and the evidence available. Where totals are unknown,
use an honest activity indicator rather than invented percentages. An error
should identify what failed, what is retained and what action is available.
Confirmation should describe a concrete consequence and target, with friction
proportional to the risk.

For Helm, distinguish submission, admission, work, required input, completion,
failure and uncertainty. Cancellation requested remains different from observed
cleanup. Disconnecting Helm remains different from canceling a voyage.

**Review question:** After interruption or failure, can the user tell what happened
and whether trying again is safe?

### 11. Make customization extend a usable default

xplr exposes modes and layouts; Television packages domains as channels; Yazi,
bottom and Joshuto provide configurable presentation and behavior. Oatmeal connects
chat to editors. These mechanisms let a focused interface cooperate with existing
workflows instead of needing to implement every adjacent tool.

Start with a complete default journey. Keep theme changes separate from action
semantics, and make configured bindings discoverable. Extension failures should
be identifiable and recoverable. Reusing an interaction shell must not obscure
the different permissions and effects of the plugged-in operation.

For Helm, customization and integrations remain subject to the live registry and
executing voyage's local policy. This principle does not propose transferring
credentials or introducing an embedded executor.

**Review question:** Is the default useful before configuration, and can users
understand what a customization changes?

### 12. Use motion only when it serves the task

Gitlogue deliberately animates historical changes and supplies playback controls.
scope-tui and bottom visualize measured change. These are different uses of motion:
storytelling, signal display and operational observation.

Use motion to reveal sequence or show real change. Keep it interruptible when
users need to inspect, and avoid making essential information depend on waiting
through an animation. Offer a static or reduced-motion path in future designs
that introduce nonessential animation.

For Helm, stream received content promptly. An optional replay could explain
recorded changes, but it should be clearly labeled as replay. Artificial typing,
unearned completion effects and decorative activity should not imply work that
the runtime has not reported.

**Review question:** Does the motion communicate real information, and can the
user get that information without waiting for a performance?

## Resolving tensions

| Tension | Decision rule |
| --- | --- |
| Dense dashboard vs readable conversation | Use a dashboard for comparing concurrent measurements; give reading and writing a dominant content area. Offer explicit view changes. |
| Vim efficiency vs first-use discoverability | Keep visible actions and familiar navigation available; teach and optionally support expert bindings without stealing text input. |
| Minimalism vs explicit state | Remove repeated ornament before removing target, focus, status, required decisions or recovery information. |
| Immediate execution vs careful review | Make harmless navigation direct. Separate selection from effects; add concrete review where consequences warrant it. |
| Live reordering vs stable attention | Allow useful recency ordering while preserving the selected identity and reading anchor. Do not make a pointer action hit a newly substituted target. |
| Extensibility vs consistent behavior | Keep a shared action vocabulary, visible scope and a useful default. Extensions remain within the executing host's authority. |
| Animation vs speed | Use controllable motion for understanding; show results immediately when waiting adds no information. |

These rules resolve conflicts in the synthesis. They are not claims that all
reviewed apps already implement the chosen behavior.

## Applying the lessons to Helm

Treat the following as design directions, not a delivery schedule or newly
implemented capabilities. Evaluate them against the current source and existing
UX journeys before opening implementation scope.

1. **Complete the ordinary conversation loop.** Preserve readable text, obvious
   composer focus, stable reading position and direct access to activity details.
   Oatmeal, GitUI and bottom provide the closest lessons. Evaluate J07–J11.
2. **Make voyage discovery work at every size.** Explore a scoped searchable picker
   with name, host and state, retaining a visible route to actions when the sidebar
   cannot fit. Use Television, Atuin and Joshuto as references. Evaluate J05–J06.
3. **Make decisions and recovery complete journeys.** Show target and effect before
   approval, retain the input needed to recover, and distinguish uncertainty from
   failure. Use Caligula and Trippy as references. Evaluate J12–J16 and J28–J31.
4. **Add focused observation where it answers a real question.** Use summaries with
   drill-down for tasks, subordinate work and resource limits; use charts only when
   a time series or comparison helps a decision. Use bottom, Trippy and scope-tui.
   Evaluate J22–J23 and J32.
5. **Keep discovery consistent across surfaces.** Align visible actions, contextual
   help and command completion with the selected owner's capabilities. Use GitUI,
   Television, Joshuto and xplr. Evaluate J20 and J40.

The [architecture](architecture.md) remains the boundary: Helm presents and routes,
Vessel supervises independent voyage processes, and the voyage owns execution,
conversation and persistence. UI convenience cannot broaden execution policy,
expose private input or change ownership. Attractive presentation is not evidence
of cleanup, authority or successful execution.

## Practical design-review checklist

Use this for a concrete journey with representative content. It supplements
[UX readiness](ux-readiness.md); it does not record a passing review.

| Check | Evidence to look for |
| --- | --- |
| Orientation | The selected object, host where relevant, active scope and input destination are visible. |
| Discovery | A new user can find the primary action from the current screen; help reflects actual bindings. |
| Target review | A user can inspect the target and consequences before a consequential action. |
| Responsiveness | Slow data or previews leave input and navigation usable; delayed results retain correct attribution. |
| Focus safety | Popups, typing, hover, selection and confirmations do not silently redirect one another. |
| Search clarity | Search scope and result context are legible; loading, no-match and failure states differ. |
| Reading stability | New content does not displace an earlier reading position; returning live is explicit. |
| Space and fidelity | Narrow, resized and long-content views preserve required actions, Unicode and copyable text. |
| Outcome and recovery | Empty, pending, completed, failed, disconnected and uncertain states offer accurate next steps. |
| Consequence | Confirmation names the exact effect; cancel requested, cleanup and detach remain distinct. |
| Visual meaning | State remains understandable without color, special icons or animation. |
| Default and extension | The default journey is usable, and custom integrations retain visible scope and policy limits. |

Record actual observations and unresolved problems. Source inspection establishes
that a mechanism exists in the inspected files; hands-on review is still needed
to establish whether the resulting interaction works well.

## Source register

Links below identify the inspected snapshots, not a moving latest release.
Additional files in the same clones informed the review; these anchors provide
the direct evidence for the per-app mechanisms above.

| App | Snapshot | Evidence |
| --- | --- | --- |
| yazi | `5f901b886b14` | [Primary reference][yazi-readme]; [implementation or configuration][yazi-detail] |
| bottom | `b77d31750288` | [Primary reference][bottom-readme]; [implementation or configuration][bottom-detail] |
| gitui | `2fa693cb6ed4` | [Primary reference][gitui-readme]; [implementation or configuration][gitui-detail] |
| rainfrog | `d8b0c8d08e64` | [Primary reference][rainfrog-readme]; [implementation or configuration][rainfrog-detail] |
| television | `ee8bb99e0c18` | [Primary reference][television-readme]; [implementation or configuration][television-detail] |
| atuin | `740992f0018c` | [Primary reference][atuin-readme]; [implementation or configuration][atuin-detail] |
| oatmeal | `a6148b247477` | [Primary reference][oatmeal-readme]; [implementation or configuration][oatmeal-detail] |
| joshuto | `ac813526553e` | [Primary reference][joshuto-readme]; [implementation or configuration][joshuto-detail] |
| xplr | `d96991f6b1d1` | [Primary reference][xplr-readme]; [implementation or configuration][xplr-detail] |
| trippy | `2e69fcfee712` | [Primary reference][trippy-readme]; [implementation or configuration][trippy-detail] |
| scope-tui | `c6c6b4b74e85` | [Primary reference][scope-tui-readme]; [implementation or configuration][scope-tui-detail] |
| gitlogue | `8975613a6406` | [Primary reference][gitlogue-readme]; [implementation or configuration][gitlogue-detail] |
| caligula | `84ecaad5ef7c` | [Primary reference][caligula-readme]; [implementation or configuration][caligula-detail] |

[yazi-readme]: https://github.com/sxyazi/yazi/blob/5f901b886b14de1f17460b6e52e9de5d67f8aba9/README.md
[yazi-detail]: https://github.com/sxyazi/yazi/blob/5f901b886b14de1f17460b6e52e9de5d67f8aba9/yazi-config/preset/yazi-default.toml
[bottom-readme]: https://github.com/ClementTsang/bottom/blob/b77d3175028849824e987c35177e8f61450d72e7/README.md
[bottom-detail]: https://github.com/ClementTsang/bottom/blob/b77d3175028849824e987c35177e8f61450d72e7/docs/content/usage/general-usage.md
[gitui-readme]: https://github.com/gitui-org/gitui/blob/2fa693cb6ed431b21ebc300dd02e83c2476699ce/README.md
[gitui-detail]: https://github.com/gitui-org/gitui/blob/2fa693cb6ed431b21ebc300dd02e83c2476699ce/src/popups/help.rs
[rainfrog-readme]: https://github.com/achristmascarl/rainfrog/blob/d8b0c8d08e6473415bcab8171159a25049be699c/README.md
[rainfrog-detail]: https://github.com/achristmascarl/rainfrog/blob/d8b0c8d08e6473415bcab8171159a25049be699c/src/focus.rs
[television-readme]: https://github.com/alexpasmantier/television/blob/ee8bb99e0c1856d00125bb42f91af9ae463055ee/README.md
[television-detail]: https://github.com/alexpasmantier/television/blob/ee8bb99e0c1856d00125bb42f91af9ae463055ee/television/screen/help_panel.rs
[atuin-readme]: https://github.com/atuinsh/atuin/blob/740992f0018c3b668368319d79d08f9274e4ca1a/README.md
[atuin-detail]: https://github.com/atuinsh/atuin/blob/740992f0018c3b668368319d79d08f9274e4ca1a/crates/atuin/src/command/client/search/keybindings/defaults.rs
[oatmeal-readme]: https://github.com/dustinblackman/oatmeal/blob/a6148b2474778698f7b261aa549dcbda439e2060/README.md
[oatmeal-detail]: https://github.com/dustinblackman/oatmeal/blob/a6148b2474778698f7b261aa549dcbda439e2060/src/domain/services/bubble.rs
[joshuto-readme]: https://github.com/kamiyaa/joshuto/blob/ac813526553e8fd0b6bd22448e71a44431aa6477/config/joshuto.toml
[joshuto-detail]: https://github.com/kamiyaa/joshuto/blob/ac813526553e8fd0b6bd22448e71a44431aa6477/src/ui/widgets/tui_help.rs
[xplr-readme]: https://github.com/sayanarijit/xplr/blob/d96991f6b1d1ff9914716ebc1150cbb4a406abca/README.md
[xplr-detail]: https://github.com/sayanarijit/xplr/blob/d96991f6b1d1ff9914716ebc1150cbb4a406abca/docs/en/src/modes.md
[trippy-readme]: https://github.com/fujiapple852/trippy/blob/2e69fcfee7126c1f8449d8b907c21b07d68fad25/README.md
[trippy-detail]: https://github.com/fujiapple852/trippy/blob/2e69fcfee7126c1f8449d8b907c21b07d68fad25/crates/trippy-tui/src/frontend/render/body.rs
[scope-tui-readme]: https://github.com/alemidev/scope-tui/blob/c6c6b4b74e8593dbebfa645b63ae306c1243a0aa/README.md
[scope-tui-detail]: https://github.com/alemidev/scope-tui/blob/c6c6b4b74e8593dbebfa645b63ae306c1243a0aa/src/app.rs
[gitlogue-readme]: https://github.com/unhappychoice/gitlogue/blob/8975613a64067dd41c927237d06e4f7066cf9364/README.md
[gitlogue-detail]: https://github.com/unhappychoice/gitlogue/blob/8975613a64067dd41c927237d06e4f7066cf9364/src/ui.rs
[caligula-readme]: https://github.com/ifd3f/caligula/blob/84ecaad5ef7c304e0c15ffba3e0ce066ce3457d2/README.md
[caligula-detail]: https://github.com/ifd3f/caligula/blob/84ecaad5ef7c304e0c15ffba3e0ce066ce3457d2/src/tui/fancy_ui/state.rs
[xplr-layout]: https://github.com/sayanarijit/xplr/blob/d96991f6b1d1ff9914716ebc1150cbb4a406abca/docs/en/src/layouts.md
