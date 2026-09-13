[Comparative audit and evidence limits](ux-comparison.md)

# Helm CLI action inventory — UX audit #271

Source-only audit of the live checkout. No binaries, builds, tests, provider calls, GitHub calls, or CLI mutations were executed. Commands below are **usage grammar, not verification that they ran**. This appendix was prepared from an ignored research report and published with the audit. TUI interaction design is out of scope; CLI-to-TUI entry boundaries are included.

## Scope and evidence

Primary grammar: [helm/src/cli.rs](../../helm/src/cli.rs); dispatch: [helm/src/main.rs](../../helm/src/main.rs). Nested shared clap types often live under `voyage/src`, re-exported by [helm/src/lib.rs](../../helm/src/lib.rs); they must not be mistaken for extra `voyage` executable commands. Architecture/current behavior consulted in [current-state](../../docs/current-state.md) and [development](../../docs/development.md).

Observed initial dirty state (not ours): modified `coverage/latest.json`, `docs/duplex-transport.md`, `helm/src/process_client/duplex.rs`; staged modification `helm/src/process_client/ui/render.rs`; deleted `scripts/check-quality`, `scripts/forgejo-server`, `scripts/forgejo-start`, `scripts/forgejo-stop`, `scripts/local-git`, `scripts/local-git-server`, `scripts/package-installer`, `scripts/package-release`, `scripts/package-release.ps1`, `scripts/quality-gates.json`, `scripts/release-documents.txt`, `tests/concurrent_voyages.py`, `tests/voyage_fixture.py`; untracked `inspiration apps/`. Later status no longer showed the first three modifications: concurrent changes are occurring, so this is a source observation, not a frozen commit audit. Staged render change and deletions remained; final status additionally showed concurrent untracked `docs/audits/`. Read-only status/diff used the documented fallback `git --git-dir=.local-git/worktree.git --work-tree=.` because the wrapper is deleted. No restoration, staging or publication attempted. Issue #271 is user-supplied context; no remote issue inspection was performed under this local-only scope.

## Notation and entry flags

Every line in the inventory names a leaf; angle brackets denote required values, square brackets optional arguments, `...` repetition. These are shell usage templates, not literal commands with real UUIDs/digests. Group prefixes apply to every following line. Clap's generated `help` routes, `-h/--help`, and root `-V/--version` are implicit meta-interfaces, not additional application leaves. Shell choices for completions: `bash`, `elvish`, `fish`, `powershell`, `zsh`.

Root entry: `helm [GLOBAL] [COMMAND]`. No command defaults to chat; an attended full-screen launch enters Helm's multiplexer rather than an embedded agent. Global flags:

- `--config <PATH>`, repeatable `--set <KEY=VALUE>`, `--model <MODEL>`, `--workspace <PATH>`.
- `--provider <openai-responses|openai-chat|chatgpt-oauth|anthropic>`; `openai` and `openai-compatible` alias **openai-chat**, not Responses.
- `--access <read-only|approval|unrestricted>` conflicts with hidden compatibility `--approval <always|on-risk|never>`.
- `-v`/`--verbose[=true|false]` (false default, bare flag true; optional boolean requires `=`), `--log-format <text|json>` (text default). JSON logging is not a universal JSON result mode.
- [Policy selection](../../voyage/src/policy_profile/cli.rs): `--policy-directory <PATH>`, `--policy-profile <NAME> --policy-revision <N> --policy-digest <SHA256> [--policy-confirm <DIGEST>]`. Profile requires both revision and digest; revision/digest/confirm require profile. Confirm is exact fresh preview acknowledgement for a more permissive launch, not a generic bypass.

Syntactic global availability does not guarantee semantic acceptance: `connect` rejects `--config`, `--set`, `--model`, `--provider`, root `--workspace`, `--access`, `--approval`, and `--policy-profile`; execution configuration belongs to the connected host. Use nested `connect new --workspace ... --config-path ...`. Other early-dispatched administrative commands need not consume all root overrides. Remembered chat preferences can affect default presentation/configuration; explicit CLI presentation flags take precedence.

## Complete Helm leaf inventory

### Root leaves, local-provider and browser installation

