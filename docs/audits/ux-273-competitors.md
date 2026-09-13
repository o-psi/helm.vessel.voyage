# #273 pinned competitor journeys: bounded prerequisite evidence

Measured 2026-09-13 UTC on Linux 6.18.50-2-lts, x86_64, glibc 2.44.
Scope: [#273](https://github.com/o-psi/voyage/issues/273), following the
[corrected comparative review](ux-comparative-review.md) and
[tutorial benchmark](ux-tutorial-benchmark.md). This is a prerequisite investigation,
**not a completed interactive UX comparison**. No competitor reached an interactive
first screen in this attempt; zero first-result/composer/discovery/return journeys
were completed. No `--help` output is counted as usability evidence.

## Pins and boundaries

| Product | Supplied source pin | Runtime/build actually used |
|---|---|---|
| Pi | `71dca871bc80b6bc97be37f0ca3189399d651fff` | Node 26.8.2, npm 11.19.1; workspace packages 0.85.1; copied-source locked install and offline build |
| OpenCode | `95daf90670b7c039c436c85537da5fbfe2205b41` | Node 26.8.2 attempted the pinned `packages/opencode/bin/opencode`; source package 1.18.30 declares Bun 1.3.14 at repository root, absent here |
| Codex | `36f0dbe796d9bb1a18a0fc0640ed08b3e1d54564` | Node 26.8.2 attempted the pinned `codex-cli/bin/codex.js`; wrapper package 0.0.0-dev; native executable unavailable |

Pins come from the supplied archive metadata, not new remote provenance verification.
Installed host commands/shims were not substituted for the pins. No Rust/Cargo
build, global install, paid authentication, or public provider request was run.
No account prerequisite was tested; **these are not authentication blockers**.

All writes for experiments are in ignored `target/ux273/competitors/`; the only
new tracked document is this file. Pi was copied from `target/ux-audit/pi/source`
with `cp -a`; originals were not edited. After installation/build, all 1,713 original
regular non-symlink files still matched their copied counterparts byte-for-byte.
The lockfile SHA-256 in both is
`b9ab7cb9d02e818ba16dd0874285175f3b2cad117326761de0e2c61f285a5e2b`.
This verifies copy/source equality, not upstream authenticity.

Commands used `env -i` and separate fresh HOME directories; no inherited provider
keys, proxies, agent configuration or credential environment. The PATH selected
the real Node installation, not credential-bearing host shims. Build and launch
attempts used `unshare -Urn`: an independent network namespace with no routes and
only a DOWN loopback interface, confirmed separately by `/proc/net/route`,
`/proc/net/dev`, and `ip link show`. No synthetic provider was started because
no complete client became available. Dependency acquisition alone had network
access: locked npm installation with lifecycle scripts disabled and audit/funding
requests disabled. This is not a fully air-gapped dependency installation.

## Executed evidence and precise stopping points

### Pi: installation works; offline source build needs an absent catalogue

Inspected root/workspace package scripts, the coding-agent esbuild bundler,
SQLite migration copier, and model-data validation/generation paths before use.
The root `build:offline` invokes TypeScript compilation, validation, asset copies,
and bundling; unlike the ordinary build it does not regenerate provider catalogues.

1. Initial npm invocation failed before installation because user and global npm
   config both pointed at `/dev/null`: `double-loading config`. This was an audit
   setup mistake, not Pi UX. Corrected using two distinct empty config files.
2. `npm ci --ignore-scripts --no-audit --no-fund` exited **0**, installing 312 packages
   in six seconds. Lockfile unchanged; native install hooks were not executed.
3. `npm run build:offline` under the offline namespace exited **1**. Chord, TUI and
   telemetry compilation steps completed; the AI package's `check:model-data`
   stopped on missing `packages/ai/src/providers/data/amazon-bedrock.json`.
   The whole `providers/data` directory is absent in the supplied source and is
   explicitly Git-ignored. The coding-agent build/launch was not reached.

The diagnostic suggests `npm run hydrate:model-data`. Inspection shows that this
runs `generate-models.ts --strict --data-only`, whose main generation path loads
models.dev and fetches OpenRouter and Vercel AI Gateway catalogues; it also contains
NVIDIA catalogue fetching. Running it is incompatible with this task's no-public-
provider-request constraint. No hydration was attempted, no catalogue was fabricated,
and the validator was not bypassed. A synthetic completion endpoint alone cannot
supply the absent build-time catalogue. The exact prerequisite is a compatible
validated catalogue for this pinned source, supplied without prohibited requests.
Subsequent build/runtime prerequisites remain unverified, including native dependencies
whose install hooks were intentionally skipped. The successful TUI compilation does
**not** establish interactive composer behavior.

### OpenCode: source runtime absent; Node wrapper fails before UI

The pinned source package's `dev` script is `bun run ./src/index.ts`; its build
also uses Bun. Bun is not installed. Direct Node execution of the supplied
extensionless `bin/opencode` exited **1** at line 3:
`ReferenceError: require is not defined in ES module scope`.
The enclosing package declares `type: module`; this is a source-tree/Node launch
context result, not an observed packaged OpenCode onboarding problem.

The inspected wrapper otherwise searches for a cached/native platform executable
or a platform package. No pinned bundled executable was supplied at that location.
No speculative renaming, module-loader shim, unpinned system binary, or Bun install
was tried. Required next prerequisite: the declared Bun runtime and locked local
build dependencies, or a demonstrably matching native build of this exact pin.
No assertion is made that satisfying only the first prerequisite completes launch.

### Codex: JavaScript wrapper present, pinned native executable absent

Direct Node execution of `codex-cli/bin/codex.js` exited **1** at its explicit
missing-dependency check:
`Missing optional dependency @openai/codex-linux-x64`.
Inspection confirms it resolves a platform package or its local `vendor` fallback,
then needs the native `x86_64-unknown-linux-musl/bin/codex` executable. The supplied
wrapper tree lacks its vendor/node_modules payload. The error's suggestion to install
`@latest` globally was **not** followed: it would lose both pin and scope.
A native artifact tied to this commit is required. Compiling Rust Codex was explicitly
out of scope to avoid competing with the parent's Cargo work. No Codex TUI appeared.

## Reproduction and retained local evidence

Let `B=/home/psi/voyage/target/ux273/competitors`,
`N=/home/psi/.local/share/mise/installs/node/26.8.2/bin`, and
`A=/home/psi/voyage/target/ux-audit`. These are exact execution locations, not portable
installation instructions. The successful install was run in `$B/pi-source`:

```sh
env -i HOME="$B/install-home" PATH="$N:/usr/bin:/bin" \
  npm_config_cache="$B/npm-cache" \
  npm_config_userconfig="$B/install-home/user.npmrc" \
  npm_config_globalconfig="$B/install-home/global.npmrc" \
  timeout 240 npm ci --ignore-scripts --no-audit --no-fund

env -i HOME="$B/install-home" PATH="$N:/usr/bin:/bin" \
  unshare -Urn timeout 240 npm run build:offline
```

Both npm config files were created empty. Launch probes used no CLI arguments,
from the parent checkout, with separate empty `codex-home` and `opencode-home`:

```sh
env -i HOME="$B/codex-home" PATH="$N:/usr/bin:/bin" TERM=xterm-256color \
  unshare -Urn node "$A/codex/openai-codex-36f0dbe/codex-cli/bin/codex.js"
env -i HOME="$B/opencode-home" PATH="$N:/usr/bin:/bin" TERM=xterm-256color \
  unshare -Urn node "$A/opencode/source/packages/opencode/bin/opencode"
```

Their output was redirected, not captured as interactive PTY screens; neither got
far enough for terminal geometry to matter. Pi install/build ran in managed PTYs
with output redirected. All three managed processes were observed exited. The
printed inner command statuses (1, 0, 1) are recorded separately from the shell's
successful final `echo`; shell success is not build success.

Local retained artifacts under `$B`:

- `pi-install.log` (initial setup failure), `pi-install-2.log` (successful install),
  `pi-build.log` (offline catalogue failure).
- `codex-launch.log`, `opencode-launch.log` (full pre-UI failure diagnostics).
- `network-namespace.log` (offline namespace inspection).
- `verify.py`, `evidence.json` (source-copy comparison, lock/log SHA-256s, exact
  versions, pin metadata, observed command outcomes).
- `pi-source/`, isolated homes and dependency cache, retained for a later approved
  attempt rather than deleted or misrepresented as complete runnable installations.

## Comparative conclusions and corrections

| Intended journey | New hands-on evidence | What remains unverified |
|---|---|---|
| First launch → useful result | Prerequisite attempts above; no interactive screen | Onboarding, first prompt, response, model/account guidance for all three |
| Compose → discover → cancel → recover draft | None; source component compilation is not the application journey | Focus, discoverability, draft retention and recovery |
| Finish → leave → return to same conversation | None | Naming/selection, persisted context, and return-task continuity |

The corrected review's central restriction still applies: source controls and
handler reachability are not a comparative usability verdict. This attempt adds
concrete, bounded prerequisite evidence, **not** perceived clarity, latency, friction,
or a winner. In particular:

- Pi's offline build name does not mean a fresh source archive is self-contained.
  That is an audit reproducibility finding, not evidence that normal packaged Pi
  users experience this build failure.
- OpenCode's failed Node wrapper is not evidence against its Bun development path
  or packaged TUI. Codex's missing native payload is not evidence against its
  composer. Launcher failures must not be ranked as interactive UX shortcomings.
- The tutorial benchmark's quickstart and return-path recommendations remain
  source/document-derived hypotheses. No successful tutorial-to-result execution
  was established here, nor were its proposed discovery/cancel journeys measured.
- No Helm advantage follows from having more controls or from competitors being
  unavailable in this audit environment. Helm's own focused tests are a different
  evidence tier and do not substitute for a matched four-product journey.

Next meaningful comparison requires those exact pinned prerequisites, then actual
terminal screen evidence for the same intent: start, submit a harmless synthetic
prompt, discover a control, cancel back to a retained draft, save a result, exit,
and resume the same conversation. A loopback-only synthetic provider can then test
interface transitions and persistence, not model quality or real authentication.
This report leaves the competitor interactive acceptance in #273 **unfinished**;
it does not close the broader issue or invent an auth/budget obstacle. The parent
owns integration/publication; this narrowly scoped worker does not stage concurrent
changes or mutate the issue.
