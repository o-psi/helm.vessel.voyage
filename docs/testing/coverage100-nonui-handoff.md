# Non-UI synthetic coverage handoff (#353)

Base: `d10e453`. Tests only; production changes are test-module declarations.
No exclusions were added. No Cargo command, build, test execution, coverage run,
provider request, human-terminal attachment, or real-browser launch was performed
for this batch. Formatting and `git diff --check` were checked locally. The parent
must compile and execute these tests before treating them as passing coverage.

## Added scenarios

- Host-browser HTTP handlers: missing authentication components, malformed JSON,
  wrong launch tokens, one-use bootstrap context, page Host enforcement,
  cancellation and operation gate contention.
- Browser WS adapter: status modes, invalid binding removal, prepared-owner
  revision refresh, preservation of command identity, repeated-preparation refusal.
- Transport: full Status/Start/Receipt prepared/session/owner identity matrix;
  malformed revisions, mismatched snapshot owner, non-Voyage browser and terminal
  responses. Peers are local scripted WebSockets, not Vessels or providers.
- Frontend: prompt byte/blank validation before opening an owner, skipping failed
  name snapshots, canonical workspace acceptance, malformed catalogue/restart.
- Admin: exact JSON byte limit, malformed and missing files, assignment inspection
  followed by owner-bound observation with exact IDs and cancellation flag.
- Managed: live/suspended preservation, stopped restart, unavailable/starting
  refusal using the existing scripted dispatcher.
- Terminal: paste byte budget and Unicode limits, complete modifier/navigation
  matrix, shared frame budget reset and poison handling, already-finished cleanup.
- Main CLI: connected configuration overrides fail before network/config loading.

## Environmental gaps and remaining work

These are constraints of this synthetic-only batch, not claims that the lines are
universally unreachable or grounds for exclusions:

- `terminal.rs` live attach requires both stdin and stdout to be TTYs. Raw mode,
  native event stream, tcflush, resize/signal handling and restoration on actual
  outer-terminal failures need an isolated PTY/subprocess harness. This batch
  does not redirect process-global stdout or interact with a human terminal.
- `terminal/output.rs` real `fcntl`, `write` and `poll` error/timeout branches need
  controlled child-process descriptors (closed fd/full pipe or PTY). Budget
  branches are tested without changing process-global descriptors.
- `host_browser.rs` full viewer lifecycle includes private launcher creation,
  permissions, HTTP server shutdown and connection/control races. The adapter
  and handlers have synthetic tests, but that does not establish lifecycle
  coverage or native browser launch behavior.
- `frontend.rs` open/run success paths require a local owning Vessel, persisted
  launch configuration and full follow/cleanup protocol. More synthetic fixtures
  remain feasible; these paths are not inherently environmental/unreachable.
- Platform-specific non-Unix terminal fallbacks cannot execute in a Unix run.
- The baseline summary is aggregate line coverage, not branch evidence. Its
  selected line percentages were frontend 25%, terminal 27.65%, host browser
  44.34%, transport 82.04%, admin 97.22%, main 65.57%, managed 43.75%. No post-change
  percentage or 100% reachability claim is supported until the parent measures.