```text
helm protect-connections --directory <PATH>
helm run <PROMPT>... [--resume <SESSION>] [--no-save]
helm chat [--resume <SESSION>] [--plain[=true|false]]
helm sessions
helm models [--json]
helm config
helm local-provider presets
helm local-provider probe <PRESET> [ENDPOINT_OPTIONS]
helm local-provider setup <PRESET> [ENDPOINT_OPTIONS] --output <NEW_PATH>
helm local-provider scan
helm completions <SHELL>
helm manpage
helm doctor
helm browser setup
helm browser status
```

PRESET = `ollama|lm-studio|vllm|llama-cpp|custom`. ENDPOINT_OPTIONS = `--endpoint <URL>`, `--model <ID>`, `--api-key-env <NAME>`, `--transport <chat|responses>` (chat default), `--chat-max-completion-tokens`, `--timeout-secs <1..120>` (15 default). Custom requires an explicit endpoint at validation.

Sources: [browser](../../helm/src/process_client/browser/mod.rs), [local-provider](../../voyage/src/local_provider.rs), [diagnostics](../../helm/src/diagnostics.rs), [frontend](../../helm/src/process_client/frontend.rs).

- `run`/plain `chat` create or resume Vessel-supervised independent voyages, not in-process execution. `--plain` forces line-oriented chat; lack of terminal also selects plain. `--no-save` still uses a durable owner while active and deletes only after observed cleanup; cleanup failure is retained/reported, not permission to delete uncertain work. Resume with no-save branches first to protect the source. Disconnect is not cancellation.
- `sessions` is local catalogue discovery, not remote account-wide enumeration. `models` is provider/account discovery and can use network; it is not equivalent to no-provider `doctor`. `config` conceals secret bindings. Doctor inspects configuration/local dependencies without dispatching a model; neither doctor nor source reading proves runtime success.
- `local-provider presets` is offline. Scan explicitly GETs /models on four fixed numeric-loopback ports without credentials. Probe and setup validate discovery **and may generate up to 16 output tokens** in a tool-free request; they require provider/budget authorization, not just permission for a network read. Setup creates a new config, never overwrites; there is no inspect/configure or confirm leaf. HTTPS or numeric-loopback HTTP is required, without URL credentials/query/fragment. API key input is an environment name; omission selects no auth.
- `protect-connections` operates on an existing private local `helm-connections` directory using an externally provisioned runtime key; it does not connect to a provider or share a browser.
- Browser setup installs the bundled lockfile-pinned Node dependency; status checks adapter/Chromium availability. Neither shares a browser. Actual sharing is a separate opt-in local companion, below.
- Completions/manpage are stdout generators. No executable help was run in this audit, so binary/source version parity is unverified.

### `helm connect` (optional UI entry, direct leaves and nested groups)

[Grammar/dispatch](../../helm/src/process_client/cli.rs), [local service](../../helm/src/process_client/local.rs).

Prefix: `helm connect [--directory <LOCAL_PRIVATE_PATH>] [--include-local] [--no-start] [--access-file <PRIVATE_PATH>]...`.

```text
helm connect                                      # no subcommand: multiplexer
helm connect browser <SESSION> [--reconcile]
helm connect artifact <SESSION> <ARTIFACT> <NEW_LOCAL_PATH>
helm connect export <SESSION> <NEW_LOCAL_PATH>
helm connect terminal <SESSION> --run <RUN> --terminal <TERMINAL>
helm connect run <SESSION> [--command-id <UUID>] <PROMPT>...
helm connect chat <SESSION>
helm connect list
helm connect new [--id <UUID>] [--command-id <UUID>] --workspace <HOST_PATH> [--config-path <HOST_PATH>]
helm connect import <SESSION> --command-id <UUID> --workspace <HOST_PATH> --source-directory <HOST_PATH> --expected-revision <N> --source-sha256 <SHA256> [--config-path <HOST_PATH>]
helm connect branch <SESSION> --branch-id <UUID> --command-id <UUID> --expected-revision <N> --expires-at-ms <MS> [--name <NAME>]
helm connect archive <SESSION> [--restore] --command-id <UUID> --expected-revision <N> --expires-at-ms <MS>
helm connect delete <SESSION> --confirm-session-id <SAME_UUID> --command-id <UUID> --expected-revision <N> --expires-at-ms <MS>
helm connect inspect <SESSION>
helm connect submit <SESSION> --command-id <UUID> --expected-revision <N> --expires-at-ms <MS> <PROMPT>...
helm connect cancel <SESSION> --run <UUID> --command-id <UUID> --expected-revision <N> --expires-at-ms <MS>
helm connect receipt <SESSION> <COMMAND_UUID>
helm connect stop <SESSION> --incarnation <UUID>
helm connect restart <SESSION> --incarnation <UUID> --command-id <UUID>
helm connect request <SESSION> <TYPED_PUBLIC_JSON>
```

