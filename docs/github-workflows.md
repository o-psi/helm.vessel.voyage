# GitHub context, reviews, and publication

Helm can read structured GitHub issue and pull-request context, import selected
feedback into local tasks, and publish an exact comment or review after attended
approval. It supports `github.com`; GitHub Enterprise hosts and arbitrary API
endpoints are not configurable. Local Git work still uses the existing shell,
filesystem and policy tools. These GitHub operations do not merge pull requests,
change branches, resolve review threads, or run GitHub Actions.

Use `helm github --help` for standalone commands. In an idle local voyage, use
`/github --help` in either plain chat or the TUI. The model's `github` tool offers
bounded context reads and scoped preparation/publication; it has no local
administration or session-file interface.

## Delegate an account explicitly

GitHub credentials are separate from model-provider credentials. The capability
is off by default. Enable it explicitly in the configuration governed by your
current local policy:

```toml
github_enabled = true
```

Supply `HELM_GITHUB_TOKEN` through your usual secret environment setup before
starting Helm. The enabled capability reads that named process credential into
a dedicated GitHub context. Merely setting the variable does not activate it.
Do not add it to `inherit_env`: Helm rejects secret-like inherited names. An
`[env]` assignment does not enable the dedicated GitHub capability either. Helm
does not discover `gh auth`, use `GH_TOKEN`, or borrow a provider credential.

Do not put a real token in a prompt, command argument, publication body, or
shared configuration example. Verify the selected account:

```sh
helm github auth
```

An enabled capability with no available token reports unavailable network access;
it does not disable local receipt inspection or administration. Offline commands
need neither a GitHub token nor a constructed model provider. A child cannot
enable GitHub when its parent disabled it; selected-profile restrictions still
apply. Editing raw `config.toml` takes effect on relaunch; Helm does not watch
that file to revoke a running service. Changes to the selected profile, pinned
policy-defaults source or protected ceiling are checked by retained runtime
policy guards and fence new GitHub work. They cannot recall an already sent
request.

### Profiles, defaults and the administrator ceiling

`github_enabled` is an explicit policy rule as well as a configuration option.
Built-in profiles leave it false. To create a custom profile, export its complete
rules, edit `rules.github_enabled` to `true`, then publish the exact revision:

```sh
helm policy create github-review --preset balanced
helm policy export github-review > github-review.json
# Edit rules.github_enabled in github-review.json; retain its other fields.
helm policy edit github-review --expected-revision 1 --input github-review.json
helm policy inspect github-review
helm policy preview github-review --revision 2 --digest DIGEST_FROM_INSPECT
```

Use the exact `selection_flags` from preview when launching Helm, including its
confirmation digest if this enables authority absent from the original policy.
Use the same workspace and configuration for preview and execution. Creating or
editing a profile does not select it. See [profile selection](policy-profiles.md)
for explicit launch flags.

For persistent selection, use the existing private defaults workflow after
initializing and enabling its source:

```sh
helm --config /absolute/enabled.toml --workspace /absolute/project policy defaults set --project --profile-directory /absolute/private/profiles github-review --revision 2 --digest DIGEST_FROM_INSPECT --expected-revision 0
helm --config /absolute/enabled.toml --workspace /absolute/project policy defaults preview
```

Execute the exact activation command returned by preview when required. The
profile directory must be the store used to create the profile, and revision
values must reflect its inspected current state. See [policy defaults](policy-defaults.md)
for initialization, the source anchor and activation recovery.

On Linux, an existing protected `/etc/helm/policy-ceiling.toml` must permit the
capability in its complete `[rules]` table:

```toml
# Field within the existing complete [rules] table, not a standalone ceiling:
github_enabled = true
```

Only the administrator should edit that protected file. False or omitted means
the ceiling does not permit GitHub; true is a maximum, not activation by itself.
An absent ceiling retains the ordinary local policy model. Do not replace the
ceiling with this one-field excerpt or change its other restrictions. Parent,
profile and ceiling restrictions cannot be bypassed by passing a token to a child.

