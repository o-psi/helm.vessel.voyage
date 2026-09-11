"""Focused offline #172 checks against actual Vessel/voyage development binaries."""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid


def wait_for(observe, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = observe()
        if value:
            return value
        time.sleep(0.04)
    raise AssertionError("timed out waiting for runtime state")


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.bodies.append(body)
        output = [{"type": "message", "role": "assistant", "content": [
            {"type": "output_text", "text": "Fixture finished."}]}]
        if "approval-fixture" in json.dumps(body.get("input", [])) and not any(
                item.get("type") == "function_call_output" for item in body.get("input", [])):
            output = [{"type": "function_call", "call_id": "fixture-write", "name": "write_file",
                       "arguments": json.dumps({"path": "approval-output.txt", "content": "unauthorized"})}]
        payload = ("data: " + json.dumps({"type": "response.completed", "response": {
            "id": "fixture-response", "status": "completed", "output": output,
            "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class Fixture:
    def __init__(self, binaries):
        self.binaries = binaries
        self.root = Path(tempfile.mkdtemp(prefix="vdr-"))
        print(f"evidence: {self.root}", flush=True)
        self.directory = self.root / "vessel"
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        self.env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
        for key in ("HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
            path = self.root / key.lower()
            path.mkdir(mode=0o700)
            self.env[key] = str(path)
        tokens = Path(self.env["XDG_DATA_HOME"]) / "helm/chatgpt-oauth.json"
        tokens.parent.mkdir(mode=0o700)
        tokens.write_text(json.dumps({"access_token": "synthetic-access", "refresh_token": "synthetic-refresh",
            "account_id": "synthetic-account", "expires_at": int(time.time()) + 3600}))
        tokens.chmod(0o600)
        self.provider = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        self.provider.bodies = []
        self.thread = threading.Thread(target=self.provider.serve_forever, daemon=True)
        self.thread.start()
        self.log = (self.root / "vessel.log").open("ab")
        self.supervisor = None
        self.results = []
        self.sessions = []

    def start(self):
        self.supervisor = subprocess.Popen([str(self.binaries / "vessel"), "local-serve", "--directory",
            str(self.directory), "--voyage-binary", str(self.binaries / "voyage")], env=self.env,
            cwd=self.workspace, stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.log)
        def ready():
            try:
                return self.request({"op": "catalogue"}) is not None
            except (OSError, ValueError):
                return False
        wait_for(ready)

    def request(self, command, allow_error=False):
        credential = json.loads((self.directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": command}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=20) as incoming:
            value = json.load(incoming)
        if allow_error:
            return value
        assert value.get("error") is None, value
        return value["result"]

    def command(self, session, command, allow_error=False):
        exact = command["op"] in ("execute_tool", "terminal", "cancel", "steer", "respond")
        identity = ({"incarnation": self.request({"op": "inspect", "session_id": session})["incarnation"]}
                    if exact else {})
        response = self.request({**command, "session_id": session, **identity}, allow_error=True)
        if response.get("error"):
            assert allow_error, response
            return response
        reply = response["result"]
        assert set(reply) == {"session_id", "incarnation", "result"}, reply
        assert reply["session_id"] == session, reply
        if allow_error:
            return {"result": reply["result"], "error": None, "outcome_unknown": False}
        return reply["result"]

    def session(self, approval=False):
        session = str(uuid.uuid4())
        self.sessions.append(session)
        config = self.root / (session + ".toml")
        config.write_text('provider = "chatgpt-oauth"\nmodel = "fixture-model"\n'
            f'chatgpt_base_url = "http://127.0.0.1:{self.provider.server_port}"\n'
            'provider_retry_attempts = 1\ncontext_window = 0\ncommand_timeout_secs = 2\n'
            f'access = "{"approval" if approval else "read-only"}"\n')
        config.chmod(0o600)
        self.request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
            "workspace": str(self.workspace), "config_path": str(config)})
        return session

    def submit(self, session, prompt):
        return {"op": "submit", "command_id": str(uuid.uuid4()),
            "expected_revision": self.command(session, {"op": "snapshot"})["revision"],
            "expires_at_ms": int(time.time()*1000) + 60000, "prompt": prompt}

    def finished(self, session):
        def observe():
            snapshot = self.command(session, {"op": "snapshot"})
            run = snapshot.get("run")
            return snapshot if run and run["state"] in ("completed", "failed", "cancelled") and snapshot["pending_cleanup_run"] is None else None
        return wait_for(observe)

    def suspended(self, session):
        wait_for(lambda: self.request({"op": "inspect", "session_id": session})["state"] == "suspended")

    def record(self, name, value):
        self.results.append(name)
        (self.root / (name + ".json")).write_text(json.dumps(value, indent=2))
        print("PASS:", name, flush=True)

    def owned_processes(self):
        owned = {}
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            try:
                argv = (path / "cmdline").read_bytes().split(b"\0")
                if (argv[:3] == [os.fsencode(self.binaries / "voyage"), b"serve", b"--directory"]
                        and len(argv) > 3 and Path(os.fsdecode(argv[3])).parent == self.directory / "sessions"):
                    owned[int(path.name)] = argv
            except (FileNotFoundError, ProcessLookupError):
                pass
        return owned

    def close(self):
        # Match exact fixture binary and exact per-session directory before signaling.
        # A signal is a request; verify process disappearance and durable cleanup too.
        try:
            for pid, argv in self.owned_processes().items():
                try:
                    descriptor = os.pidfd_open(pid)
                    try:
                        if self.owned_processes().get(pid) == argv:
                            signal.pidfd_send_signal(descriptor, signal.SIGTERM)
                    finally:
                        os.close(descriptor)
                except ProcessLookupError:
                    pass
            wait_for(lambda: not self.owned_processes(), timeout=10)

            def observed():
                evidence = {}
                for session in self.sessions:
                    directory = self.directory / "sessions" / session
                    try:
                        stopped = json.loads((directory / "stopped.json").read_text())
                        registration = json.loads((directory / "registration.json").read_text())
                    except (FileNotFoundError, ValueError):
                        return False
                    if (stopped.get("cleanup_observed") is not True
                            or stopped.get("session_id") != session
                            or stopped.get("incarnation") != registration.get("incarnation")):
                        return False
                    evidence[session] = stopped
                return evidence or not self.sessions
            cleanup = wait_for(observed, timeout=10)
            self.record("fixture-cleanup-observed", cleanup)
        finally:
            if self.supervisor is not None:
                self.supervisor.terminate()
                self.supervisor.wait(timeout=10)
            self.provider.shutdown()
            self.provider.server_close()
            self.thread.join(timeout=5)
            self.log.close()
            (self.root / "provider-requests.json").write_text(json.dumps(self.provider.bodies, indent=2))
            (self.root / "results.json").write_text(json.dumps(self.results, indent=2))


def stopped_resolution(fixture):
    session = fixture.session()
    original = fixture.submit(session, "never dispatched before explicit stop")
    info = fixture.request({"op": "inspect", "session_id": session})
    fixture.request({"op": "stop", "session_id": session, "incarnation": info["incarnation"]})
    wait_for(lambda: fixture.request({"op": "inspect", "session_id": session})["state"] == "stopped")
    directory = fixture.directory / "sessions" / session
    before = json.loads((directory / "registration.json").read_text())
    resolve = {"op": "resolve", "command_id": original["command_id"], "original": original}
    receipt = fixture.command(session, resolve)
    assert receipt["status"] == "not_admitted", receipt
    assert fixture.command(session, resolve) == receipt
    assert json.loads((directory / "registration.json").read_text()) == before
    assert not (directory / "runtime.sock").exists()
    fixture.record("stopped-owner-resolution-without-restart", receipt)


def scoped_resolution(fixture):
    session = fixture.session()
    original = fixture.submit(session, "scoped never dispatched")
    resolve = {"op": "resolve", "command_id": original["command_id"], "original": original}

    def grant(rights):
        return fixture.request({"op": "grant", "command_id": str(uuid.uuid4()),
            "grant_id": str(uuid.uuid4()), "principal_id": str(uuid.uuid4()),
            "session_id": session, "workspace": str(fixture.workspace), "rights": rights,
            "expires_at_ms": int(time.time() * 1000) + 60_000,
            "endpoint": "http://127.0.0.1"})

    def scoped(credential, command):
        return fixture.request({"op": "granted", "grant_id": credential["grant_id"],
            "token": credential["token"], "command": {**command, "session_id": session}})

    def denied(credential, command):
        try:
            response = scoped(credential, command)
        except AssertionError:
            return  # Supervisor rejected the scoped operation before dispatch.
        assert response.get("error"), response

    observer = grant(["observe", "history"])
    denied(observer, resolve)
    assert fixture.command(session, {"op": "receipt", "command_id": original["command_id"]})["status"] == "unknown"
    executor = grant(["observe", "history", "execute"])
    denied(executor, {"op": "resolve", "command_id": original["command_id"]})
    resolved = scoped(executor, resolve)
    assert resolved.get("error") is None and resolved["result"]["status"] == "not_admitted", resolved
    denied(grant(["observe", "history", "execute"]), resolve)
    fixture.request({"op": "revoke_grant", "command_id": str(uuid.uuid4()),
                     "grant_id": executor["grant_id"], "expected_revision": 1})
    another = fixture.submit(session, "revoked never dispatched")
    denied(executor, {"op": "resolve", "command_id": another["command_id"], "original": another})
    assert fixture.command(session, {"op": "receipt", "command_id": another["command_id"]})["status"] == "unknown"
    fixture.record("scoped-resolution-rights-and-principal", {"observe_cannot_reserve": True,
        "original_required": True, "principal_bound": True, "revoked_cannot_reserve": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    try:
        fixture.start()
        scoped_resolution(fixture)
        stopped_resolution(fixture)
        session = fixture.session()
        absent = fixture.submit(session, "never dispatched")
        resolve = {"op": "resolve", "command_id": absent["command_id"], "original": absent}
        mismatched = {**resolve, "original": {**absent, "command_id": str(uuid.uuid4())}}
        rejected = fixture.command(session, mismatched, allow_error=True)
        assert rejected.get("error"), rejected
        receipt = fixture.command(session, resolve)
        assert receipt["status"] == "not_admitted", receipt
        conflicting = {**resolve, "original": {**absent, "prompt": "changed payload"}}
        rejected = fixture.command(session, conflicting, allow_error=True)
        assert rejected.get("error"), rejected
        assert fixture.command(session, resolve) == receipt
        fixture.record("resolution-rejects-id-and-payload-conflicts", rejected)
        late = fixture.command(session, absent, allow_error=True)
        assert late.get("error") or late["result"].get("status") == "not_admitted", late
        assert not fixture.command(session, {"op": "snapshot"}).get("run")
        fixture.suspended(session)
        assert fixture.command(session, resolve) == receipt
        fixture.supervisor.terminate()
        fixture.supervisor.wait(timeout=10)
        fixture.start()
        assert fixture.command(session, resolve) == receipt
        late = fixture.command(session, absent, allow_error=True)
        assert late.get("error") and late["result"]["status"] == "not_admitted", late
        assert not fixture.command(session, {"op": "snapshot"}).get("run")
        fixture.record("not-admitted-fenced-across-suspension-and-vessel-restart", receipt)

        session = fixture.session()
        admitted = fixture.submit(session, "accepted despite discarded response")
        # Hold the socket without reading its reply; observe admission independently,
        # then close it. The submitting client never learns the acceptance response.
        registration = json.loads((fixture.directory / "sessions" / session / "registration.json").read_text())
        message = {"protocol": 1, "session_id": session, "incarnation": registration["incarnation"],
                   "token": registration["token"], "command": admitted}
        with socket.socket(socket.AF_UNIX) as connection:
            connection.connect(str(fixture.directory / "sessions" / session / "runtime.sock"))
            encoded = json.dumps(message).encode()
            connection.sendall(struct.pack(">I", len(encoded)) + encoded)
            snapshot = fixture.finished(session)
        resolve_admitted = {"op": "resolve", "command_id": admitted["command_id"], "original": admitted}
        accepted = fixture.command(session, resolve_admitted)
        assert accepted["status"] == "accepted", accepted
        duplicate = fixture.command(session, admitted)
        assert duplicate["run_id"] == accepted["run_id"] and duplicate["duplicate"] is True, (duplicate, accepted)
        assert len(fixture.command(session, {"op": "snapshot"})["turns"]) == 1
        fixture.suspended(session)
        assert fixture.command(session, resolve_admitted) == accepted
        fixture.supervisor.terminate()
        fixture.supervisor.wait(timeout=10)
        fixture.start()
        assert fixture.command(session, resolve_admitted) == accepted
        assert len(fixture.command(session, {"op": "snapshot"})["turns"]) == 1
        fixture.record("lost-acceptance-response-exactly-once", accepted)

        timings = []
        for index in range(5):
            session = fixture.session()
            original = fixture.submit(session, f"poll-race-{index}")
            cursor = fixture.command(session, {"op": "events", "after": 0, "limit": 128, "wait_ms": 0})["cursor"]
            errors = []
            def poll():
                try:
                    fixture.command(session, {"op": "events", "after": cursor, "limit": 128, "wait_ms": 10000})
                except Exception as error:
                    errors.append(str(error))
            observer = threading.Thread(target=poll)
            observer.start()
            time.sleep(2.1)
            started = time.monotonic()
            received = fixture.command(session, original)
            elapsed = time.monotonic() - started
            assert received["status"] == "accepted", received
            assert elapsed < 4, elapsed
            duplicate = fixture.command(session, original)
            assert duplicate["run_id"] == received["run_id"] and duplicate["duplicate"] is True
            snapshot = fixture.finished(session)
            assert len(snapshot["turns"]) == 1
            observer.join(timeout=15)
            assert not observer.is_alive()
            assert not errors, errors
            timings.append(elapsed)
        fixture.record("idle-suspension-overlapping-event-poll", timings)

        for mode in ("expired", "denied", "cancelled"):
            session = fixture.session(approval=True)
            fixture.command(session, fixture.submit(session, "approval-fixture " + mode))
            decisions = wait_for(lambda: fixture.command(session, {"op": "decisions"}))
            decision = decisions[0]
            if mode != "expired":
                snapshot = fixture.command(session, {"op": "snapshot"})
                command = {"op": "respond" if mode == "denied" else "cancel", "command_id": str(uuid.uuid4()),
                    "expected_revision": snapshot["revision"], "expires_at_ms": int(time.time()*1000)+60000,
                    "run_id": decision["run_id"]}
                if mode == "denied":
                    command.update(decision_id=decision["decision_id"], response="denied")
                fixture.command(session, command)
            snapshot = fixture.finished(session)
            serialized = json.dumps(snapshot)
            if mode == "expired":
                assert "approval expired without a response" in serialized, serialized
                assert "user declined approval" not in serialized
            elif mode == "denied":
                assert "user declined approval" in serialized, serialized
            else:
                assert snapshot["run"]["state"] == "cancelled", snapshot
                assert "user declined approval" not in serialized
            assert not (fixture.workspace / "approval-output.txt").exists()
            fixture.record("approval-" + mode, snapshot)
    finally:
        fixture.close()


if __name__ == "__main__":
    main()