Bare connect is a valid entry, not an enum leaf. Without access files the local route is used; remote credentials alone do not implicitly add local, unless `--include-local` requests it. Local service can auto-start unless `--no-start`; failure to discover with no-start is an explicit error. `--access-file` is a private credential file, not a bearer value on argv. No separate general-purpose SSH execution mode exists here.

`run` is the convenience submit-and-follow path; `submit` exposes exact identity/revision/expiry. Lost responses should be recovered with receipts/inspection, not fresh mutation UUIDs. `chat` is line-oriented; EOF/Ctrl+C detaches without cancelling. There are no direct connect history/attach/rename leaves: `request` takes a typed public operation, including history, steering or decisions. Stop targets an incarnation; restart requires previous observed cleanup. Delete purges canonical history with exact session confirmation and retains a tombstone. Cancel requests do not establish termination. Import is a fenced ordinary-session migration requiring UUID/revision/fingerprint; branch creates a new independent owner from idle history. Archive/restore requires idle state. Remote grants can deny otherwise valid grammar. Export pins public history to a revision and creates a new local Markdown file; artifacts come from the owning voyage and land locally, not at a host workspace path. Neither overwrites existing files.

Terminal is a separate private human-input channel requiring exact run and terminal identities; human input/capture must not enter model-visible history. Browser uses the same full-duplex Helm–Vessel socket, not a separately networked browser bridge; consent and captured local browser data remain distinct from Voyage policy. `--reconcile` submits retained positive cleanup evidence, never replays a browser effect.

#### `helm connect inbox` (10 leaves)

[Source](../../helm/src/process_client/inbox.rs). Inherits connection options above.

```text
helm connect inbox configure <JSON_FILE> --command-id <UUID>
helm connect inbox accept <DESTINATION> --command-id <UUID>
helm connect inbox destinations
helm connect inbox revoke <DESTINATION> --command-id <UUID>
helm connect inbox test <DESTINATION> --command-id <UUID>
helm connect inbox list <DESTINATION> [--after <N>] [--limit <1..100>]
helm connect inbox watch <DESTINATION> [--after <N>] [--seconds <1..3600>]
helm connect inbox inspect <DESTINATION> <EVENT>    # alias: open
helm connect inbox seen <DESTINATION> <EVENT>
helm connect inbox dismiss <DESTINATION> <EVENT>
```

After defaults 0; list limit 50; watch duration 60 seconds. Configure accepts typed bounded JSON and immutable destinations; recipient acceptance is separate from source consent. Inbox is metadata-only, not prompt/tool/approval content. Read does not mark seen. Inspect/open refreshes metadata, not TUI navigation or an approval response. Dismiss does not cancel a run or resolve an owner request. Test is synthetic, never a fabricated successful run. Watch is bounded and stops on error. Revoked destination identity cannot be reused.

#### `helm connect admin` (7 leaves)

[Arguments](../../helm/src/process_client/admin/args.rs), [dispatch](../../helm/src/process_client/admin.rs), [move](../../helm/src/process_client/admin/transfer.rs).

```text
helm connect admin identity
helm connect admin trust <IDENTITY_JSON>
helm connect admin move <SESSION> --incarnation <UUID> --expected-revision <N> --destination-directory <LOCAL_VESSEL_PATH> --workspace <DESTINATION_HOST_PATH> [--config-path <DESTINATION_HOST_PATH>] --journal <PRIVATE_LOCAL_PATH>
helm connect admin accept-participant <BINDING_JSON> --command-id <UUID>
helm connect admin remove-participant <BINDING> --command-id <UUID> --expected-revision <N> [--cancel]
helm connect admin assignment <SESSION> --run <UUID> --assignment <UUID> --participant <ROUTE> [--cancel]
helm connect admin recover <SESSION> --incarnation <UUID> --command-id <UUID> [--acknowledge-cleanup <RUN>] [--reconcile-tools <RUN>] [--expected-revision <N>] [--acknowledge-resource <UUID>]...
```

