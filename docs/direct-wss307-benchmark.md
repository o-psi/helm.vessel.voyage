# #307 local synthetic transport measurement

## Scope and result

Measured 2026-09-16 (UTC), Linux x64, kernel 6.18.50-2-lts,
AMD Ryzen 7 8840U, 16 logical CPUs; Node v26.8.2, `ws` 8.21.3.
This is **local synthetic transport evidence**, not deployed-browser acceptance,
real Vessel execution, capacity certification, whole-deployment resource savings,
or a power/watts measurement.

The unchanged legacy Node gateway was exercised through its existing test-only
transport injection point. Direct mode removes that process; it does **not**
remove the hosted bootstrap process. Both modes use the same mock Vessel replies
and browser-like Node `ws` clients. At ten concurrent users / fifty sockets,
legacy gateway CPU alone was a median 1,183 ms over approximately 2.4 seconds
including bootstrap, with 142 MiB sampled peak RSS and about 100.5 MB received
**and** 100.5 MB sent. In direct mode those relay bytes go between client and
mock Vessels, not the hosted bootstrap. The remaining mock hosted process still
used CPU, memory, and network, as tabulated below. This demonstrates the relay
cost removed in this fixture, not a prediction for production PHP/HTTP/TLS costs.

## Reproduce

From the checkout containing the script:

```sh
# Only if dependencies are absent in this shared worktree:
(cd web/gateway && npm ci --ignore-scripts)
node --check web/tests/direct-transport-benchmark.mjs
node web/tests/direct-transport-benchmark.mjs "$PWD/target/direct-wss-307"
```

Use a fresh output directory to retain an earlier measurement. The dependency
symlink is an untracked local prerequisite (do not stage it) and reuses the already installed dependency; no install,
browser, Rust build, real OAuth, or provider call is involved. No existing
application source files were edited for this measurement.

Detailed ignored evidence: root `target/direct-wss-307/results.json`, individual
`{users}users-{repeat}-{mode}.json`, and corresponding gateway/process `.log`
files. `results.json` records environment, sizes, individual repeats, per-role
metrics at bootstrap and finish, client-driver metrics, and SHA-256 source
fingerprints. All 18 scenarios exited successfully with **45,000 replies**
validated for request identity, null error, and expected text byte length.

Measured script SHA-256:
`5db6f3ed723715d4c2af38c94542399a11e8efc0bb95cf006be8ef0219688d06`.
Measured gateway SHA-256:
`77eb8b108a7cf49b562d3374bc2594b53d01ea5ec8a6a922126126075c86c6d0`.
These identify dirty-worktree sources, not a claimed clean source commit.

## Design and accounting

- 1, 4, and 10 concurrent users, each with five distinct logical Vessel identities
  and five sockets. The five logical Vessel endpoints share one mock Node process,
  **not five separately measured real Vessel processes**.
- Three fresh-service-process repetitions per size/mode; ordering alternates.
  No warmup subtraction. Each socket performs ten rounds of one 32 KiB snapshot,
  one 128 KiB history reply and eight 4 KiB live-output replies. These are chosen
  representative synthetic text sizes, not an empirical production distribution
  or complete canonical snapshot/history schemas. Gateway wire envelopes and
  commands are valid. Text payload is identical between modes.
- Ten requests/round, sequential per socket, with 20 ms pacing after each reply;
  sockets/users run concurrently. Default gateway limits are unchanged, including
  64 connections and 60 requests/second/socket. This is a fixed-work paced test,
  **not saturation throughput**. Replies total 500 / 2,000 / 5,000 and text payload
  totals 9.375 / 37.5 / 93.75 MiB per scenario.
- Separate child processes measure gateway, hosted mock, and Vessel mock CPU via
  `process.cpuUsage()` and RSS via `process.memoryUsage()`. CPU counters reset
  after service startup and include bootstrap plus traffic. Module loading and
  process startup CPU are excluded. RSS is absolute process memory, not incremental
  workload allocation; peak RSS sampled every 10 ms can miss shorter peaks.
  Memory sampling and gateway logging are included overhead. Legacy gateway logs
  are retained, not disabled. Client driver is the parent process and its RSS can
  include previous repetitions and retained child logs; it is not browser memory
  and is not included in the service comparison below.
