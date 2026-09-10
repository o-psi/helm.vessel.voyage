"""Offline Linux happy path: cancel inference, observe cleanup, continue the voyage."""
import argparse
import http.server
import json
from pathlib import Path
import select
import threading
import time
import uuid

from delivery_recovery import Fixture, wait_for


FIRST = "Start the first task; remember the word lighthouse."
SECOND = "Continue with a fresh task using the retained word."
ANSWER = "The retained word is lighthouse; the fresh task is complete."


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            assert self.path == "/responses", self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert body["stream"] is True
            self.server.bodies.append(body)
            users = [item["content"] for item in body["input"] if item.get("role") == "user"]
            if len(self.server.bodies) == 1:
                assert FIRST in json.dumps(users), users
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.wfile.flush()
                self.server.started.set()
                # Never complete the first response. Observe the executing voyage
                # closing inference, rather than inferring cancellation from its receipt.
                deadline = time.monotonic() + 20
                while time.monotonic() < deadline and not self.server.stop.is_set():
                    if select.select([self.connection], [], [], 0.1)[0]:
                        assert self.connection.recv(1) == b"", "unexpected provider input"
                        self.server.disconnected.set()
                        return
                raise AssertionError("cancelled provider connection did not close")
            assert len(self.server.bodies) == 2, "unexpected inference retry"
            assert FIRST in json.dumps(users) and SECOND in json.dumps(users), users
            assert not any(item.get("role") == "assistant" for item in body["input"]), body["input"]
            payload = ("data: " + json.dumps({"type": "response.completed", "response": {
                "id": "continued-response", "status": "completed", "output": [{
                    "type": "message", "role": "assistant", "content": [{
                        "type": "output_text", "text": ANSWER}]}],
                "usage": {"input_tokens": 2, "output_tokens": 1}}}) + "\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except Exception as error:
            self.server.errors.append(repr(error))


def assert_run_cleanup(snapshot, run_id):
    assert snapshot["pending_cleanup_run"] is None, snapshot
    cleanup = snapshot["cleanup"]
    assert cleanup["run_id"] == run_id and cleanup["phase"] == "observed", cleanup
    assert cleanup["pending"] == [], cleanup
    assert snapshot["session_resources"] == [], snapshot["session_resources"]
    retained = snapshot["retained_cleanup"]
    assert retained["run_ids"] == [] and retained["resources"] == [], retained


def suspended_clean(fixture, session, incarnation):
    fixture.suspended(session)
    wait_for(lambda: not fixture.owned_processes())
    stopped = json.loads((fixture.directory / "sessions" / session / "stopped.json").read_text())
    assert stopped["cleanup_observed"] is True, stopped
    assert stopped["session_id"] == session and stopped["incarnation"] == incarnation, stopped
    return stopped


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    fixture.provider.RequestHandlerClass = Provider
    fixture.provider.started = threading.Event()
    fixture.provider.disconnected = threading.Event()
    fixture.provider.stop = threading.Event()
    fixture.provider.errors = []
    try:
        fixture.start()
        session = fixture.session()
        first = fixture.command(session, fixture.submit(session, FIRST))
        assert first["status"] == "accepted", first
        wait_for(lambda: fixture.provider.started.is_set() or fixture.provider.errors)
        assert not fixture.provider.errors, fixture.provider.errors
        before = fixture.command(session, {"op": "snapshot"})
        assert before["run"]["state"] == "running", before["run"]
        assert before["run"]["run_id"] == first["run_id"]
        incarnation = fixture.request({"op": "inspect", "session_id": session})["incarnation"]
        assert len(fixture.owned_processes()) == 1, "expected an independent voyage process"
        receipt = fixture.command(session, {"op": "cancel", "command_id": str(uuid.uuid4()),
            "expected_revision": before["revision"], "expires_at_ms": int(time.time()*1000) + 60000,
            "run_id": first["run_id"]})
        cancelled = fixture.finished(session)
        assert cancelled["run"]["state"] == "cancelled", cancelled["run"]
        assert_run_cleanup(cancelled, first["run_id"])
        wait_for(fixture.provider.disconnected.is_set)
        stopped = suspended_clean(fixture, session, incarnation)
        cancelled = fixture.command(session, {"op": "snapshot"})
        assert [(m["role"], m["content"]) for m in cancelled["messages"]] == [("user", FIRST)]
        assert [t["phase"] for t in cancelled["turns"]] == ["interrupted"], cancelled["turns"]
        fixture.record("cancelled-and-clean", {"receipt": receipt, "snapshot": cancelled, "stopped": stopped})

        second = fixture.command(session, fixture.submit(session, SECOND))
        assert second["status"] == "accepted" and second["run_id"] != first["run_id"], second
        completed = fixture.finished(session)
        assert completed["run"]["state"] == "completed", completed["run"]
        next_incarnation = fixture.request({"op": "inspect", "session_id": session})["incarnation"]
        assert next_incarnation != incarnation, "continuation should wake a fresh owner"
        stopped = suspended_clean(fixture, session, next_incarnation)
        completed = fixture.command(session, {"op": "snapshot"})
        assert_run_cleanup(completed, second["run_id"])
        assert [(m["role"], m["content"]) for m in completed["messages"]] == [
            ("user", FIRST), ("user", SECOND), ("assistant", ANSWER)]
        assert [t["phase"] for t in completed["turns"]] == ["interrupted", "completed"], completed["turns"]
        assert len(fixture.provider.bodies) == 2, fixture.provider.bodies
        assert not fixture.provider.errors, fixture.provider.errors
        fixture.record("continued-with-retained-conversation", {"snapshot": completed, "stopped": stopped})
    finally:
        fixture.provider.stop.set()
        fixture.close()
        (fixture.root / "provider-errors.json").write_text(json.dumps(fixture.provider.errors, indent=2))
    print("PASS: cancel running inference, observed cleanup, successful next turn and exact retained history")


if __name__ == "__main__":
    main()