These are explicit account-authorized operations, not powers conferred by ordinary observation grants. Independently exchange and verify public identities before trust. Move uses a second **local Vessel directory**, pinned signatures and a durable courier journal: do not market this CLI as arbitrary remote URL migration. Repeat the exact journal-backed operation to resume; destination configuration/workspace is destination-owned and credentials are not copied. Participants are subordinate bindings, not competing canonical owners. Removal distinguishes drain/cancel. Recovery requires observed exact owner/resource identities and OS fencing; attestation is a human claim, not measured cleanup, and cannot safely paper over uncertain live effects.

### `helm managed` (6 leaves)

[Grammar](../../helm/src/managed.rs), [commands](../../helm/src/managed/commands.rs), [connection](../../helm/src/managed/connection.rs).

Prefix: `helm managed --directory <ABSOLUTE_PRIVATE_INSTALLATION_PATH> [--json]`.

```text
helm managed --directory <PATH> create [--id <UUID>] [--name <NAME>]
helm managed --directory <PATH> list [--after <UUID>] [--limit <N>]
helm managed --directory <PATH> submit <SESSION> --expected-revision <N> [--command-id <UUID> --expires-at-ms <MS>] <PROMPT>...
helm managed --directory <PATH> cancel <SESSION> --run <UUID>
helm managed --directory <PATH> recover <SESSION> [--acknowledge-cleanup <UUID>] [--acknowledge-resource <UUID>]... [--reconcile-tools <UUID> --expected-revision <N>]
helm managed --directory <PATH> upgrade
```

List default limit 20, metadata-only. Create is once-only: recover a lost response by list, not implicit retry. Explicit submit command ID/expiry require each other; reconcile-tools/revision also require each other. Private SQLite journal remains authoritative; create/submit use independent voyage owners. Unavailable owner errors direct to explicit recovery, not replay. Alternate directories have distinct local identities. Upgrade requires a quiescent journal and stopping older writers; it is not a transparent repair. Cancel requested is not stopped. JSON is a group flag, not a root-wide result mode.

### `helm github` (17 regular + 7 admin leaves)

[Operator args](../../voyage/src/github/operator.rs), [frontend host boundary](../../helm/src/process_client/frontend/github.rs), [admin](../../voyage/src/github/admin.rs).

Prefix: `helm github [--session <LOCAL_SESSION>]`. Regular operations require `--session` at dispatch even though clap makes it optional; only admin permits omission for orphaned local recovery.

```text
helm github auth
helm github logs <URL> <JOB_ID>
helm github remotes
helm github view <URL> [--section <SECTION>] [--page <N>] [--after <CURSOR>] [--resource <N>] [--thread <ID>] [--head <SHA>]
helm github continue <EXACT_JSON_REQUEST>
helm github reference <URL>
helm github references
helm github unreference <URL>
helm github feedback <URL> [--section <SECTION>] [--page <N>] [--after <CURSOR>] [--resource <N>] [--thread <ID>] [--head <SHA>] [--id <N>]
helm github prepare [<URL>] [--body <TEXT> | --body-file <PATH> | --draft-file <PATH>] [--head <SHA>] [--base <SHA>]
helm github publish <UUID> <DIGEST>
helm github inspect <UUID>
helm github list [--offset <N>]
helm github cancel <UUID> <DIGEST>
helm github forget <UUID> <DIGEST>
helm github reconcile <UUID> <DIGEST> <REMOTE_ID>
helm github dispose <UUID> <DIGEST> <NOTE>
helm github admin list [--offset <N>]
helm github admin inspect <UUID>
helm github admin cancel <UUID> <DIGEST>
helm github admin dispose <UUID> <DIGEST> <NOTE>
helm github admin forget <UUID> <DIGEST>
helm github admin audit
helm github admin clear-audit <DIGEST>
```

This group has 17 regular leaves and 7 admin leaves (24 total). SECTION = `details` (default), `comments`, `reviews`, `files`, `threads`, `thread-comments`, `review-comments`, `statuses`, `checks`, `suites`, `suite-checks`, `annotations`, `runs`, `jobs`; page defaults 1, list offsets 0. Nested sections validate required resource/thread/head identities. Continuation is exact returned JSON, not a URL guessing loop. Prepare's three body sources conflict pairwise.

