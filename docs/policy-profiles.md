# Policy profile resolution contract

The `policy_profile` library provides typed profile documents, preset expansion,
effective-rule resolution and a protected Linux system-ceiling reader. Ordinary
Linux CLI, TUI, managed-session, saved-workflow and child execution now enforce
the administrator ceiling through matching runtime policy and environment rules.
See [runtime boundaries](policy-ceiling-runtime.md) for enforcement points and
limitations. Explicit named-profile CLI management and launch selection are
implemented below. [Persistent private defaults](policy-defaults.md) are available;
the TUI can review and switch the active workspace runtime as described below.

Policy layers named global, project and session refer to local policy resolution;
they do not define a mandatory voyage project or a fleet-wide grant. Planned
[multi-Helm scope](voyages.md) selects eligible Helms, while each executing Helm
resolves its own effective policy. A coordinator or interface cannot broaden those rules.

## Typed rules and presets

Schema 1 profiles contain a name, positive revision and complete rules. Restricted,
Balanced and Autonomous expand respectively to read-only, approval and unrestricted
access. All three preserve workspace read/write roots, deny `shutdown`, `reboot`
and `mkfs`, inherit only the names `PATH`, `LANG`, `LC_ALL`, `TERM`, and deny
unattended automatic approval. Read-only access still disables mutations; a listed
write root does not override that mode.

