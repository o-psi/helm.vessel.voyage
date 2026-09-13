# Tutorial benchmark: make Helm as simple to start and use

Tracked in [#272](https://github.com/o-psi/voyage/issues/272). Official tutorials
retrieved 2026-09-13 UTC. This supplements, rather than repeats, the
[source-level comparative review](ux-comparative-review.md).

## Verdict

**Helm does not yet demonstrate onboarding simplicity parity in its documentation.**
The competing tutorials teach a first useful task; our front door teaches an
architecture and source build, then sends the reader elsewhere to assemble provider
setup. That is a concrete documentation/journey gap, not proof that every competitor
runtime is easier or faster.

The benchmark is not fewer slash commands. It is whether a newcomer can go from
“I want help with this project” to a successful task, then inspect and resume it,
without learning the harness's internal administration model.

**We read the official taught paths, including provider authentication pages. We
did not install these products, sign in, enter a key, purchase credit or run a model.**
Official tutorials describe intended behavior; they are not independent evidence of
successful authentication, billing entitlement or runtime usability. Live website
content can differ from the pinned development code in the earlier audit.

## Comparable starting conditions

Compare separately:

1. A local supported machine, project directory and **existing eligible ChatGPT
   account**. Account creation/purchase is outside the happy path but must be
   disclosed if needed. Compare native Codex login, supported Pi/OpenCode login,
   and Helm's native ChatGPT connection—not unrelated providers or billing plans.
2. A local supported machine/project and **existing OpenAI API key**. Compare
   API-key setup without treating a ChatGPT subscription as API credit.
3. No provider account: include external signup, billing and key acquisition.
   OpenCode's recommended Zen path belongs here when the user has no Zen account;
   it is not a fair shortest-path baseline against an already authenticated Codex.
4. Remote/headless, multiple accounts and organization-managed environments:
   separate advanced journeys. Their real authority/credential steps must not be
   silently removed, but need not dominate local first use.

Do not assign exact click counts from prose: selectors, auth state, trust prompts,
OS installer tabs and browser consent vary. Count required **user tasks and concept
switches** first, then measure actual interactions on a controlled setup.

## The front-door lesson each product teaches

| Product / official entry | Taught path and user-facing concepts | What the reader has to assemble | Implication for Helm |
|---|---|---|---|
| **Pi quick start** [T1,T2] | Install with npm or installer; API-key environment setup + `pi`, **or** `pi` → `/login` → choose provider; “Then just talk to pi.” Provider details and model selection are secondary. | Node/npm or installer prerequisites; actual key/account; browser authorization varies by provider. The API example uses Anthropic, so substitute the documented OpenAI env binding for a same-provider comparison. | Lead with one recommended local path and two clear authentication choices. Do not require understanding agent/runtime architecture before the first prompt. |
| **OpenCode introduction** [T3,T4] | Install; `/connect` → provider. Recommended new-provider example chooses OpenCode Zen, visits its auth site, signs in/adds billing/copies key, then pastes into the connection prompt. `cd` project → `opencode` → recommended `/init` → ask questions/change code. | The configure section mentions the TUI before the later launch example. Zen signup/billing is external work; existing OpenAI/ChatGPT paths differ. `/init` is a documented onboarding recommendation, not assumed necessary for every prompt. | Show launch before telling users to type an in-app command. Keep optional project guidance distinct from first-task readiness. Disclose external billing rather than hide it behind “connect.” |
| **Codex CLI quickstart** [T5,T6] | Three headings: Install; open project and run `codex`, first-run sign-in; describe first task (example: “Tell me about this project”). Links explain auth alternatives and Git checkpoints. | Existing eligible account or API billing; platform prerequisites/trust/config may still add work. Generic `/codex/quickstart` now redirects to desktop/web onboarding; the CLI-specific page is the relevant tutorial. | One obvious executable and readiness-driven sign-in. A first-task example is more useful on the entry page than a list of internal components. |
| **Helm current README/config/account/installer guides** [H1–H5] | Root README explains three programs/process ownership, gives workspace release build and `helm connect`, then says configure a provider elsewhere. Helm README emphasizes Vessel connections before provider onboarding. Installer/provider/account/default instructions are separate. | Find an install/build path, distinguish three executable names, find current account enrollment rather than legacy auth, select model/account and discover that a host default is mandatory. API setup includes connection/account metadata and private-host terminal input. | The reader must act as integrator. Provide one tested local quickstart that takes ownership of the complete setup-to-task journey. Preserve architecture in implementation, not as prerequisite vocabulary. |

### Installation is part of simplicity, not a footnote

Pi, OpenCode and Codex teach a distributable installation command prominently.
Helm has a bootstrap and reviewed installer, but the entry README starts with a
source build. The installer guide says published downloads require GitHub release
assets; `gh release list --repo o-psi/voyage --limit 3 --json tagName,isDraft,isPrerelease,publishedAt` returned **an empty list in this review**.
This observation is not a claim that local release artifacts are absent or that
GitHub could never publish a release. It means we cannot currently certify a
public-download happy path from the release catalogue we observed.

Current bootstrap supports Linux x86_64/aarch64 with a systemd user manager;
published download setup additionally requires curl and Python 3.11+. The source
build needs Rust/build prerequisites. Do not compare a competitor's published
installer with an assumed working Helm release, or present a hypothetical
curl-install command as tested. Actual release/install availability is a prerequisite
for a credible newcomer simplicity claim. [H5,H6]

## Same-provider onboarding benchmark

| Stage | Pi tutorial | OpenCode tutorial | Codex CLI tutorial | Helm current documented path / friction |
|---|---|---|---|---|
| **Existing ChatGPT login** | Launch `pi`, `/login`, select ChatGPT/Codex provider; provider page states eligible subscription requirement. | `/connect`, choose OpenAI, select ChatGPT subscription option, complete browser flow; use `/models` for model selection. | Launch `codex`; first-run sign-in choice → ChatGPT browser flow; return to first task. | Account → Sign in to another account → native ChatGPT connection → new alias → private code view → explicit O/open website → enter code → return → select enrolled account and review settings. Additional required host-default selection is separate. [H3] |
| **Existing OpenAI API key** | `OPENAI_API_KEY` binding (provider docs) and launch; `/login`/auth-file alternatives vary. No mandatory account UUID workflow taught. | `/connect` → OpenAI → manual API-key entry → `/models`. Private connection input is distinct from sending a chat message. | API-key sign-in option; auth page distinguishes usage billing and unattended key paths. | Executing-host `vessel auth accounts connections`; create connection if needed with endpoint/transports; `add --connection UUID --account ALIAS` prompts without echo in a private human terminal; select account/settings in Helm, explicitly set host default. Do not send keys through chat. [H3] |
| **Set model** | Catalog/default plus `/model` when needed; not taught as endpoint plumbing. | `/models`; page describes configured/default/last-used resolution and model IDs. | First-run default sufficient for introductory task; `/model` for changes, auth/model availability dependent. | Account selection reviews model/reasoning/service. Defaults exist for those settings, but account default is a separate mandatory action. A simple path should make optional settings recognizably optional. |
| **Ready to send** | Tutorial proceeds to talking after auth. | After connect, choose model/project and optionally init as taught. | Tutorial proceeds directly from sign-in to first task. | Successful enrollment is **not** by itself readiness: host default must exist even when an explicit account is selected for the new voyage. `require_default_account` enforces this. [H7] |
| **No credentials or failure** | Provider page explains supported routes, env/auth lookup and provider-specific fixes. Not proof every error is easy. | Troubleshooting starts with logs/cache/providers/config; auth guidance depends on provider. | Auth tutorial gives browser/headless device alternatives, admin restrictions and diagnostics. Some headless alternatives involve credential copying. | Failure categories, R refresh, N explicitly close/restart, reconnect and retained identities are documented. Useful safety behavior, but alphabetic recovery actions and setup state need contextual explanation. Do not copy credential-relocation tutorials into Helm's host-bound model. |

**Critical avoidable detour:** Helm sign-in, account selection and host-default
selection are three different actions. Keeping explicit billing/default consent is
correct. Requiring a newcomer to discover F6 independently is not inherent to that
boundary. Offer **“Use this account for this task; make it this host's default for
new voyages”** as explicit, scoped choices within the successful setup flow. This
is a proposal, not permission to auto-select the first account or broaden a remote
principal's rights. If the current host-default prerequisite remains mandatory,
explain it before the first failed submission and route the authorized user to it.

## The tutorial must carry the user beyond login

| User intent | What competitor teaching contributes | What Helm should teach/test on the same path |
|---|---|---|
| First useful result | Pi says start talking; Codex supplies “Tell me about this project”; OpenCode walks questions and changes. | One concrete read-only first task in the chosen project, followed by a small requested edit. Show what success looks like; no need to discover F8 tools first. |
| Correct work in progress | Pi explains steering/follow-up; Codex describes steering and follow-ups; OpenCode teaches plan/agent choices and iterative changes. | Describe what typing/Enter does while busy, how to Stop, and how that differs from leaving Helm. Keep this adjacent to the first task, not only in a shortcut appendix. |
| Understand the change | OpenCode teaches undo/redo and sharing; Codex teaches Git checkpoints and review; Pi describes tree/fork/compaction. | Show read-only change inspection and its scope; clearly separate draft undo, conversation branch and filesystem restoration. Do not promise competitor rollback semantics Helm does not implement. |
| Return tomorrow | Pi documents `pi -c`/`pi -r`; Codex exposes resume; OpenCode TUI docs teach session selection. | Reopen Helm, find named voyage, continue with retained context. Teach that disconnect does not stop work, without requiring the user to understand supervisor internals. |
| Recover from failure | Competitors link targeted troubleshooting after the main path, rather than teach every recovery token up front. | Keep draft and safe original operation identity automatically; display one next action in context, with diagnostics available secondarily. Never retry uncertain effects under a new identity. |

## Do not copy tutorial mistakes into Helm

The tutorial benchmark is critical reading, not competitive marketing acceptance:

- **Pi:** its quick start is compact but assumes an account/key and does not supply
  a concrete first task or explicit project-directory step. Borrow proximity of
  setup/model/resume guidance; improve the novice example rather than merely match
  its word count. Current fetched README/provider pages match the available Pi pin
  byte-for-byte; the package-scope change is not new relative to that pin.
- **OpenCode:** `/connect` appears before launch is taught. The provider page's
  Anthropic prose offers Claude Pro/Max while its shown selector only lists API key
  and a later warning says the plugins are no longer bundled and use is prohibited.
  Pi's page separately makes an affirmative extra-usage claim. These conflicting
  application-authored statements are **not an authoritative provider-policy
  resolution**; do not promise that route without checking the provider itself.
  The TUI tutorial also teaches `/details`, while the inspected pinned handler has
  a palette tool-details action without that slash name. Tutorial text and source
  need reconciliation before declaring an end-to-end pass.
- **Codex:** the general quickstart redirects away from CLI; features redirects to
  the CLI page, and model pages mix surfaces/examples. Follow the right surface,
  not a remembered URL. Model names shown in different examples are not one
  guaranteed default or entitlement. Its read-first task and explicit inspection
  outcomes are useful lessons despite those navigation costs.

Helm should therefore be **simpler than the shortest valid comparable journey,
not shorter than an incomplete or stale tutorial**. Preserve necessary consent,
explain it at the point of action, and remove administrative detours rather than
hide consequences.

## Simplification acceptance for #272

These targets are proposed requirements, not implemented features or verified
parity. Measure the same task with the same starting account/project and OS.

| Priority | Change to the overall experience | Concrete acceptance |
|---|---|---|
| **P0 — one complete front door** | One entry page and one recommended local launch path from install to first useful task. Architecture, remote hosting and developer builds become clearly separate branches. | A newcomer follows the entry tutorial without another guide except provider authorization. Every command is current and every required readiness step is included. If releases are unavailable, say so; do not invent installation success. |
| **P0 — readiness-driven provider setup** | No-account launch explains sign in/API key choices, billing distinction and host. After auth, finish account/default readiness in the same guided flow. | No UUID copying, endpoint/transport selection or surprise F6 prerequisite for the normal built-in-provider/local path. Explicit scope/default consent remains, with a safe path when the user lacks authority. |
| **P0 — current auth guidance** | Separate legacy cache login/import/migration from named-account onboarding. | Following the main setup instructions from empty state creates a usable named/default account. `vessel auth login` alone is not represented as completing new-voyage readiness. |
| **P1 — private setup without unnecessary administration** | Friendly built-in provider choices conceal routine connection IDs while preserving private-host input. | Local API enrollment stays out of chat/model history; remote enrollment stays on the executing host. The user chooses provider/account intent, not internal metadata unless using advanced/custom setup. |
| **P1 — defaults with visible consequence** | Keep reasoning/service/model advanced controls out of the required happy path when safe supported defaults exist. | User sees effective provider/model and billing source, can change them, and can finish setup without configuring irrelevant fields. Unknown entitlement remains unknown. |
| **P1 — finish, inspect, return** | A short tutorial demonstrates a real task, change inspection and resumption, plus one failure. | Correct authored text/project/host; result inspectable without a new diagnostic task; return preserves place/context. Record actual screens and task completion, not only tutorial length. |
| **P1 — parity measurement** | Compare minimum valid existing-account paths and novice default tutorials separately. | Record commands, decisions, app/browser/terminal switches, documentation detours, failed attempts and time-to-first-success. Helm must require no extra *unexplained* concept or detour; any necessary extra consent has an explicit benefit and is located where relevant. No numeric superiority claim before execution. |

### Documentation conflict discovered during the tutorial walk

The configuration guide still prominently teaches legacy `vessel auth login`,
`login --device` and `import-codex`. The named-account guide says an old unbound
configuration must select a named account or explicitly migrate before admission;
new voyages require an explicit host default. These commands can remain valid
legacy operations without forming a complete first-use tutorial. The problem is
routing the newcomer through incompatible stages without a single ready-to-send
outcome. This review records the problem rather than silently claiming the short
legacy example is the actual minimal current journey. [H2,H3,H7]

## Evidence and verification

### Official tutorial sources (live observations, not pinned implementation)

- **T1:** [Pi Quick Start](https://github.com/earendil-works/pi/tree/main/packages/coding-agent#quick-start).
- **T2:** [Pi provider authentication](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/providers.md).
- **T3:** [OpenCode introduction/install/configure/first task](https://opencode.ai/docs/).
- **T4:** [OpenCode providers](https://opencode.ai/docs/providers/), [models](https://opencode.ai/docs/models/), [TUI](https://opencode.ai/docs/tui/), [troubleshooting](https://opencode.ai/docs/troubleshooting/).
- **T5:** [Codex CLI quickstart](https://developers.openai.com/codex/cli).
- **T6:** [Codex authentication](https://developers.openai.com/codex/auth), [CLI features](https://developers.openai.com/codex/cli/features), [CLI commands](https://developers.openai.com/codex/cli/slash-commands). The generic [models URL](https://developers.openai.com/codex/models) redirects to a mixed-surface page; only its explicitly labeled interactive CLI `/model` / `--model` section supports CLI claims, not the desktop power-control illustration.

Subscription labels do not guarantee included usage. Pi's current provider page,
for example, describes Claude subscription authentication as extra usage billed
per token, and OpenRouter OAuth as minting a key billed from credits. These are
attributed tutorial claims, not provider/billing validation. Use the selected
provider's current terms and observed account eligibility for any live trial.

### Helm sources

- **H1:** [root README](../../README.md), [Helm README](../../helm/README.md).
- **H2:** [configuration guide](../configuration.md), especially native auth examples.
- **H3:** [named accounts](../provider-accounts.md): required default, device sign-in, private API enrollment and legacy migration.
- **H4:** [UI account implementation](../../helm/src/process_client/ui/accounts.rs).
- **H5:** [installer tutorial](../../installer/README.md).
- **H6:** [bootstrap prerequisites](../../install.sh), observed GitHub release-list result described above.
- **H7:** [default-account enforcement](../../voyage/src/config.rs) (`require_default_account`) and [start validation](../../vessel/src/process/start.rs).

Downloaded pages, extracted text and retrieval metadata remain under ignored
`target/ux-audit/tutorials/`. The compact manifest below records actual fetch time,
redirect destination and source hash so live tutorial observations are not silently
attributed to the earlier code pins. Source links, documentation references and
Git diff are checked; no Rust change means no coverage rerun. #272 remains open
for implementation and hands-on simplicity measurement.

### Tutorial retrieval manifest

HTTP status was 200 for the entries below. A successful page fetch is not a successful product journey. All times are UTC.

| Requested page | Final URL | Retrieved UTC | SHA-256 of downloaded page |
|---|---|---|---|
| https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/README.md | https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/README.md | 2026-09-13T16:48:36.355082+00:00 | `b5daa09877da34d122818e03436b51755bc1b5044ae8ab810df63f02888f1ebe` |
| https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/docs/providers.md | https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/docs/providers.md | 2026-09-13T16:48:36.356509+00:00 | `c8741ab21f78c4df43a9068ff05c06639a2fd961116642722b034756a8c26dc8` |
| https://opencode.ai/docs/ | https://opencode.ai/docs/ | 2026-09-13T16:48:36.357456+00:00 | `cf2ac3f268c57590b98ae074b1ddaf101c8f958ba70bb11707323b14f5796874` |
| https://opencode.ai/docs/providers/ | https://opencode.ai/docs/providers/ | 2026-09-13T16:48:36.358162+00:00 | `6043fe7e5eb7980e659586b126ea052a55cd950d126dd5745e841347134b8e15` |
| https://opencode.ai/docs/models/ | https://opencode.ai/docs/models/ | 2026-09-13T16:48:36.484583+00:00 | `1011a8a7482a467f15daff9b882dea390e00e1e7fb3958cda78d49050e1852a8` |
| https://opencode.ai/docs/tui/ | https://opencode.ai/docs/tui/ | 2026-09-13T16:48:36.486831+00:00 | `9026af9e4f04caef2144af40bdb571ddabba63bdd94d5897fda95ff9c346b3d7` |
| https://opencode.ai/docs/troubleshooting/ | https://opencode.ai/docs/troubleshooting/ | 2026-09-13T16:48:36.578898+00:00 | `091855978b414964739cc4f47a97ea3b303c090994fc517a253287a1d444592b` |
| https://opencode.ai/docs/zen/ | https://opencode.ai/docs/zen/ | 2026-09-13T16:48:36.875135+00:00 | `a92e49faac8a5f74fb056a284e5bd60cee29f35916ad14cbe68261833c033415` |
| https://developers.openai.com/codex/quickstart | https://learn.chatgpt.com/docs/quickstart | 2026-09-13T16:48:36.901524+00:00 | `cb154e81fb036705b22234081d54b2c9f86c5eac6ade2a48e6c6af7f7cf92b2f` |
| https://developers.openai.com/codex/cli | https://learn.chatgpt.com/docs/codex/cli | 2026-09-13T16:48:36.902173+00:00 | `1a44433580b980ebe47a082f13464837f559a8605ab3f438a4b57e70739a128f` |
| https://developers.openai.com/codex/cli/features | https://learn.chatgpt.com/docs/codex/cli | 2026-09-13T16:48:36.934476+00:00 | `1a44433580b980ebe47a082f13464837f559a8605ab3f438a4b57e70739a128f` |
| https://developers.openai.com/codex/cli/slash-commands | https://learn.chatgpt.com/docs/developer-commands?surface=cli | 2026-09-13T16:48:37.140553+00:00 | `0c0313510d8c80ea833bc9ac19631a596d5f7c1ace1da884247ab5f502ea1699` |
| https://developers.openai.com/codex/auth | https://learn.chatgpt.com/docs/auth | 2026-09-13T16:48:37.492309+00:00 | `2e544481fbdf266c5b2dc6db0ea759529f07b75f5b7fdd86f3c118d608497918` |
| https://developers.openai.com/codex/reference/troubleshooting | https://learn.chatgpt.com/docs/reference/troubleshooting | 2026-09-13T16:48:37.608683+00:00 | `6171a04080c2e89a7e7552b3162747e64b93370e3f9d216141149ded074bd836` |
| https://developers.openai.com/codex/models | https://learn.chatgpt.com/docs/models | 2026-09-13T16:48:37.636146+00:00 | `a48b9d4778876baebdf1632da83d90c1ea5c28f2abed1a43cbb39c00b6e7950d` |
| https://developers.openai.com/codex/pricing | https://learn.chatgpt.com/docs/pricing | 2026-09-13T16:48:37.940950+00:00 | `3a4faf0c908be22a57147b0b03e010c9395a5a49b7b112bd4a0d993d040f2830` |