Auth shows the explicitly delegated GitHub identity, not a token. Network reads are bounded untrusted observations. Remotes does not choose a fork silently. References/feedback modify local voyage context/work, not GitHub. Publishing is prepare → inspect/review exact bytes and digest → attended publish; retained operation identities distinguish cancel, forget, reconcile and disposition after uncertainty. No automatic replay of uncertain external writes. Admin is attended **local-only**, with write operations denied in read-only mode; audit can contain private evidence. Clearing audit requires the exported snapshot digest. Do not confuse this `auth` with provider login: it is nested GitHub delegated-account inspection, not `helm auth`.

### `helm extension` (16 leaves)

[Source](../../voyage/src/extensions/cli.rs). Prefix `helm extension [--scope <user|project>]` (user default; group-global).

```text
helm extension pack <DIRECTORY> <OUTPUT>
helm extension install <SOURCE>
helm extension update <ID> <SOURCE> --expected <DIGEST>
helm extension fetch <INDEX_PATH> <ID> [--expected <DIGEST>]
helm extension list
helm extension grants
helm extension revoke-grant <BINDING> --expected <DIGEST>
helm extension inspect <ID>
helm extension resource <ID> <PATH>
helm extension enable <ID> --expected <DIGEST>
helm extension review-executable <ID> --expected <DIGEST> --capability <CAPABILITY>...
helm extension execution-grants
helm extension execution-status <BINDING>
helm extension revoke-execution <BINDING> --expected <DIGEST>
helm extension disable <ID> --expected <DIGEST>
helm extension remove <ID> --expected <DIGEST>
```

Pack validates and creates a bounded archive. Install/fetch are inactive; update clears activation first. Fetch is explicit digest-pinned HTTPS index access. Enable grants exact declarative bytes as untrusted context, not execution authority. Executable review is separate, requires one or more explicit capabilities and does not start a process. Runtime policy still applies to execution. Orphaned grants remain inspectable/revocable independently of missing source packages. Revocation does not assert that active execution stopped. Resource prints a JSON string, avoiding raw terminal controls. Scope is local user/repository storage, not a remote Vessel account selector.

### `helm inference` (4 leaves)

[Source](../../voyage/src/inference/cli.rs), [history](../../voyage/src/inference/history.rs).

```text
helm inference inspect [--session <UUID>] [--after <N>] [--limit <N>]
helm inference history [--session <UUID>] --from <UTC> --until <UTC> [--group-by <session|model|agent|purpose|day>] [--group <EXACT_JSON_KEY>] [--offset <N>] [--limit <N>] [--snapshot <TOKEN>]
helm inference audit [--session <UUID>] [--after <N>] [--limit <N>]
helm inference configure [--session <UUID>] --operation <UUID> --expected-revision <N> (--limit <N> | --unlimited) [--warning <N>] --reason <TEXT> [--confirm]
```

Inspect/audit default after 0/limit 100. History defaults group model, offset 0, limit 50; nonzero offset requires the preceding snapshot, and group drill-down uses exact returned JSON keys. Session must already belong to this local project; omission selects project scope. Configure previews until repeated exactly with confirm, requires reason and immutable operation ID; limit is cumulative attempts since binding, not extra credits. Provider token fields can be null; historical sums are not billing totals. Local ledger operations are not provider budget authorization and cannot broaden execution policy. Read-style commands may initialize local ledger storage; they were not executed for this audit.

### `helm policy` (9 regular + 8 defaults leaves)

[Profiles](../../voyage/src/policy_profile/cli.rs), [defaults](../../voyage/src/policy_profile/defaults/cli.rs). Uses root policy-directory selection.

```text
helm policy list [--after <NAME>] [--limit <N>]
helm policy inspect <NAME>
helm policy create <NAME> [--preset <NAME>] [--expected-revision <N>] [--operation <UUID>]
helm policy edit <NAME> --input <PATH> --expected-revision <N> [--operation <UUID>]
helm policy duplicate <SOURCE> <NAME> [--expected-revision <N>] [--operation <UUID>]
helm policy delete <NAME> --expected-revision <N> [--operation <UUID>]
helm policy import <NAME> --input <PATH> [--expected-revision <N>] [--operation <UUID>]
helm policy export <NAME>
helm policy preview <NAME> --revision <N> --digest <DIGEST>
helm policy defaults init --directory <PATH> [--store-id <UUID>]
helm policy defaults enable --directory <PATH> --store-id <UUID> --output <NEW_CONFIG_PATH>
helm policy defaults list [--after <NAME>] [--limit <N>]
helm policy defaults inspect (--global | --project)
helm policy defaults set (--global | --project) --profile-directory <PATH> <NAME> --revision <N> --digest <DIGEST> --expected-revision <N> [--operation <UUID>]
helm policy defaults clear (--global | --project) --expected-revision <N> [--operation <UUID>]
helm policy defaults preview
helm policy defaults activate --expected-revision <N> [--operation <UUID>] --confirm <DIGEST>
```

