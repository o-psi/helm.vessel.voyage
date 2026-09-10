# Two voyages doing useful work

Focused offline Linux happy-path test for issue #247:

```sh
python3 voyage/tests/two_voyages.py --bin-dir target/debug
```

Requires Python 3.11+, Linux `/proc` and pidfds, and existing `vessel` and
`voyage` binaries. Build them beforehand through the checkout's coordinated
build workflow; the test does not invoke Cargo. Do not use Python `-O`, which
disables assertions.

The standalone standard-library fixture creates a private temporary root,
HOME/XDG directories and one shared workspace. It starts a real Vessel and two
independent supervised voyages. No user credentials or configuration are
inherited; an unauthenticated scripted provider listens only on loopback.

Each voyage receives a distinct report-writing request and executes the native
`write_file` tool. The provider withholds both final replies until **both** tool
results have arrived and the corresponding files have their expected contents.
Before releasing that barrier, the test requires two distinct live voyage PIDs,
both run snapshots in `running` state, and both actual output files. Sequential
completion cannot satisfy this barrier.

After release, both runs must complete and suspend. The test then reads their
separately retained snapshots through Vessel and checks session identity, exact
user prompts and assistant replies, successful per-voyage tool results, completed
turns and unchanged output files. It also checks that the other report's unique
content did not enter the conversation.

Waits, HTTP requests and the overlap barrier are bounded. Cleanup signals only
voyages whose executable and private session directory match this fixture,
using pidfds with a second ownership check. It observes their disappearance and
waits for its owned supervisor. Forced cleanup is attempted if necessary but is
reported as a failure, not a passing happy path.

The printed `/tmp/voyage-two-voyages-*` evidence directory is retained, including
`overlap.json`, separate `*-retained.json` snapshots, actual workspace files,
provider requests/errors, the supervisor log and `cleanup.json`. It is private
local evidence, not a publication artifact. A successful cleanup records no
remaining owned voyage PIDs and no live provider serving thread.

This verifies runtime orchestration and real file-tool effects, not model
intelligence, live-provider interoperability, Helm TUI behavior, throughput,
failure recovery, or native macOS/Windows behavior. It does not depend on or
restore the removed broad concurrent-voyage suite. The coordinating delivery
owns combined coverage measurement, integration and publication.
