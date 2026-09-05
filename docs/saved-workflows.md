# Saved workflows

Save repeatable tasks as TOML files and invoke them with validated inputs. Public
inputs become model-visible data; explicitly bound secrets remain transient.
A workflow supplies a prompt, not permissions: Helm keeps your selected provider,
model, workspace, access mode, approvals and resource limits.

Place personal definitions in the `workflows` directory beside Helm's default
configuration file (normally `~/.config/helm/workflows` on Linux). Repository
files belong in `<workspace>/.helm/workflows`. Discovery uses the selected workspace
only; it does not search ancestor repositories. `--user-directory DIR` selects an
explicit personal workflow directory. Missing directories contain no workflows.
The filename must be `<id>.toml`.

For example, save `review-change.toml`:

```toml
schema_version = 1
id = "review-change"
version = "1.0"
description = "Review a change and explain outstanding risks"
prompt = "Review {{target}}. Produce at most {{count}} findings."

[parameters.target]
type = "string"
required = true
description = "File or change to review"
max_length = 1024

[parameters.count]
type = "integer"
default = 5
minimum = 1
maximum = 20
```

```sh
helm workflow list
helm workflow inspect review-change
helm workflow validate review-change
helm workflow preview review-change --input target=src/main.rs
helm workflow run review-change --input target=src/main.rs --input count=3
```

Use `--json` for structured list, inspect, validate and preview output. `run` streams
ordinary agent output and reports the saved session ID on stderr; it does not accept
`--json`. Inspection and preview do not load provider configuration, initialize
tools, contact a model, or create sessions. For these commands select a workspace
with `--workspace`; a `workspace` entry in provider configuration is not consulted.
Execution resolves the normal configured workspace.

Repository definitions take precedence over personal definitions with the same ID.
`list` shows both sources; `--scope user` or `--scope repository` selects one
explicitly. An untrusted repository definition never falls back silently to a
personal definition. Built-in CLI and slash-command names are reserved.

Before previewing or running a repository definition from the CLI, inspect it and supply the
exact SHA-256 digest as an explicit trust decision:

```sh
helm --workspace ./project workflow inspect review-change
helm --workspace ./project workflow preview review-change \
  --input target=src/main.rs --trust-repository SHA256_FROM_INSPECT
helm --workspace ./project workflow run review-change \
  --input target=src/main.rs --trust-repository SHA256_FROM_INSPECT
```

The decision applies to the exact bytes loaded for that invocation. Changing even
whitespace changes the digest; no trust decision is saved automatically. Definitions
are read once into bounded memory, so publication of a different file afterward
cannot change the reviewed invocation. Trust in workflow instructions does not grant
access to files or commands, and does not make repository guidance trustworthy in
other contexts. Concurrent edits require inspecting the resulting digest again.

Parameters support `string`, signed 64-bit `integer`, and `boolean` (`true` or
`false`). Every declaration can include a description, `required`, `default`, and
`choices`. Integers support `minimum`/`maximum`; strings support a byte-count
`max_length`. Missing optional parameters without defaults render as JSON `null`.
Unknown or duplicate inputs, invalid defaults, out-of-range values and unknown
fields fail before provider execution. Missing required inputs produce an error;
this CLI does not read a competing input prompt.

`{{name}}` substitutes the parameter as a JSON value: strings include quotes and
escape newlines, quotes and controls. Substitution is single-pass; parameter values
containing more placeholders, shell syntax or environment names remain literal
text. There is no shell, expression evaluation, file inclusion or environment
expansion in rendering. These are model-visible data, not an OS sandbox or a promise
that a model cannot follow instructions inside supplied data. All resulting tools
remain governed by ordinary local policy.

Optional `[recommended]` fields `provider`, `model`, and `access` are shown by
inspection but never applied. Workflows cannot configure tools, environment,
credentials or access roots. Secret declarations (`secret = true`) use the isolated
binding path described below. Secret defaults and choices are invalid. Never put
credentials in workflow files or ordinary `--input` values: nonsecret inputs are
model-visible and recorded.