Defaults init creates inert preferences; enable copies source config to a new output with the explicit store anchor. Other defaults commands use that configured anchor. List limit 100; create preset balanced; create/duplicate/import expected revision defaults 0. Other revisions are required. Defaults inspect/set/clear require exactly one of --global or --project; project uses this invocation’s root --workspace. Creating/importing a profile is inert; source revision does not confer authority. Export is rules/name/revision, not Config/secrets. Preview resolves actual Config/ceiling and emits digest; more-permissive selection requires exact confirmation. Defaults are private persistent preferences, separately previewed and activated, not a session and not silently inherited session authority.

### `helm workflow` (5 leaves)

[Args](../../voyage/src/workflow.rs), [execution](../../helm/src/process_client/frontend/workflow.rs).

Prefix `helm workflow [--user-directory <PATH>] [--json]`; both group-global.

```text
helm workflow list
helm workflow inspect <ID> [--scope <user|repository>]
helm workflow validate <ID> [--scope <user|repository>]
helm workflow preview <ID> [--scope <user|repository>] [INPUT_OPTIONS]
helm workflow run <ID> [--scope <user|repository>] [INPUT_OPTIONS]
```

INPUT_OPTIONS = repeatable `--input <NAME=VALUE>`, repeatable `--secret-env <NAME=ENVIRONMENT_VARIABLE>`, `--trust-repository <SHA256>`, `--no-save`, `--prompt-missing`, `--input-timeout-seconds <1..300>` (120 default).

List → inspect/validate → preview exact definition and rendered nonsecret inputs → run. Repository trust is pinned to exact digest; declarative recommendations do not grant authority. Missing-input prompting is attended and deadline-bounded, not an indefinite unattended wait. Never put secret values in `--input`, argv, files intended as nonsecret workflows, or chat. Secret-env takes names; preview validates without reading values. On execution only explicitly opting-in one-shot shell calls receive current-run bindings, with their output suppressed. Run goes through an independent supervised voyage. No-save workflows still require observed resource cleanup; failed/pending effects are not erased.

### `helm onboard` (3 leaves)

[Source](../../helm/src/onboarding/mod.rs).

```text
helm onboard inspect [--json]
helm onboard preview [--against <DRAFT> | --output <NEW_PATH> [--confirm]]
helm onboard accept --draft <PATH> --sha256 <SHA256> --output <NEW_PATH>
```

Inspect reads bounded repository evidence without command execution/provider loading. Candidate commands are suggestions, not verified results. Preview prints a draft/diff or explicitly confirmed new draft file; against conflicts with output, confirm requires output. Accept publishes exact reviewed UTF-8 bytes matching SHA256 to a new file, preserving existing guidance. Root workspace selects local repository. Generated project guidance is optional, never a prerequisite to a voyage.

## Vessel and Voyage executable boundaries

These are separate binaries, not missing Helm subcommands. [Vessel grammar](../../vessel/src/main.rs), [authentication](../../vessel/src/auth.rs), [grants](../../vessel/src/process/grant_cli.rs), [pairing](../../vessel/src/process/pair_cli.rs).

Vessel has optional subcommand: no command serves the configured management plane. Root options: `--bind <ADDRESS>` (127.0.0.1:9480), `--database <PATH>` (vessel.db), `--process-directory <PATH>` and `--public-origin <HTTPS_ORIGIN>` require each other, `--allow-insecure-loopback` requires public-origin, `--operator-token <SECRET>` or `VESSEL_OPERATOR_TOKEN` (prefer the environment/private provisioning to argv exposure), global `--log-format <text|json>` (text default), help/version. User-visible leaves:

