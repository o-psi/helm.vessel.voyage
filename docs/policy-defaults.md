# Persistent local policy defaults

This feature stores explicit operator preferences outside the repository. It does not
load policy from repository files, grant remote authority, or enforce an OS sandbox.
Supported rules and the mandatory Linux administrator ceiling are described in
[policy-profiles.md](policy-profiles.md). Explicit invocation selection takes precedence
and is independent of an unavailable defaults store. Children inherit the root's
narrowed effective policy and source-freshness checks; they never apply a broader default
again.

## Enable a source without overwriting configuration

First initialize a private directory. Its existing parent must already exist:

```sh
helm policy defaults init --directory /absolute/private/defaults
```

The response includes the new `store_id`. Preserve it for retries; initializing the
same directory with another ID refuses. Enabling creates a new configuration file:

```sh
helm --config /absolute/original.toml policy defaults enable \
  --directory /absolute/private/defaults --store-id UUID \
  --output /absolute/enabled.toml
```

The original bytes, comments, provider settings and environment values are preserved
verbatim. Only a `[policy_defaults]` anchor is appended. Output reports the new path
and anchor, never the configuration contents. Existing destinations are not replaced:
an exact retry succeeds only if the complete intended bytes already exist. A different
existing anchor or output requires an explicitly chosen new source/output. Without
`--config`, enable reads the ordinary configuration when it exists, otherwise starts
from an empty document. Subsequent commands and launches must use `--config` with the
new output path. This does not silently replace the user's normal configuration.

An absent anchor preserves existing behavior. A configured source that is missing,
corrupt, pending publication, or has the wrong store identity refuses execution. This
is ordinary single-user configuration: an operator can deliberately choose another
configuration. It is not a guarantee against the owner deleting/restoring all of their
configuration or private storage.

## Choose exact candidates and activate a workspace

Inspect a named profile first, then store its exact source/revision/digest:

```sh
helm --config /absolute/enabled.toml --workspace /absolute/project \
  policy defaults set --global --profile-directory /absolute/private/profiles \
  review --revision 1 --digest PROFILE_DIGEST --expected-revision 0
```

Use `--project` instead of `--global` for a private preference keyed to the canonical
workspace path. The preference is not written into that repository. Restrictive or
equal preferences may apply when a directory is replaced at the same canonical path;
escalation activation and active-agent freshness bind the actual directory identity,
so replacement cannot reuse a grant or preserve an active agent's authority. `list` is bounded and
paginated; `inspect --global` or `inspect --project` shows the exact current revision.
Changes accept optional `--operation UUID` for immutable exact retries. A revision
conflict requires inspecting the current record; never silently refresh the CAS value.

Precedence is explicit launch selection, private project preference, private global
candidate, then Config. Explicit invocation policy overrides follow the selected
profile and the mandatory ceiling remains the final maximum. A changed/deleted named
profile refuses; defaults never track its next revision automatically. Runtime guards
recheck the original workspace, candidate, profile and ceiling before new execution.
Existing effects are not instantaneously revoked.

Preferences are candidates, not proof that a workspace adopted their authority. Preview
uses the actual execution workspace, Config policy and explicit CLI overrides:

```sh
helm --config /absolute/enabled.toml --workspace /absolute/project \
  policy defaults preview
helm --config /absolute/enabled.toml --workspace /absolute/project \
  policy defaults activate --expected-revision ACTIVATION_REVISION \
  --confirm TRANSITION_DIGEST
```

Preview also emits the exact activation argument array and a shell-quoted command.
Repeat the exact policy overrides from preview. Activation binds the workspace identity,
configuration policy, explicit overrides, source identity/revision, current candidate,
and fresh administrator ceiling. It is not wildcard approval for other workspaces.
A launch within both the original Config authority and the relevant adopted/history
authority needs no prompt. Above-Config authority always requires activation of the
exact current candidate; identical rules at a newer candidate cannot reuse an old grant. An unconfirmed escalation refuses promptly,
including unattended use. An activation retry is historical receipt observation; it
cannot restore an old grant after the current source changes.

`clear --global --expected-revision REV` and `clear --project --expected-revision REV`
write tombstones. Clearing a project preference reveals the global candidate; clearing
a global preference reveals Config. Clearing never silently activates that fallback
for every affected workspace. Each workspace must preview/activate an escalation under
its own actual context. Existing default-derived agents fail freshness after relevant
changes. An explicit project choice does not depend on unrelated global edits.

The resolver compares potentially adopted relevant choices since the last exact
workspace activation. Merely storing broader B after restrictive A, then storing C
with B's rules, does not authorize C. If no adoption receipt exists, earlier relevant
restrictions remain comparison evidence. This can conservatively require review when
a workspace never launched an earlier candidate. Incomplete or unusable history causes
refusal, not an inferred grant. An explicit new launch selection is the recovery path
when historical comparison cannot be reconstructed safely.

## Persistence and frontend boundaries

Defaults use a separate version-1 private append-only log capped at 1,024 records,
64 KiB per record and 30 KiB per mutation. Config input/output is capped at 1 MiB.
The log uses
sequence/hash checks, publication witnesses, nonblocking shared readers and exclusive
writers. Hashes bind evidence; they are not authentication. Historical profile bytes
support comparison only. Current selected profile bytes are reread from their exact
source before they can become runtime policy. Only the exact valid pending candidate
can finish publication; malformed/partial evidence fails closed. Preserve uncertain
storage for investigation rather than deleting history or assuming rollback.

The Config anchor is intentionally serialized; runtime `Selection`, CLI override
metadata and effective policy snapshots are not persisted as execution authority.
Root reconstruction resolves the actual saved/managed workspace afresh. TUI subprocess
relaunch remains refused while a profile/default source is selected, so a temporary
configuration cannot discard invocation-specific restrictions. Live TUI switching is
separate work requiring observed resource cleanup and a safe in-process rebuild.

New enforcement and anonymous create-only config publication require Linux. Ordinary
runtime use on other supported platforms remains unchanged without this feature.
No native-platform verification is claimed from Linux fixtures.