Saved sessions include default-compatible `workflow_runs` metadata: identity,
version, source scope, definition digest and resolved nonsecret inputs. This records
an invocation attempt, not a success claim. The ordinary checkpointed executor
records completion, failure or interruption; cancellation preserves accepted history.
Session branches preserve their source workflow history. `--no-save` skips session
documents but does not disable normal runtime tool effects. There is no workflow
resume/replay command or automatic execution on startup.

Limits: 128 directory entries per scope, 64 KiB per document, 32 parameters,
32 KiB prompt templates, 8 KiB per string input/default, 64 choices per parameter,
128 KiB rendered prompts and 128 workflow records per session. Names are lowercase
ASCII identifiers, at most 64 bytes. Versions are at most 64 ASCII letters, digits,
periods, underscores or hyphens. Invalid discovered TOML definitions fail the command
rather than choosing another source. Repository path components and definition files
cannot be symlinks; files must be regular and bounded. Personal directory selection
is an explicit local operator choice.

The offline system fixture exercises real CLI/native HTTP execution, exact trust,
changed definitions, precedence, literal input, metadata before dispatch, ordinary
policy denial, provider failure, cancellation, no-save and secret rejection. Unit
coverage includes parser/render bounds, malformed data, file confinement, session
round trips and branching. Native macOS/Windows execution is not claimed; Linux is the verification gate. No live provider evaluation accompanies
this prompt-format feature.

## TUI workflow forms

In an idle conversation, use `/workflow` to browse both sources. Select a definition
with Up/Down and Enter, or open `/workflow review-change --scope user` directly.
The `/workflow` namespace keeps saved IDs separate from built-in slash commands.
Discovery is explicit and read-only; workflows never run automatically on startup.

The form shows one named, typed input at a time. Tab/Shift-Tab moves between fields;
Enter validates every field and opens a rendered preview. Type `true` or `false`
for boolean inputs, a decimal integer for integer inputs, or literal text for a
string. Choices and bounds remain enforced by the same decoder as the CLI.
Untouched fields use defaults or optional `null`; Ctrl-U resets a field to unset.
An explicitly edited empty string differs from unset. Shift-Enter inserts a newline;
bracketed paste and Unicode editing stay within the selected field's 8 KiB limit.
Cursor navigation alone does not change an unset field. Backspace/Delete begin an
explicit edit. Nonsecret inputs are model-visible and saved. Secret fields use an
independent clearing buffer and display only `[hidden]`; previews show public
references. Closing the form drops its private values. Preparation rejection keeps
the private form values available for correction without recording them.

Review the rendered prompt, identity, version, source and SHA-256 digest. Up/Down
or PageUp/PageDown scrolls the preview. For a repository definition, press plain
`t` to trust exactly the displayed digest; plain `r` requests execution. Modifier
shortcuts do not grant trust or run a workflow. Personal definitions need no repository
trust decision, but still require the preview and explicit Run action. Before dispatch,
a fresh bounded discovery must resolve the same identity, scope and digest. A changed
or removed definition is rejected; reopen it to review new bytes. The captured
definition supplies the prompt, so a later filesystem edit cannot replace it. Trust
is not saved or reused by another invocation. Recommended settings remain advisory.

Esc returns from preview to editing, then closes the form without discarding the
underlying composer draft. Invalid inputs and run-preparation errors keep the form
values and create no invocation. Questions, approvals and attached PTYs retain their
existing input priority. The form does not consume direct PTY input. Discovery runs
outside the input loop with a five-second response budget; closing the panel invalidates
late results. A timed-out read-only filesystem worker may still finish independently.

An active run rejects workflow invocation; it cannot create a second run or silently
turn the slash command into model steering. Accepted workflows use the ordinary TUI
policy check, session ownership, completion scope and checkpointed Agent path. The
rendered user message and `workflow_runs` metadata are saved together before provider
dispatch. Metadata records an attempt, not completion. Canonical save failures retain
the existing fatal/recoverable storage behavior with zero dispatch; an uncertain save
is not presented as a safe retry. Cancellation and partial output use ordinary run
summaries and checkpoints. There is no separate workflow executor or policy expansion.