```text
vessel [ROOT_OPTIONS]                         # no command: serve
vessel protect-connections --directory <PATH>
vessel auth status
vessel auth login [--device]
vessel auth logout
vessel auth import-codex [--path <PATH>] [--force]
vessel connection-audit --directory <PATH> [--limit <N>] [--cursor <TOKEN>]
vessel auth accounts list
vessel auth accounts connections
vessel auth accounts connect --label <TEXT> --endpoint <URL> [--transports <CSV>]
vessel auth accounts add --connection <UUID> --account <ALIAS> [--env <NAME>]
vessel auth accounts login --connection <UUID> --account <ALIAS>
vessel auth accounts status --account <UUID>
vessel auth accounts rename --account <UUID> --alias <ALIAS> --label <TEXT>
vessel auth accounts logout --account <UUID> [--remove]
vessel auth accounts import --connection <UUID> --account <ALIAS> --path <PATH>
vessel auth accounts rotate-api --account <UUID> --generation <N> [--env <NAME>] [--attest-same-identity]
vessel auth accounts reauthenticate --account <UUID> --generation <N> --path <PATH> [--replace-identity]
vessel auth accounts migrate-legacy [--old-writers-stopped]
vessel auth accounts enrollment-status --enrollment <UUID>
vessel auth accounts cancel-enrollment --enrollment <UUID>
vessel process-grant --directory <PATH> --output <PRIVATE_NEW_PATH> [--resume] [--session <UUID>] [--principal <UUID>] [--workspace <HOST_PATH>] [--endpoint <HTTPS_URL>] [--rights <CSV>] [--accounts <CSV_UUIDS>] [--enrollment-connections <CSV_UUIDS>] [--ttl-seconds <N>]
vessel pair-invite --directory <PATH> --endpoint <URL> --principal <UUID> --workspace <HOST_PATH>... --output <PRIVATE_NEW_PATH> [--rights <CSV>] [--accounts <CSV_UUIDS>] [--enrollment-connections <CSV_UUIDS>] [--ttl-seconds <N>]
vessel list-connections --directory <PATH>
vessel revoke-connection --directory <PATH> --grant <UUID> --expected-revision <N> --command-id <UUID>
vessel process-revoke --directory <PATH> --grant <UUID> --expected-revision <N> --command-id <UUID>
vessel local-serve --directory <PATH> [--voyage-binary <PATH>] [--capacity <N>]
vessel completions <SHELL>
vessel manpage
```

- `process-grant`, `pair-invite`, `list-connections`, `revoke-connection`, `process-revoke` `protect-connections`, and `connection-audit` are Linux-conditional grammar. Do not promise their availability on macOS/Windows. Local private Vessel checks explicitly reject non-Unix; that is not evidence of complete native macOS support either.
- Process-grant requires session/principal/workspace/endpoint unless resume; resume reloads an immutable retained request rather than minting a fresh uncertain grant. Rights default observe, TTL 86400. Pair-invite requires at least one workspace, binds intended principal, defaults rights `catalogue,create,observe,history,execute`, TTL 600; terminals/destructive rights are not default. Private credential outputs are files, never stdout. Human workspace connection grants, scoped voyage grants, participant bindings and provider accounts are different authority surfaces.
- Connection-audit defaults limit 64, requires same limit with returned cursor, reads existing private state without repair. There is no hash-token CLI leaf. Legacy auth status/login/logout/import-codex coexist with named accounts; import-codex imports credentials (default ~/.codex/auth.json), not a Codex execution bridge. --force replaces existing local ChatGPT credentials.
- Account connection transports are comma-separated explicit names `openai-responses,openai-chat,chatgpt-oauth,anthropic`. Credentials live on the execution host. API add/rotate use environment names or private local input, not key arguments; login/enrollment may contact a provider, and import/reauthentication consume host-local files. Same-identity attestation is not provider verification of billing identity; replacement invalidates admitted bindings. Migration requires all old writers stopped: the switch is an attestation, not a fence against old binaries. Logout/removal and cancellation of enrollment are explicit state changes.
- `local-serve --capacity` is hidden **deprecated compatibility**: voyage count is no longer capped by it. Local-serve supervises independent voyage processes over authenticated loopback duplex sockets/compatibility HTTP. It is not an embedded agent executor. The no-subcommand root entry is the configured public service boundary, not a command to silently start as part of this audit.

[Voyage executable grammar](../../voyage/src/main.rs), [resource maintenance](../../voyage/src/host_resources/cli.rs):

