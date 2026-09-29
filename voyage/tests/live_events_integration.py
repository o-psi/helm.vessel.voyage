"""Offline public-v2 replay, restart and delayed-page checks against supervised binaries.

Run only after #367/#368 are integrated and the affected debug binaries rebuilt:
    python3 voyage/tests/live_events_integration.py --bin-dir target/debug
No live provider, credentials, or client UI are involved. This does not certify Web
or TUI rendering; it verifies the shared server subscription boundary.
"""
import argparse
import json
import os
import subprocess
import time
from pathlib import Path
import sys
import uuid

from delivery_recovery import Fixture as BaseFixture



class Fixture(BaseFixture):
    """Explicit synthetic account; never borrow the executing host's default."""
    def __init__(self, binaries):
        super().__init__(binaries)
        self.env["LIVE_EVENTS_FIXTURE_KEY"] = "synthetic-fixture-key"

    def session(self):
        session = str(uuid.uuid4())
        endpoint = f"http://127.0.0.1:{self.provider.server_port}/v1"
        def account_cli(*args):
            result = subprocess.run([str(self.binaries / "vessel"), "auth", "accounts", *args],
                env=self.env, cwd=self.workspace, capture_output=True, text=True, timeout=15)
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        connection = account_cli("connect", "--label", session, "--endpoint", endpoint,
            "--transports", "openai-responses")
        account = account_cli("add", "--connection", connection["id"], "--account", session,
            "--env", "LIVE_EVENTS_FIXTURE_KEY")
        binding = {"account_id": account["id"], "connection_id": connection["id"],
            "identity_generation": account["identity_generation"],
            "connection_revision": connection["revision"], "transport": "openai_responses"}
        config = self.root / (session + ".json")
        config.write_text(json.dumps({"version":1, "workspace":str(self.workspace),
            "config":{"provider":"openai-responses", "model":"fixture-model",
                "api_key_required":False, "base_url":endpoint, "account":binding,
                "access":"read-only", "provider_retry_attempts":1,
                "context_window":0, "command_timeout_secs":2},
            "explicit":{"access":"read-only"}, "selection":None, "confirmation":None}))
        config.chmod(0o600)
        self.request({"op":"start_settings", "session_id":session,
            "command_id":str(uuid.uuid4()), "workspace":str(self.workspace),
            "config_path":str(config), "binding":binding, "settings":{}})
        self.sessions.append(session)
        return session

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

    # Replay survives supervisor restart without waking the suspended owner or
    # replaying inference. Retain the exact journal prefix before the restart.
    fixture.suspended(first)
    fixture.suspended(second)
    before_owner = fixture.request({"op": "inspect", "session_id": first})
    retained, retained_cursor = collect(fixture, first)
    requests_before = len(fixture.provider.bodies)
    fixture.supervisor.terminate()
    assert fixture.supervisor.wait(timeout=10) == 0
    fixture.start()
    restarted_owner = fixture.request({"op": "inspect", "session_id": first})
    assert restarted_owner["state"] == "suspended", restarted_owner
    assert restarted_owner["incarnation"] == before_owner["incarnation"]
    assert collect(fixture, first) == (retained, retained_cursor)
    assert len(fixture.provider.bodies) == requests_before
    fixture.record("public-v2-replay-after-supervisor-restart", {
        "cursor": retained_cursor, "count": len(retained), "inference_replayed": False})

    # A genuinely fresh Voyage incarnation appends to the same journal. Delayed
    # one-event pages and repeated older pages must not lose or duplicate content.
    follow_up = fixture.submit(first, "public live event explicit follow-up " + marker)
    fixture.command(first, follow_up)
    next_finished = fixture.finished(first)
    assert next_finished["run"]["state"] == "completed", next_finished["run"]
    assert len(fixture.provider.bodies) == requests_before + 1
    new_owner = fixture.request({"op": "inspect", "session_id": first})
    assert new_owner["incarnation"] != before_owner["incarnation"]
    all_events, next_cursor = collect(fixture, first)
    assert all_events[:len(retained)] == retained
    assert next_cursor > retained_cursor
    slow_events, slow_cursor = [], retained_cursor
    for _ in range(256):
        response = page(fixture, first, slow_cursor, limit=1)
        assert response["replay_gap"] is False
        assert len(response["events"]) <= 1
        if not response["events"]:
            assert response["has_more"] is False
            break
        assert page(fixture, first, slow_cursor, limit=1)["events"] == response["events"]
        slow_events.extend(response["events"])
        assert response["cursor"] > slow_cursor
        slow_cursor = response["cursor"]
        time.sleep(0.02)
        if not response["has_more"]:
            break
    else:
        raise AssertionError("delayed one-event paging did not terminate")
    assert slow_events == all_events[len(retained):]
    assert slow_cursor == next_cursor
    assert len(fixture.provider.bodies) == requests_before + 1
    fixture.record("public-v2-fresh-owner-delayed-page-replay", {
        "previous_cursor": retained_cursor, "cursor": slow_cursor,
        "additional_events": len(slow_events), "fresh_incarnation": True})


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
