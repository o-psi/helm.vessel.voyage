# Native Voyage evaluation with Harbor

This opt-in local adapter evaluates the real **Helm → Vessel → independent
Voyage** path. Harbor supplies isolated task environments and independent
verifiers; it does not replace Voyage's agent/tool loop. The initial experiment
is tracked in [#397](https://github.com/o-psi/helm.vessel.voyage/issues/397).
The completed [2026-09-30 baseline](baseline-2026-09-30.md) includes all 30 trials.
This is new explicitly requested evaluation work, not restoration of the retired
broad evaluation runner. GitHub automation remains build-only.

## Reproducibility and requirements

- Linux x86-64 with a usable rootless Docker daemon and Docker Compose.
- Harbor **0.23.0**, installed with Python 3.12, and the adapter's `aiohttp`/`httpx`
  dependencies from that pinned Harbor environment.
- A pinned Terminal-Bench 2.1 checkout and a recorded runtime source/build.
- An existing ready named native ChatGPT subscription account on the executing
  PC. Live inference needs an approved model/provider and budget. Keep API billing,
  subscription allowance, reported tokens and unknown monetary cost separate.

The initial PC setup used user-local uv 0.12.20, Docker/rootless 29.8.1 and
Compose 5.5.1. Rootless prerequisites include subordinate UID/GID mappings and
working `newuidmap`/`newgidmap`. Use Docker's maintained
[rootless setup guide](https://docs.docker.com/engine/security/rootless/); inspect
its installer before execution. Do not weaken host permissions, grant broad
Docker-socket access or disable sandbox policy to make a trial pass.

```sh
uv tool install 'harbor==0.23.0' --python 3.12
DOCKER_CONTEXT=rootless docker info
DOCKER_CONTEXT=rootless docker compose version
```

The rootless daemon is a separate user service. The initial installation enabled
it at user login and selected Docker's `rootless` CLI context. No system-wide
Docker service, sudo permission or Docker-group membership was changed.

## Runtime build and credential boundary

Published nightlies may require newer glibc than some benchmark images. The
initial compatible benchmark variant compiled unchanged Rust/Cargo sources in
`rust:1.94.1-slim-bookworm`, using:

```sh
cargo build -p helm -p vessel -p voyage --release --locked \
  --features reqwest/rustls-tls-native-roots -j 8
```

Record the source commit, compiler/image identity, exact flags and hashes in
`benchmark-build.json`, and put `helm`, `vessel`, `voyage` in `bin/` alongside
`BUILD.txt`. Use a target/cache appropriate to this different libc/build context;
do not contaminate the checkout's existing coverage/release objects. Do not
change checked-in Cargo versions or label this local variant a published release.
The baseline source is **675f12ed1f05b03f6fc45828993f95fec8e55b85**. Its published
nightly archive/checksum were independently downloaded and verified before the
benchmark variant was needed. No Rust source or Cargo input was modified.

Named subscription accounts enforce the fixed native backend origin. The adapter
preserves that origin and TLS verification. Each task container trusts a dedicated
short-lived local CA and resolves `chatgpt.com` to its own loopback TLS relay.
The relay forwards native request/response bytes over a mounted Unix socket.
Only the host broker reads real account tokens and injects them on requests to
`https://chatgpt.com/backend-api/codex`; upstream TLS and redirect refusal remain
independent. No host DNS or certificate trust store is modified.

Only the scoped relay token, server certificate/key and CA certificate enter a
task container. Keep the CA signing key and real OAuth registry outside the mount.
The broker allows only models/responses, gpt-6.1-sol and medium/high/xhigh, bounds
requests, checks account readiness/identity changes, and does not rotate accounts
or replay uncertain requests. An expired or unavailable account needs native
reconciliation. This relay and the native-root build difference must be included
in every comparison report.

## Launching an experiment

Create a private broker directory with a `socket/` subdirectory. Provision
`socket/provider-ca.crt`, a CA-signed `socket/provider.crt` for DNS `chatgpt.com`
with `CA:FALSE` and `serverAuth`, and `socket/provider.key`. Keep signing material
outside `socket/`; the mounted directory is read-only. The broker parent should
be mode 0700, socket directory traversable for the task user, and private keys
mode 0600. Never disable certificate verification.

Run the broker with Harbor's Python environment, with `eval/` on `PYTHONPATH`:

```sh
python -m voyage_harbor.broker --directory /private/benchmark-broker \
  --account ACCOUNT_UUID --mode live --max-requests 6000
```

The broker stays in the foreground. Its scoped token is written privately to
`proxy-token`, never printed. `--mode mock` makes no upstream provider requests
and supports a deterministic native shell/file smoke task. Broker statistics keep
usage and identifiers, not requests, responses, credentials or reasoning text.
Stop the owned broker after the experiment; do not restart a broker over an active
socket or automatically resume uncertain external work.

```sh
python eval/voyage_harbor/run.py \
  --dataset /path/to/pinned/terminal-bench-2-1 \
  --runtime /path/to/verified/benchmark-runtime \
  --broker-directory /private/benchmark-broker \
  --output /private/new-experiment \
  --job-name voyage-medium-high-xhigh

python eval/voyage_harbor/report.py /private/new-experiment \
  --output /private/new-experiment/summary.json
```

`run.py` refuses to overwrite an experiment, freezes the adapter into a copied,
hashed snapshot, records the dataset/build identity and selected tasks before
admission, and runs the same ten tasks once per effort. Three trials may overlap,
with at most one per effort; there are no automatic trial retries. Task-declared
CPU, memory and time settings are retained. Report unsupported resource
constraints explicitly rather than claiming enforcement from metadata alone.

`agent.py` installs the native programs. `runner.py` provisions private account,
configuration and supervisor state inside the disposable container, creates an
exact session, submits its task through Helm, exports public history, inspects
canonical completion and verifies matching `stopped.json` cleanup evidence.
The adapter also refuses grading after normal exit unless runtime cleanup is
observed. On an agent deadline, it explicitly stops the exact Voyage incarnation
and observes matching cleanup before propagating the timeout to Harbor. If cleanup
cannot be observed, it fails outside the recoverable agent-exit classes so shared
verification is prohibited. A forced-stall offline check verifies that cancellation
produces a truthful incomplete outcome and observed cleanup before grading.
The trial's full container retirement is a separate Harbor obligation. Neither a
successful answer nor a stopped supervisor alone proves resource cleanup.

## Evidence and limits

Initial offline controls used an isolated file task: oracle reward 1; no-op reward
0; native scripted-provider reward 1 with actual file output and matched runtime
cleanup. A live gpt-6.1-sol medium smoke also received reward 1 and observed cleanup.
These validate integration; they are not Terminal-Bench capability scores.

Retain initial setup failures and exact run identities. For real trials report
independent rewards, exceptions, canonical outcome, cleanup, elapsed time, and
provider-reported completed-request usage. Failed/unfinished inference may have
unknown usage; cached input is included in total input, and dollars are unknown
for this subscription path. Separate setup/provider failures, refusals, timeouts
and incorrect solutions. A 10-task, one-attempt pilot is not an official full
benchmark or a statistically strong ranking.

No human acceptance gate, native macOS/Windows claim, hosted test/coverage job,
public trace upload or stable release is implied. Raw traces remain private;
publish only inspected, sanitized summaries. Follow [quality](../../docs/quality.md)
and [project instructions](../../AGENTS.md) for in-scope local verification and
delivery. Source/test changes that trigger workspace coverage still require it;
this adapter does not make historical Rust coverage a new measurement.

The tested runtime cleans up run-owned shell descendants at run finalization.
Tasks that require persistent services after the agent ends can therefore fail
verification even after the model demonstrated working services. An offline
HTTP-service control observed the service dead after canonical completion and
before explicit adapter stop. Preserve this boundary; do not disable cleanup to
inflate scores. Treat service-dependent task outcomes as a compatibility finding.

The first matrix exposed and retained the timeout-detach flaw; it was cancelled
and marked invalid. The corrected matrix uses a fresh adapter snapshot and fresh
containers. Do not merge invalid pilot rows into the final comparison.