- Actual per-process socket `bytesRead`/`bytesWritten` include HTTP bootstrap,
  redemption, upgrade headers, WebSocket envelopes/framing/masks, and commands.
  Both gateway ingress/egress legs are counted. Metrics are collected before
  closing sockets, so exclude teardown and TCP/IP/link-layer overhead. IPC/log
  pipes are not network traffic. Loopback uses unencrypted HTTP/WS, no compression,
  TLS, DNS cost, proxy, WAN latency, or retransmission measurement. Do **not** sum
  all role byte counts as unique network payload: that double-counts each link.
- Remaining bootstrap is **actual measured mock HTTP traffic implementing a
  simplified model**, not the deployed endpoint implementation. Each user makes
  one aggregate bootstrap POST returning five synthetic tickets. Legacy makes
  five additional real gateway-to-mock redemption POSTs/user. Direct bootstrap
  returns five endpoint URLs/tickets; direct clients send authenticate frames,
  and the Vessel mock accepts them without real credential verification. The mock
  retains minted ticket entries until process exit (also in direct mode).
  It does not execute PHP, sessions, database operations, pairing, credential
  encryption, ticket cryptography, renewal, reconnect, or production authorization.
  Separate bootstrap service counters in raw JSON make that cost explicit.
- Both modes parse replies in the same Node client. No UI rendering, actual
  browser networking/security behavior, or gateway-stop journey is tested. The
  fixture's transport injection bypasses public-address restrictions only for
  this test, just as the existing gateway unit fixture does.

## Observed medians (three runs; not confidence intervals)

CPU is user + system milliseconds for bootstrap plus workload. RSS is sampled
peak MiB. Bytes are decimal bytes and include bootstrap. Hosted = mock bootstrap
and redemption service, not a deployed web stack. Vessel = all five mock endpoints.

| Users | Mode | Process | CPU ms | RSS MiB | Ingress bytes | Egress bytes |
|---:|---|---|---:|---:|---:|---:|
| 1 | legacy | gateway | 193.53 | 99.05 | 10,047,880 | 10,046,235 |
| 1 | legacy | hosted | 34.64 | 80.17 | 1,416 | 3,350 |
| 1 | legacy | Vessel | 110.03 | 91.48 | 125,590 | 9,919,980 |
| 1 | direct | hosted | 29.91 | 80.16 | 211 | 855 |
| 1 | direct | Vessel | 117.10 | 91.04 | 125,435 | 9,919,980 |
| 4 | legacy | gateway | 488.28 | 105.36 | 40,191,520 | 40,184,940 |
| 4 | legacy | hosted | 51.81 | 81.92 | 5,664 | 13,400 |
| 4 | legacy | Vessel | 265.25 | 94.12 | 502,360 | 39,679,920 |
| 4 | direct | hosted | 35.50 | 80.61 | 844 | 3,420 |
| 4 | direct | Vessel | 211.62 | 94.99 | 501,740 | 39,679,920 |
| 10 | legacy | gateway | 1,183.07 | 142.06 | 100,478,800 | 100,462,350 |
| 10 | legacy | hosted | 82.27 | 82.61 | 14,160 | 33,500 |
| 10 | legacy | Vessel | 541.40 | 98.18 | 1,255,900 | 99,199,800 |
| 10 | direct | hosted | 46.65 | 80.80 | 2,110 | 8,550 |
| 10 | direct | Vessel | 525.12 | 99.43 | 1,254,350 | 99,199,800 |

| Users | Mode | Bootstrap elapsed ms | Traffic elapsed ms | Per-run reply p95 ms (median) |
|---:|---|---:|---:|---:|
| 1 | legacy | 25.84 | 2,097.98 | 1.72 |
| 1 | direct | 11.84 | 2,061.20 | 1.06 |
| 4 | legacy | 73.64 | 2,110.95 | 2.35 |
| 4 | direct | 19.95 | 2,067.36 | 1.59 |
| 10 | legacy | 149.19 | 2,239.42 | 7.39 |
| 10 | direct | 55.05 | 2,100.54 | 2.53 |

Latency excludes bootstrap and the explicit pacing delay. The shared host was
not reserved/isolated; CPU scheduling, GC, cache state and other host work can
influence these short runs. Three repeats do not establish statistical
significance, production concurrency limits, or reliable absolute latency.

## Remaining acceptance work

A controlled deployed browser journey must still verify five enrolled Vessels,
multiple concurrent authenticated users, real bootstrap/session costs, actual
snapshot/history/live output, and continued operation after stopping the legacy
gateway. TLS, browser Origin behavior, reconnect/renewal, credential handling,
errors and real deployment resource counters require separate evidence. Nothing
here claims those checks passed or closes #307. Issue publication and deployment verification are separate from this benchmark.
