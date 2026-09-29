"""Offline public-v2 process replay/privacy acceptance against supervised binaries.

Run only after #367/#368 are integrated and the affected debug binaries rebuilt:
    python3 voyage/tests/live_events_integration.py --bin-dir target/debug
No live provider, credentials, or client UI are involved. This does not certify Web
or TUI rendering; it verifies the shared server subscription boundary.
"""
import argparse
import json
from pathlib import Path
import sys
import uuid

from delivery_recovery import Fixture, wait_for


def page(fixture, session, after=0, limit=4, projection="public-v2"):
    return fixture.command(session, {"op": "events", "after": after,
        "limit": limit, "wait_ms": 0, "projection": projection})


def collect(fixture, session, projection="public-v2"):
    cursor = 0
    events = []
    for _ in range(256):
        response = page(fixture, session, cursor, projection=projection)
        assert response["projection"] == projection, response
        assert response["replay_gap"] is False, response
        chunk = response["events"]
        assert all(item["session_id"] == session for item in chunk), response
        positions = [item["cursor"] for item in chunk]
        assert all(previous < current for previous, current in zip([cursor] + positions, positions)), response
        assert response["cursor"] == (positions[-1] if positions else cursor), response
        assert response["cursor"] <= response["latest_cursor"], response
        events.extend(chunk)
        cursor = response["cursor"]
        if not response["has_more"]:
            return events, cursor
        assert chunk, response  # A busy stream must make bounded forward progress.
    raise AssertionError("event pagination did not terminate")


def run(fixture):
    first = fixture.session()
    second = fixture.session()
    marker = "fixture-public-live-" + uuid.uuid4().hex
    # Receipt admission must be exact; event observation must not submit commands.
    command = fixture.submit(first, "public live event " + marker)
    fixture.command(first, command)
    finished = fixture.finished(first)
    assert finished["run"]["state"] == "completed", finished["run"]

    v2, cursor = collect(fixture, first)
    assert v2 and cursor > 0, v2
    assert any(event["kind"] == "message_finalized" for event in v2), v2
    assert any(event["kind"] == "run_state" for event in v2), v2
    assert any(event["kind"] == "command_outcome" for event in v2), v2
    assert all(event["revision"] <= finished["revision"] for event in v2), v2
    assert all(len(json.dumps(event["payload"], ensure_ascii=False).encode("utf-8")) <= 32 * 1024
               for event in v2), "oversize v2 payload"
    assert page(fixture, first, cursor)["events"] == []  # No duplicates on resume.
    assert collect(fixture, first)[0] == v2  # Replayed page contents are stable.
    assert all(event["session_id"] == second for event in collect(fixture, second)[0])

    v1, _ = collect(fixture, first, projection="public-v1")
    assert v1, "existing public-v1 replay vanished"
    assert all("payload" not in event for event in v1), "v1 exposed v2 content"
    default = fixture.command(first, {"op": "events", "after": 0, "limit": 4, "wait_ms": 0})
    assert default["projection"] == "public-v1", default

    # A deliberately invalid projection must refuse, not silently downgrade.
    invalid = fixture.command(first, {"op": "events", "after": 0, "limit": 4,
        "wait_ms": 0, "projection": "public-v3"}, allow_error=True)
    assert invalid["error"] is not None, invalid
    fixture.record("public-v2-process-replay", {"session": first, "cursor": cursor,
        "kinds": sorted({event["kind"] for event in v2}), "count": len(v2),
        "revision": finished["revision"], "v1_count": len(v1),
        "other_session_count": len(collect(fixture, second)[0])})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        raise SystemExit("Do not run with python -O; assertions are the test oracle")
    binaries = args.bin_dir.resolve()
    for name in ("vessel", "voyage"):
        assert (binaries / name).is_file(), f"missing binary: {binaries / name}"
    fixture = Fixture(binaries)
    try:
        fixture.start()
        run(fixture)
        print("public-v2 process replay PASS", flush=True)
    finally:
        fixture.close()


if __name__ == "__main__":
    main()
