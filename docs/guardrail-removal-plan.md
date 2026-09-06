# Remove implicit barriers to authorized work

Planning only, 2026-09-06. Runtime changes below are not implemented by this document.
Tracking: [execution issue #5](https://github.com/o-psi/voyage/issues/5), with
[the operator's direction and scope](https://github.com/o-psi/voyage/issues/5#issuecomment-5561523660).
The [explicit token-removal clarification](https://github.com/o-psi/voyage/issues/64#issuecomment-5561542020)
reopens #64 for the new scope and supersedes its earlier mandatory token gate.

The operator directed: “we DONT want to get in the way of the user... plan which
guardrails to remove,” then clarified that token guardrails are unwanted too.
Helm should carry out authorized work without adding arbitrary stopping points or
asking the user to authorize the same work again. Restrictions explicitly chosen
by the operator or installed by an administrator remain enforceable.

## 1. Remove Helm's token gates first

Current [context accounting](../helm/src/context.rs) uses a 65,536-token default,
counts each serialized byte as a token, adds framing and output reservations, and
rejects requests locally when whole-turn omission cannot make them fit.
[Configuration](../helm/src/config.rs) also defaults output to 8,192 tokens.
The reported failure estimated 67,272 tokens after only a few visible tool calls.

- Remove the default context ceiling and local estimate-based refusal to dispatch.
  Do not replace them with a larger fixed ceiling or a more accurate mandatory
  token gate. Token accounting may inform diagnostics and context preparation;
  it must not independently stop authorized work.
- Remove Helm's default output-token cap and mandatory output reservation. Omit
  optional provider output limits unless the user requested one. Where a transport
  requires a value, use its supported capacity and expose that transport fact;
  do not substitute another arbitrary harness budget.
- Make ordinary voyages work without configuring `context_window` or `max_tokens`.
  Preserve deliberate operator limits as optional settings; distinguish them from
  application-supplied defaults. No default token, turn or spending budget.
- Handle actual provider context rejection automatically: retain canonical input
  and results, reduce the outgoing context, and retry a materially changed request.
  Support active-turn tool results, recoverable artifact references and semantic
  summaries, preserving tool-call/result validity and the user's current objective.
  Context preparation may happen proactively without making estimates an admission
  gate. Large instruction sets must also have an explicit recoverable path.
- Never replay completed tools or discard the saved conversation to recover model
  context. If the provider cannot accept even a useful reduced request, report that
  actual constraint and retain resumable work. Do not spin on unchanged rejections.

Acceptance: the screenshot's request passes local admission; requests over the old
default and active tool loops continue without a token-setting prompt; provider
context errors recover across native transports; default requests carry no optional
Helm output cap. Explicit limits still apply. Test malformed errors, cancellation,
failed summarization, replay metadata, restart, Unicode and unreducible input.
Related: [#64](https://github.com/o-psi/voyage/issues/64),
[#4](https://github.com/o-psi/voyage/issues/4).

## 2. Remove automatic deadlines that terminate useful work

The [agent](../helm/src/agent.rs) gives completion reconciliation one pass and
60 seconds before an incomplete/error path can cancel owned children.
[Defaults](../helm/src/config.rs) give commands 120 seconds, four provider attempts
and an eight-second maximum retry delay. A longer server `Retry-After` currently
prevents retry. [Questions](../helm/src/tools/questions.rs) reuse the command timeout.

- Remove the one-pass reconciliation rule and its 60-second deadline. An early
  final proposal should lead to continued useful reconciliation, with progress
  visible to the user. It must not trigger cancellation of still-useful children.
  Keep truthful completed, incomplete, blocked and cancelled outcomes.
- Remove default elapsed-time termination of commands and MCP work. Support
  explicit user deadlines separately from transport health checks. Long work
  remains observable, cancellable and owned until completion or deliberate shutdown.
- Remove the two-minute deadline on attended human answers. Waiting for input is
  a visible state; the user may answer, cancel or continue another activity.
  Unattended runs with genuinely missing required input report a recoverable blocked
  state rather than waiting on inaccessible stdin or inventing an answer.
- Remove abandonment solely because four transient attempts were used or the
  provider requests a wait longer than eight seconds. Honor server backoff, show
  retry state, and allow cancellation or an explicit user retry/deadline policy.
  Authentication, invalid requests and ambiguous side effects require their own
  recovery; do not blindly retry them. Prevent tight loops and duplicate output.

Acceptance: commands, clarifications and useful child reconciliation exceed the old
deadlines without being killed; transient failures recover after more than four
attempts and waits above eight seconds. Cancellation, process cleanup, explicit
deadlines, unavailable input, partial responses and unrecoverable failures remain
correct. Test actual provider/tool/PTY boundaries and late-result races.
Related: [#5](https://github.com/o-psi/voyage/issues/5),
[#82](https://github.com/o-psi/voyage/issues/82),
[#83](https://github.com/o-psi/voyage/issues/83),
[#21](https://github.com/o-psi/voyage/issues/21).

## 3. Honor authorization consistently

[Policy](../helm/src/policy.rs) defaults to prompting for writes and consequential
commands; ordinary MCP calls also request approval. Even unrestricted mode retains
workspace roots and default forbidden commands. The
[dedicated GitHub service](../helm/src/github/service.rs) additionally demands an
attended confirmation regardless of unrestricted or unattended-allow selection.

- Make new ordinary local voyages autonomous by default: authorized file changes,
  shell commands and enabled MCP tools run without repetitive confirmation. Keep
  read-only and approval modes as explicit choices, preserving existing explicit
  restrictions. Make effective authority easy to inspect.
- Remove implicit workspace-only access from autonomous defaults. The workspace
  identifies the working directory; the local account's OS permissions define
  available access unless an explicit profile or administrator imposes narrower
  roots. Restricted tools still validate canonical paths and symlink boundaries.
- Remove `shutdown`, `reboot` and `mkfs` from the built-in autonomous deny list.
  Keep explicitly configured denies. Fix command classification so an argument
  naming a file such as `reboot` cannot masquerade as a forbidden executable.
- Remove dedicated GitHub's unconditional second confirmation and attended-only
  rule. Apply the same effective authorization as other tools. Retain destination,
  payload, stale-state and uncertain-send validation without a compulsory extra
  approval ceremony for work already authorized by the user.
- Preserve reusable, scoped operator grants across continuations and delegated
  work. A grant does not authorize unrelated targets or survive revocation. Repo
  text, model assertions and tool output cannot create a grant. Background execution
  honors existing authority and does not silently create new authority.
- Remove blanket rejection of explicitly selected environment names containing
  `KEY`, `TOKEN`, `SECRET`, `PASSWORD` or `CREDENTIAL`. Restore expected nonsecret
  local environment behavior and support explicit credential bindings for tools.
  Provider credentials remain isolated; secret values never become model-visible
  configuration diagnostics, prompts, logs or session content.

Acceptance: fresh autonomous local work can edit files, build, use enabled MCP and
perform authorized GitHub publication without extra approval. Explicit restricted
profiles, admin ceilings, revoked grants and remote consent still block unauthorized
actions. Test harmless command arguments, authenticated local tools, secret canaries,
subagent inheritance, stale grants and untrusted instruction injection.
Related: [#70](https://github.com/o-psi/voyage/issues/70),
[#54](https://github.com/o-psi/voyage/issues/54),
[#68](https://github.com/o-psi/voyage/issues/68),
[#21](https://github.com/o-psi/voyage/issues/21).

## 4. Replace capacity dead ends with resource management

Current caps include 16 terminals and 128 KiB returned output by default;
[managed shell](../helm/src/tools/shell/managed.rs) caps tracked jobs at 64 and
capture at 8 MiB. Inputs, session files and completion records have additional byte
or record ceilings. These are separate from token budgets.

- Remove arbitrary terminal/job counts as task-failure triggers. Schedule against
  actual resources, queue work when necessary, and expose operator-configured
  concurrency limits. Preserve ongoing processes while waiting for capacity.
- Replace permanent tool-output truncation with bounded previews backed by durable,
  pageable full results. Spill large histories, ledgers and attachments to storage;
  chunk large input rather than rejecting it at an incidental in-memory limit.
- Keep bounded resident memory, transport/parser defenses and backpressure.
  Disk-full or real OS/provider limits must preserve existing work and explain the
  actual failure. Removing product caps does not mean unbounded allocations.
- Keep inference allowances unlimited by default. Enforce only explicitly requested
  budgets. Do not add model-turn, assignment-duration, total-child or nesting limits.

Acceptance: large outputs remain retrievable, long histories resume, large inputs
retain their content, and workloads beyond former counts queue or run successfully.
Exercise slow readers, storage exhaustion, concurrent writes, malformed frames,
restart, cancellation and bounded-memory behavior.
Related: [#31](https://github.com/o-psi/voyage/issues/31),
[#37](https://github.com/o-psi/voyage/issues/37),
[#71](https://github.com/o-psi/voyage/issues/71).

## Controls retained

Preserve explicit user restrictions and budgets, administrator ceilings, remote
authentication and sharing consent, credential confidentiality, subagent authority
inheritance, atomic persistence, stale-write detection, cancellation, and honest
process/recovery states. Keep terminal-control sanitization and malformed-input
defenses. These controls protect ownership and data; they must not become pretexts
for adding new defaults that stop authorized work.

## Delivery order, dependencies and evidence

1. Implement token-gate removal with working context recovery. Required provider
   request fields and explicit-setting provenance must be handled in the same
   delivery; merely deleting preflight while breaking long voyages is incomplete.
2. Implement continuous work/retry behavior and completion reconciliation. Build
   on context recovery so longer tool runs can actually finish.
3. Implement consistent autonomous defaults and reusable authority. This can run
   independently in an isolated worktree; coordinate environment and remote-tool
   changes with the policy owners. Remote enrollment never creates local authority.
4. Complete durable resource scaling, then run the combined useful-work scenarios.
   If this storage work is needed by context recovery, deliver that dependency
   alongside step 1 rather than deferring a required path.

For every step, add regressions proving the formerly blocked workflow succeeds and
the retained controls still work. Cover unit, deterministic native-provider,
integration, PTY/UI, failure/restart, concurrency and adversarial cases as applicable.
Run [the full Linux quality suite](quality.md) using `./scripts/check-quality` from
a clean committed worktree before publishing runtime changes. Record exact revision,
commands, exits and logs. Test affected wire/storage compatibility on both peers;
report native-platform evidence separately. Use approved, budgeted behavioral/live
evaluations for useful context recovery and sustained work; offline fixtures alone
cannot prove summary quality or model completion behavior. Track those outcomes in
[#22](https://github.com/o-psi/voyage/issues/22) and #83. Never count an unrun check
as passing or close broad issues for a partial delivery.

Current status: planning complete; all runtime implementation and acceptance above
remain outstanding. No implementation access blocker was tested. Audit source was
`79442bc19e6c33b8f498b29fb83602658d5fb6fe`; the complete 88-issue inventory and relevant
discussions were inspected with two independent source reviewers. This document's
validation is limited to source references, links, command accuracy and whitespace;
runtime, provider, platform and behavioral tests are not applicable to this
documentation-only change and were not run.