Rules support access, unattended approval, read/write directories, denied command
basenames, inherited environment **names**, and the default-off
[`github_enabled` capability](github-workflows.md#profiles-defaults-and-the-administrator-ceiling). Roots are absolute existing directory
paths or the literal `$workspace`; they are canonicalized before comparison.
Relative paths and parent traversal are rejected. The resolver preserves current
Policy's implicit workspace roots and records when that supplies an omitted root.
It cannot express excluding the workspace while otherwise granting arbitrary tool
access.

Unknown fields and schema versions fail closed. Profile JSON is bounded to 64 KiB,
each collection to 128 entries, names to 128 bytes, and root spellings to 4,096
bytes. Export serializes the typed document only: no Config environment values,
API keys, MCP credentials, provider configuration or redaction values are read or
copied. This is not a generic secret detector for text an operator deliberately
puts in a path/name field.

There is no general network policy, OS sandbox, arbitrary tool-grant scheme, secret store or
restored subagent lifetime/child/history cap. Unsupported declarations are rejected.
These rules are application policy, not process containment.

## Resolution and provenance

`resolve_current(workspace, base, layers)` is the public profile-layer resolution
entrypoint. It always checks `/etc/helm/policy-ceiling.toml` on Linux; there
is no public alternate path, environment override or skip flag. Effective fields
and workspace identity are private and exposed through read-only accessors.
They cannot be deserialized from untrusted input as a resolved policy.

After the caller translates existing access settings into base rules, layers must
be ordered global, project, session, explicit, with at most one of each. An override
replaces its whole field rather than implicitly appending grants. `Layer::from_profile`
binds provenance to the validated profile name, revision and document digest.
Explicit field overrides have their own identity and digest. The pure resolver
operates on prepared canonical inputs; filesystem reads occur before it.

Each field retains contribution history, including explicit assignments that happen
to keep the same value and the final system-ceiling restriction. Roots and sets are
normalized deterministically. The effective digest binds final rules, source history,
ceiling digest and canonical workspace device/inode identity. Profile revisions
with identical rules still have distinct provenance. A digest is binding evidence,
not authentication or proof of operator interaction.

Repository files and session transcripts are not automatically loaded as layers.
Named-profile selection derives inputs from explicit local operator choices and
retains pinned revisions; imports remain inert until selection.

## Protected Linux ceiling

The protected file is strict TOML with `schema = 1` and a `[rules]` table containing
the same complete rule fields. An absent source leaves base/layer rules intact.
An existing unreadable, malformed, oversized or untrusted source fails closed.
The loader pins `/`, `etc`, `helm` and the file through directory-relative no-follow
opens. Every component must be root-owned and not group/other writable. The file
must be a regular single-link file; symlinks, hardlinks and FIFOs are rejected.
Pinned identities and metadata are checked again after the bounded read. There is
no automatic creation, repair or rewriting of administrator state.

The ceiling intersects read/write roots and inherited names, unions command denies,
and selects the more restrictive access and unattended-approval modes. The workspace
must itself fit **both** ceiling root sets, even in read-only mode, because existing
Policy always includes it. Empty ceiling roots therefore refuse the workspace;
they do not trigger an unrestricted fallback. Canonical root intersections may
narrow an extra root to an allowed descendant, never widen it.

Root-owned administrators can intentionally change or remove the ceiling. This
reader is not protection against root, hostile same-UID process manipulation,
filesystem rollback, or instantaneous revocation of already-dispatched effects.
Runtime builders resolve the current ceiling, and reused agents check it again
before new turns. A changed ceiling requires restart or rebuild. The public
profile-layer resolver refuses on non-Linux systems; ordinary non-Linux startup
retains its existing Config adapter without claiming ceiling enforcement. Schema
operations remain portable. Native platform validation is not claimed.

## Transition confirmation and integration requirements

`transition(previous, proposed)` requires matching canonical workspace filesystem
identity. Increasing access, enabling unattended approval, adding root/environment
grants, or removing a command denial requires confirmation. A mixed restrictive and
permissive change counts as escalation. Narrower roots and stricter rules do not.

The transition digest binds both effective digests and workspace identity. Comparing
an exact digest confirms only that metadata. A cached transition or successful
`confirm` call is **not** an execution permit. The caller must freshly resolve the
current ceiling and pinned profile revisions before committing a transition.

Current runtime builders give Policy, tool environment, child policy and terminal
managers the same effective rules. Explicit Config and MCP environment values
retain precedence only where the administrator environment-name ceiling permits
them. In-process profile switching stops and observes prior owned effects before rebuilding
and showing a new effective-policy label. A missing approver must never cause an
unattended wait or authority promotion. Explicit named-profile launch selection is
implemented below. Persistent private defaults are described in
[policy-defaults.md](policy-defaults.md).
The [TUI switching workflow](#switch-policy-in-the-tui) documents idle handoff,
confirmation and recovery requirements.

Tests exercise strict schemas, presets, source history, root/set intersections,
escalation and stale confirmations, canonical aliases/recreated workspaces, and
protected-source ownership, permissions, links, malformed data and replacement.
Filesystem tests use private internal injection; they do not modify operator `/etc`.

## Explicit named-profile CLI lifecycle

`helm policy` manages a private store under the Helm config directory's `profiles`
child, or an explicit absolute `--policy-directory` whose parent already exists.
Default administrative initialization creates missing parent directories privately
through pinned no-follow components; runtime freshness never initializes them. It never selects a profile
merely because one was created, imported, or found in a repository. Built-in
`restricted`, `balanced`, and `autonomous` profiles are immutable. Custom profiles
have a current revision and stable incarnation ID; deletion leaves a tombstone.
Recreating a deleted name increments its revision and gives it a new incarnation.

```sh
helm policy list
helm policy create review --preset restricted
helm policy inspect review
helm policy duplicate review review-copy
helm policy export review > review.json
helm policy edit review --expected-revision 1 --input review.json
helm policy import imported-review --input review.json
helm policy delete review-copy --expected-revision 1
```

Creation/import/duplication use expected revision zero by default. Recreating a
name requires its tombstone revision. Edits require the document's name to match;
imports deliberately copy only its rules into the explicitly chosen destination.
Source revisions are inert metadata, not authority over destination history.
Exports contain only the typed name, revision, and rules. There is no arbitrary
file overwrite in the exporter: stdout redirection is an operator choice.

`list` includes tombstones and supports `--after NAME --limit N` (1–100). `inspect`
returns the current snapshot and its digest. Mutations print an operation UUID
before publication; provide `--operation UUID` and the identical original payload
for an uncertain retry. A historical duplicate returns its original receipt even
if later edits exist. It does **not** restore or activate that older revision.
Changed payloads under the same operation ID are refused. Duplicate-source retries
must still describe the same copied rules; if the source changed, use an original
reviewed export with `import` and the original operation ID instead.

Select a profile explicitly for one invocation:

```sh
helm --config /absolute/config.toml --workspace /absolute/project \
  policy preview review --revision 2 --digest DIGEST_FROM_INSPECT
# Repeat the same Config/workspace/explicit policy overrides, then add the emitted flags:
helm --config /absolute/config.toml --workspace /absolute/project \
  --policy-profile review --policy-revision 2 --policy-digest DIGEST_FROM_INSPECT \
  run 'Review this project'
```

Preview reports previous and proposed effective rules and provenance, plus exact
`selection_flags`. When confirmation is required, those flags include
`--policy-confirm TRANSITION_DIGEST`. No interactive confirmation wait is added;
missing, stale, or wrong confirmation refuses the invocation before provider/tool
construction. The digest binds this Config's policy, explicit CLI overrides, exact
profile revision/incarnation and canonical store directory, canonical workspace
identity, and current system ceiling. Identical exported rules in a recreated or
relocated store require a fresh selection and transition preview. This
is an explicit local launch choice, not cryptographic authentication of a person.

The actual precedence is Config base, selected profile, explicit policy CLI
options (`--access`, `--approval`, and supported policy `--set` keys), then
the mandatory administrator ceiling. The comparison baseline is the actual Config
including those same explicit overrides. Explicit Config/MCP environment values
retain existing precedence and are restricted only by the actual administrator
name ceiling; they are never copied into profile provenance or preview.

Shared ordinary, plain-chat, TUI, managed-submit, workflow-run, and model-discovery
builders consume selection. Before a new run they reread its exact current revision
and the protected ceiling; changed/deleted profiles or a changed transition require
explicit reselection. Children receive resolved parent maxima, not a second
application of a broader profile. Their inherited root selection freshness still
refuses new work after a parent-profile edit. In-flight effects are not instantly
revoked; application policy is not an OS sandbox.

Selection survives in-process Config clones and overrides but is deliberately
omitted from persisted Config and sessions. Resume requires explicit reselection;
otherwise it uses the invocation's ordinary Config and ceiling. The selected
workspace must match the actual saved workspace. Supply that explicit `--workspace`
when previewing and resuming; Helm does not silently rebind a reviewed transition.
Profile selection is not accepted by unrelated administrative commands.

TUI subprocess relaunch requests (including `/plain`, `/verbose`, and commands that
leave the UI and return) are refused while a profile is selected. This prevents a
serialized temporary Config from dropping a restrictive selection. Exit normally,
preview/reselect explicitly, and launch the requested frontend. Persistent global/project preferences use an explicit private source anchor; see
[policy-defaults.md](policy-defaults.md). For in-process switching, see
[Switch policy in the TUI](#switch-policy-in-the-tui). Other-platform
schema/storage administration remains available, but explicit profile enforcement
requires Linux; ordinary no-profile behavior retains existing platform support.

## Private publication and recovery boundary

The private store uses the reviewed local-actor directory-relative storage helper,
nonblocking exclusive mutation/bootstrap locks, shared initialized-read locks, and
no-clobber immutable publication. Selection freshness never creates a missing
directory/lock/header or fsyncs state. Concurrent readers can coexist; a conflicting
writer still causes explicit Busy refusal rather than cached authorization. History is
bounded to 1,024 operations. Each change is at most 30 KiB, each immutable record at
most 64 KiB, and documents retain the schema's collection/name/path bounds. A
publication witness prevents a missing final record from silently exposing an
older revision. Hashes bind records and requests; they are integrity evidence, not
authentication or protection against an owner deliberately restoring the complete
store from an older backup.

A valid staged record is never selected by reads. Its exact operation can finish
publication; a different operation is refused. Malformed or partially written
candidates/witnesses fail closed and are **not** automatically repaired, discarded,
or guessed from an unrelated retry. Stop affected Helm processes and preserve the
whole directory for inspection. To recover manually, choose a new private directory,
import only previously exported rules that you explicitly review again, and obtain
new revision/digest/transition selections. Do not overwrite evidence or reuse old
selection receipts as permission. History exhaustion uses the same explicit new-store
workflow; there is no destructive automatic compaction. Locks are never retained
across network, provider, approval, or Journal operations.


## Switch policy in the TUI

Press **Ctrl+P** or enter `/policy` to browse private profiles. Use
`/policy "/absolute/private/directory"` for another store. Opening the picker may
initialize inert preset metadata; it does not select a policy. **I** shows the
running effective rules and their field provenance even if profile sources are
unavailable. Profile creation, editing, duplication, deletion, import/export and
persistent-default management use the CLI commands above and in
[policy defaults](policy-defaults.md).

Select a profile or **Use launch defaults**, then press **Enter** to review the
current and proposed rules, provenance and exact review receipt. The proposal is
compared with both the running policy and the original launch configuration.
An authority increase requires **Y**; Enter and pasted text cannot confirm it.
Otherwise Enter applies the reviewed policy. Escape cancels. Arrow keys and
Page Up/Page Down scroll the review; Home returns to its beginning. Controls and
bidirectional formatting in metadata are made safe for display, and known runtime
secrets are redacted without altering the rules being checked.

Preview never stops work. Apply refuses while a root run, child, outstanding
model/title/supervisor request, terminal or unobserved shell effect remains. Finish or cancel
work first; explicitly terminate retained terminals through the process tool before
retrying. If stale policy prevents cleanup requests or retained child resources cannot
be controlled from the current workspace, quit Helm for owned cleanup, then reopen
under freshly resolved policy. Helm rechecks profile revision/source identity, workspace identity,
explicit overrides, defaults and the administrator ceiling before handoff. A
changed review must be opened again; old confirmation cannot authorize new rules.

If saving the voyage fails, confirmation leaves the review and draft open with
the current runtime intact. Repair session storage and retry.

An accepted idle handoff blocks old runtime admission and observes owned cleanup
before releasing its persistent writer. Linux shell, PTY and MCP cleanup observes
the original process session, including ordinary forked descendants; this is not
OS containment and cannot account for processes deliberately escaping that session.
On other platforms, handoff from a runtime allowing shell or MCP effects is refused
because equivalent cleanup observation is unavailable. Ordinary shell execution
and direct-child MCP cleanup on those platforms remain available. Native macOS and
Windows validation has not been performed for this workflow.

If cleanup cannot be confirmed, the old runtime remains blocked with its ownership
retained. If constructing the replacement fails, Helm restores only the exact
previous effective policy after fresh validation. If that policy cannot be restored,
Helm exits with the voyage and draft saved; reopen under a freshly resolved policy.
A successful switch applies to the current workspace runtime, including its child
policies. Other cached workspaces keep their existing authority. Selection is not
written as session or Config authority and does not change persistent defaults;
ordinary restart resolves the configured launch policy afresh.