`tests/system/tui_workflows.py` exercises actual PTY input, native HTTP, exact digest
changes, defaults, literal input, metadata-before-dispatch, ordinary policy denial and
cancellation. Unit/routing tests cover malformed values, stale discovery, secret
masking/cancellation, modifier keys, draft retention, input isolation, narrow rendering, run limits
and canonical save failure. This is deterministic offline evidence; no new live model
or native-platform acceptance is implied. Plain-mode missing-input prompting remains
separate #67 work; unattended calls must supply required inputs explicitly.


## Transient private shell bindings

Declare a secret parameter without a default or choices. Its type and bounds still
apply. For CLI execution, pass the **name of an existing environment variable**:

```sh
helm workflow preview private-check --secret-env token=OPERATOR_TOKEN
helm workflow run private-check --secret-env token=OPERATOR_TOKEN
```

The preview validates input names and completeness but never reads `OPERATOR_TOKEN`.
Execution reads that source explicitly; it does not expand template text or import
other environment variables. Missing/non-UTF-8 sources fail with a fixed diagnostic.
In the TUI, enter the secret in the masked field instead. Do not send secret values
through ordinary messages, steering, Questions, `--input`, or command-line arguments.

A `{{token}}` placeholder becomes the public JSON reference
`{"workflow_secret":"token","environment":"HELM_WORKFLOW_TOKEN"}`. Optional unbound
secrets become `null`; required secrets must be supplied. Secret names are normalized
to uppercase with hyphens replaced by underscores after the `HELM_WORKFLOW_` prefix;
colliding names are rejected. Generated environment keys and their configured case
aliases are reserved consistently across platforms; a private binding cannot replace
a configured alias. This deliberately rejects some names that Unix would distinguish.
Only nonsecret values enter `workflow_runs.inputs`.
References reveal the parameter's name and intended environment key, never its value.

The provider may explicitly request the existing one-shot `shell` tool with
`"workflow_secrets":["token"]` beside its normal `command`. The registry resolves only
names bound to the exact accepted run, checks for configured-environment conflicts,
and leaves ordinary roots, command rules, approvals, cancellation and deadlines in
force. A private call additionally rechecks current policy before approval and before
spawn, including after managed worker scheduling. A stale selected profile refuses
the new private environment. These are bounded checks, not atomic revocation: a later
policy change does not stop an existing subprocess, and ordinary tools retain their
existing new-turn freshness contract. Unknown/stale references and bindings requested
by other tools fail. Plain shell calls without the field receive no workflow secret.
Bindings are not passed to PTYs, MCP tools, subagents, later runs or resumed sessions.
Persisted metadata cannot recreate binding authority.

For a bound shell call, stdin, stdout and stderr are connected to the null device
**before spawn**. No output is captured, truncated, decoded or redacted: even short,
Unicode, split or encoded secret output cannot enter the tool result. The result is
fixed typed JSON, `{"status":"exited","code":0}` (with the actual exit code) or
`{"status":"signalled"}`; failures use fixed diagnostics. This intentionally prevents
reading ordinary stdout from the same private call. Use a separate unbound operation
for public output, and never write private values into files that Helm will read.

This is application-level data routing, **not an OS sandbox**. Authorized commands
can still write files, access the network, or create descendants under existing local
policy. The feature does not promise to erase OS/environment copies or stop escaped
processes. Normal one-shot shell lifecycle limits remain; the managed adapter retains
its existing bounded session observation and cleanup obligations. Clearing buffers
on drop is best effort, not a guarantee that every allocator, terminal or OS copy is
zeroized. Cancellation and retry never silently carry bindings into another run.

`tests/system/workflow_secrets.py` checks actual native HTTP, CLI and PTY flows,
private environment use verified by a digest artifact, public-only canonical records,
no-save, selected-profile denial and stale-profile refusal. Unit tests cover exact run
fencing without completion scope, short/Unicode and split/encoded output, typed bounds,
unknown/duplicate refs, environment conflicts, approval denial, timeout, cancellation,
abandoned execution and observed managed cleanup. Linux validation is required;
macOS/Windows native tests and new live-model evaluations are not part of this slice.