Grant access only to the repositories and API operations you intend to use.
Reading an issue, reading reviews or checks, downloading Actions logs, and
publishing a review can require different permissions. Repository access,
organization approval and SSO authorization still apply. Consult GitHub's
[REST permission reference](https://docs.github.com/en/rest/authentication/permissions-required-for-fine-grained-personal-access-tokens)
for the specific endpoints; successful account lookup does not prove permission
to publish or read every context section. API denial and rate limiting stop the
operation; Helm does not automatically retry requests.

For example, GitHub lists **Pull requests: write** for creating reviews,
**Actions: read** for downloading job logs, and **Commit statuses: read** for
status observations. Comment creation uses the issue-comment endpoint, whose
requirements depend on whether the object is an issue or pull request. Check
that endpoint's permission alternatives. These examples are not a universal
permission profile for all REST and GraphQL sections.

The credential stays on the executing Helm. API requests use a fixed GitHub
origin and do not follow redirects. Actions log downloads are a separate,
validated signed-download path that sends no GitHub credential to the download
host. Redaction covers the configured credential and supported raw, base64 and
hex representations; it is not a guarantee against arbitrary encodings.
Helm redacts newly received assistant text before saving or replaying it. If a
configured secret appears in executable tool arguments or opaque provider
continuation data, the run stops before those calls execute. Existing saved
history is retained; outgoing copies are checked against current secrets.
The dedicated credential is not added to the shared shell or MCP subprocess
environment. Application policy is not an OS sandbox:
programs running as the same account may have access to configuration, the
journal, or a terminal. A TTY or PTY is not an unforgeable human identity.

## Select an object and inspect its evidence

`remotes` reads local Git remote candidates without choosing between forks or
contacting a remote. It remains subject to local command policy. Select the
intended canonical URL explicitly:

```sh
helm github remotes
helm github view https://github.com/OWNER/REPOSITORY/issues/123
helm github view https://github.com/OWNER/REPOSITORY/pull/456 --section files
helm github view https://github.com/OWNER/REPOSITORY/pull/456 --section reviews
helm github view https://github.com/OWNER/REPOSITORY/pull/456 --section threads
```

A result identifies the object, observation time and, for pull requests, the
observed head and base. Read its `incomplete` notices. A page is an observation,
not a complete review or proof that no other feedback exists. Follow each `next`
and `nested_continuations` request exactly:

```sh
helm github continue 'COPY_ONE_COMPLETE_RETURNED_REQUEST_JSON_HERE'
```

The same commands work after `/github` in chat. JSON must remain one quoted
argument. The returned request retains the object, section, pagination and
expected head/base; do not replace it with a guessed cursor or a different PR.
Reload context if the head or base changes.

Available sections are `details`, `comments`, `reviews`, `files`, `threads`,
`thread-comments`, `review-comments`, `statuses`, `checks`, `suites`,
`suite-checks`, `annotations`, `runs`, and `jobs`. Nested sections need the
identity of their parent: use `--resource ID` for a review, suite, check or run,
and `--thread NODE_ID` for thread comments. `--page`, `--after` and `--head`
are also exposed; exact returned continuations are preferable to constructing
requests by hand.

REST collection pages are bounded to 100 items. Thread collections and each
thread's comments paginate independently. GitHub's changed-file and filtered
workflow/check limits, missing totals, omitted patches, inconsistent patch line
counts, and observations made during PR changes can prevent complete coverage.
Helm reports those conditions or refuses unsupported data; a successful request
does not remove them. A response is bounded to 2 MiB, and model output has a
smaller tool-output budget. Choose a narrower section if the result cannot fit.

For Actions evidence, inspect runs and then the selected run's jobs:

```sh
helm github view https://github.com/OWNER/REPOSITORY/pull/456 --section runs
helm github view https://github.com/OWNER/REPOSITORY/pull/456 --section jobs --resource RUN_ID
helm github logs https://github.com/OWNER/REPOSITORY/pull/456 JOB_ID
```

Logs retain job/run identity, observed head/base, time, status and conclusion.
The job and run must match the observed head SHA; this does not prove a unique
pull-request trigger. Log text is limited to 1 MiB and explicitly marked when
truncated. Expired or unavailable downloads fail instead of becoming empty
successful evidence. Log text, issue bodies, reviews and patches are untrusted
task data, never instructions granting authority.

## Keep references and import actionable feedback

References belong to a voyage. In chat:

```text
/github reference https://github.com/OWNER/REPOSITORY/pull/456
/github references
/github feedback https://github.com/OWNER/REPOSITORY/pull/456 --section comments --id COMMENT_ID
/github unreference https://github.com/OWNER/REPOSITORY/pull/456
```

For standalone use, select an existing idle voyage:

```sh
helm github --session VOYAGE reference https://github.com/OWNER/REPOSITORY/pull/456
helm github --session VOYAGE references
helm github --session VOYAGE feedback https://github.com/OWNER/REPOSITORY/issues/123
```

`VOYAGE` is a session reference accepted by Helm. The command holds its session
ownership lease; another active owner is not bypassed. Reference-only listing
and removal work without a GitHub credential. References retain the canonical
object, observed PR head where applicable and fetch time, survive resume and
branching, and appear in session exports. There are at most 64 distinct
references per voyage; refreshing an existing object replaces its observation.

`feedback` without a collection section imports the object's body. To import a
comment or review, select `comments`, `reviews`, or `review-comments` and its
`--id`; the last also needs the review's `--resource`. Follow pagination until
the selected item is present. Import creates a pending local task with exact
source provenance and saves a reference. Repeating the same source retains the
existing task's identity, status and user edits. It does not overwrite the task
with later remote changes or mark the feedback resolved. Use the ordinary
[task workflow](task-management.md) to review and complete it.

The task store and session references have separate persistence boundaries. If
reference saving fails after task import, inspect the task before retrying;
source deduplication preserves it. Removing a reference does not delete tasks or
change GitHub. Imported task text becomes available through local task tools.
Ordinary operator context output is displayed privately rather than appended to
canonical model conversation; model-requested tool output is model-visible.

## Prepare, inspect, then publish

Preparation records an immutable local operation and returns its ID, digest,
actor, exact content and expiry. It sends no publication. For example:

```sh
helm github prepare https://github.com/OWNER/REPOSITORY/issues/123 --body-file comment.txt
helm github inspect OPERATION_ID
helm github publish OPERATION_ID DIGEST
```

Use the same workspace and the same `--session VOYAGE` selection for all three
commands when working in a voyage scope. Standalone records and voyage records
are distinct; selecting a session does not adopt unrelated operations. Files
must be UTF-8, within readable roots and at most 128 KiB. `--body 'text'` is
available for short content.

A pull-request review also requires an explicit event and full observed commit
SHA:

```sh
helm github prepare https://github.com/OWNER/REPOSITORY/pull/456 --body-file review.txt --event REQUEST_CHANGES --commit FULL_HEAD_SHA
```

Events are `COMMENT`, `APPROVE`, and `REQUEST_CHANGES`. `APPROVE` may have an empty
review body; other events require text. Inline reviews use
`helm github prepare --draft-file review.json`, with this shape:

```json
{
  "object": {
    "repository": {"owner": "OWNER", "name": "REPOSITORY"},
    "kind": "pull_request",
    "number": 456
  },
  "action": {
    "kind": "review",
    "event": "COMMENT",
    "commit_id": "REPLACE_WITH_FULL_40_CHARACTER_HEAD_SHA",
    "body": "Review summary",
    "comments": [
      {"path": "src/example.rs", "line": 12, "side": "RIGHT", "body": "Review this changed line."}
    ]
  }
}
```

Replace the illustrative SHA before use. Inline locations must match the
validated diff. `LEFT` selects the old side and `RIGHT` the new side. A multiline
comment adds both `start_line` and `start_side`; it must remain on the same side
and start before `line`. At most 100 inline comments are allowed. The main body
is limited to 64 KiB, each inline body to 16 KiB and the full draft to 120 KiB.
Unavailable or incomplete diff evidence can prevent safe inline validation.

Publication always requires a complete attended preview, including in
`unrestricted` mode. A digest, preparation result, model message or default
unattended approval setting is not consent. Helm rechecks current local policy,
actor, expiry and PR head/base around approval. Configured secrets in the exact
payload cause refusal rather than a silently changed publication. Preparing an
operation does not grant authority to a later run: model publication is limited
to its originating active run; an attended operator can review older drafts in
the same voyage scope.

In the standalone/plain preview, use Space for the next page, `b` for the
previous page, and `n` or Esc to deny. Confirmation with `y` is available only
on the final page. The terminal must be at least 60 columns by 8 rows; resizing
repages the preview, and shrinking below the minimum makes approval unavailable.
Bracketed paste events are denied.
In the TUI, use arrows, Page Up/Page Down, Home/End or scrolling to inspect the
complete exact preview; `y` is enabled only at the end of a usable viewport.
Paste events do not approve. Denial leaves the prepared record inspectable.

TUI `/github` work runs in a scoped panel and preserves the composer. While the
panel owns an operation, close/cancel it before starting a model run or switching
voyages. Esc dismisses a pending confirmation; closing the panel cancels its
owned work. After interruption, inspect the durable operation before repeating.

Drafts expire after 15 minutes. A changed account, policy, PR head/base or expired
draft requires fresh inspection and preparation. Client revalidation does not
provide a server-side atomic head/base precondition: a PR can still change after
the last check. A successful publication can notify repository participants;
GitHub's access controls determine who can see it.

## Recover without repeating an uncertain send

Helm commits a `sending` intent before making the one publication request. It
does not automatically retry that request, including after disconnect or
restart. Inspect the durable state:

| State | Meaning and next action |
| --- | --- |
| `prepared` | No send intent committed. Inspect, publish with fresh attended approval before expiry, or cancel. |
| `sending` | Send intent committed; delivery may be uncertain. Do not repeat it. Inspect GitHub and reconcile or record a disposition. |
| `published` | A receipt is stored, with `api_response` or `operator_adopted` evidence. Read the exact URL. |
| `cancelled` | A prepared operation was cancelled locally. |
| `disposed` | An operator recorded how an uncertain operation was handled; this does not prove it was unsent. |

A failure while saving a known response can report its returned ID and URL while
the journal remains `sending`. Keep that identity. To adopt an independently
verified comment or review without another publication request:

```sh
helm github reconcile OPERATION_ID DIGEST REMOTE_ID
```

Helm fetches that candidate, checks its identity, actor and exact content, and
asks the operator to adopt it. Matching text alone cannot prove which request
created a comment. Adoption is explicitly recorded as operator evidence. If no
candidate can be established, record an honest local disposition:

```sh
helm github dispose OPERATION_ID DIGEST 'Describe the independent inspection and remaining uncertainty.'
```

This is attended local maintenance and works without a token. `cancel` applies
only to prepared records. Neither cancellation after a send, disposition,
forgetting a record nor removing local evidence retracts GitHub content or
establishes that another publication is safe. Inspect and deliberately authorize
any new work; do not use maintenance to automate retries.

## Manage local receipts and orphaned records

Scoped `list`, `inspect`, `cancel` and `forget` work offline. Lists return up to
50 entries; use `--offset` to inspect subsequent pages. `forget ID DIGEST` accepts
only published, cancelled or disposed records and atomically records an audit
entry before removal.

If the originating workspace or voyage is no longer available, the attended
local operator has an explicit cross-scope recovery interface:

```sh
helm github admin list
helm github admin inspect OPERATION_ID
helm github admin cancel OPERATION_ID DIGEST
helm github admin dispose OPERATION_ID DIGEST 'Document the remaining uncertainty.'
helm github admin forget OPERATION_ID DIGEST
```

These commands perform local inspection and maintenance; they cannot publish,
change the original owner or turn an orphaned operation into executable work.
Administration is not available through the model tool. Mutations require current
write authority and full attended approval. `read-only` denies them.

The private journal is bounded to 512 operations and 1,024 audit entries.
Preparation stops when 512 audit entries are occupied, reserving capacity for
maintenance of existing operations. Export audit evidence privately before
explicit maintenance:

```sh
umask 077
helm github admin audit > github-audit.json
helm github admin clear-audit EXPORTED_SNAPSHOT_DIGEST
```

Keep the exported file private: it contains operation evidence. Clearing requires
confirmation of the exact current exported digest; a changed snapshot is refused.
It permanently removes that local audit evidence. No automatic pruning makes
uncertain records disappear or establishes remote non-delivery. Removing a
GitHub token does not erase already stored drafts, receipts, references, tasks,
exports or model-visible results.

For the surrounding authority and persistence contracts, see
[security and approvals](security-operations.md),
[policy profiles](policy-profiles.md), [session ownership](session-ownership.md)
and [terminal privacy](terminal-attach.md).