```text
voyage discover-models
voyage serve --directory <PATH> --registration <JSON_PATH> --endpoint <ENDPOINT_PATH>
voyage supervise --directory <PATH> --registration <JSON_PATH> --endpoint <ENDPOINT_PATH>
voyage observe-suspended --directory <PATH>
voyage recover --directory <PATH>
voyage legacy-recover --directory <PATH> --session <UUID> [--acknowledge-cleanup <UUID>] [--acknowledge-resource <UUID>]... [--reconcile-tools <UUID> --expected-revision <N>]
voyage upgrade-journal --directory <PATH>
voyage validate-start --workspace <PATH> [--config <PATH>]
voyage import-plan --source-directory <PATH> --session <UUID>
voyage host-resources inspect
voyage host-resources attest <RESERVATION_UUID> --confirm <SAME_UUID> --reason <TEXT>
voyage completions <SHELL>
voyage manpage
```

Voyage requires a command and supports help/version, **not** a root log-format option. Serve/supervise are registration-file-based supervisor interfaces, not user chat. Supervise guards descendant cleanup on Linux; its grammar alone is not native portability evidence. Discover-models reads bounded discovery JSON on stdin, with no session creation. Observe-suspended/recover use framed stdin/stdout protocol: they are not shell-interactive recovery wizards. Suspended observation is bounded (25 seconds), fails closed on malformed framing/timeout, and does not start execution. Recover holds an exclusive fence. Legacy-recover never replays effects; reconciliation and revision require each other. Upgrade requires an existing absolute quiescent journal. Validate-start resolves host config/workspace without model dispatch. Import-plan checks private source identity/fingerprint rather than silently creating a new session. Host-resource attest requires matching confirmation and independently inspected dead-owner descendants; it never proves cleanup by itself. Sources: [server args](../../voyage/src/server.rs), [suspended](../../voyage/src/server/suspended.rs), [recovery](../../voyage/src/server/recovery.rs), [legacy recovery](../../voyage/src/server/legacy_recovery.rs), [bootstrap](../../voyage/src/server/bootstrap.rs).

## UX/failure/privacy findings for parent review

1. **Grammar vs usable workflow:** root `--model/--workspace/--access` look globally available but connect rejects them; regular GitHub `--session` is runtime-required despite optional clap syntax. Help should clearly distinguish these boundaries. Nested flags with the same spelling may be host-specific (`connect new --workspace`) or local (`onboard` root workspace).
2. **Inspection is not uniformly side-effect-free:** config/doctor/browser status are not model dispatch; models/local-provider scan/GitHub view are network reads; local-provider probe/setup may generate model output; some local inspection opens/initializes private ledgers; connect may start a service. A read-only audit must inspect source, not indiscriminately execute every “list/status/inspect” command.
3. **Attended vs unattended:** exact publication/policy confirmation and GitHub admin are not unattended bypasses; missing workflow input is bounded; private terminal/browser sharing requires local human consent. Remote route rights cannot override execution-host roots, policy or provider-account bindings.
4. **Failure identity is part of the UX:** revisions, digests, command IDs, expiry, incarnation, pagination snapshots and courier journals are not interchangeable opaque tokens. Lost/uncertain responses must not trigger fresh identities or effect replay. Cancel/revoke requested, dead process, and observed cleanup are distinct states.
5. **Unavailable is not deprecated:** missing/unsafe private directories, absent browser prerequisites, unreachable Vessel, unsupported host platform, insufficient route rights or provider account are operational refusals. Hidden `--approval` is compatibility syntax; provider `openai` aliases legacy Chat Completions; Vessel capacity is explicitly deprecated. Outbound worker mode, Codex bridge, `helm auth` provider management, `helm voyages`, and a general Helm/browser executor are **not current CLI leaves** merely because older docs, reserved workflow words, or TUI concepts mention them.
6. **Do not conflate public with harmless:** exported public history/artifacts can still contain user material; audit JSON and draft bodies may be private. Avoid shell history for secrets, never expose access-file contents, preserve new-file/no-overwrite behavior, and keep direct human terminal input out of model-visible output.
7. **Evidence limits:** all behavior statements are grounded in current source/docs, not exercised journeys. No Linux runtime test or native macOS/Windows proof, provider/budget validation, binary help comparison or GitHub issue acceptance review is claimed. Parent can use this inventory for subsequent explicitly authorized CLI journeys separately from TUI review.

## Local report validation

Checked every report relative source link exists (no missing paths), Markdown code fences are balanced, and Git identifies this report as ignored. Compared enum variants and nested argument definitions with the inventory; no executable help or runtime behavior was exercised. Final Git status retained the unrelated staged edit/deletions/untracked work described above; this audit made no tracked edits.
