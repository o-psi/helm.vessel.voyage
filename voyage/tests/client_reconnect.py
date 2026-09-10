"""Offline Linux happy path: detach real Helm, finish work, reconnect and continue.

Uses real Vessel-supervised voyage processes and a bounded loopback provider.
No live credentials, full-screen TUI, or network-failure simulation is involved.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid


def wait_for(observe, label, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = observe()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError("timed out: " + label)


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        # Close-delimited SSE permits a real partial response before the gate.
        self.connection.settimeout(10)
        try:
            assert self.path == "/v1/chat/completions", self.path
            size = int(self.headers["Content-Length"])
            assert 0 < size <= 4 * 1024 * 1024, size
            body = json.loads(self.rfile.read(size))
            assert body["stream"] is True
            self.server.requests.append(body)
            step = len(self.server.requests)
            users = [m["content"] for m in body["messages"] if m["role"] == "user"]
            assert step in (1, 2), "unexpected provider replay"
            assert users == self.server.prompts[:step], users
            if step == 2:
                assert any(m["role"] == "assistant" and m.get("content") == self.server.replies[0]
                           for m in body["messages"]), "follow-up lost completed conversation"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            if step == 1:
                self.emit({"content": self.server.prefix})
                self.server.waiting.set()
                assert self.server.release.wait(45), "provider gate timed out"
                self.emit({"content": self.server.suffix})
            else:
                self.emit({"content": self.server.replies[1]})
            self.emit({}, "stop")
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        except Exception as error:
            self.server.errors.append(repr(error))

    def emit(self, delta, reason=None):
        event = {"choices": [{"index": 0, "delta": delta, "finish_reason": reason}]}
        self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
        self.wfile.flush()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        parser.error("run without -O: assertions are the test oracle")
    binaries = args.bin_dir.resolve()
    for name in ("helm", "vessel", "voyage"):
        assert (binaries / name).is_file(), f"missing binary: {binaries / name}"
    root = Path(tempfile.mkdtemp(prefix="voyage-client-reconnect-"))
    print(f"evidence: {root}", flush=True)
    directory = root / "vessel"
    workspace = root / "workspace"
    workspace.mkdir()
    env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
    for key in ("HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
        path = root / key.lower()
        path.mkdir(mode=0o700)
        env[key] = str(path)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    server.daemon_threads = False  # server_close joins the bounded request handlers.
    server.requests, server.errors = [], []
    server.waiting, server.release = threading.Event(), threading.Event()
    marker = uuid.uuid4().hex
    server.prefix = f"Work started {marker}. "
    server.suffix = "Finished while the first Helm client was disconnected."
    server.prompts = ["Complete the reconnect task " + marker,
                      "Recall the completed reconnect task and confirm its marker."]
    server.replies = [server.prefix + server.suffix, "Retained task marker: " + marker]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / "config.toml"
    config.write_text('provider = "openai-chat"\nmodel = "fixture-model"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\naccess = "unrestricted"\ncontext_window = 0\n')
    config.chmod(0o600)
    logs, clients = [], []
    supervisor = None
    session = str(uuid.uuid4())
    evidence = {"session_id": session, "client_exits": [], "binary_sha256": {}}
    for name in ("helm", "vessel", "voyage"):
        with (binaries / name).open("rb") as binary:
            evidence["binary_sha256"][name] = hashlib.file_digest(binary, "sha256").hexdigest()

    def log_file(name):
        handle = (root / name).open("wb")
        logs.append(handle)
        return handle

    def request(value):
        credential = json.loads((directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": value}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as incoming:
            payload = incoming.read(4 * 1024 * 1024 + 1)
        assert 0 < len(payload) <= 4 * 1024 * 1024
        reply = json.loads(payload)
        assert reply["protocol"] == 1 and reply.get("error") is None, reply
        assert not reply.get("outcome_unknown"), reply
        return reply["result"]

    def snapshot(name=None):
        reply = request({"op": "snapshot", "session_id": session})
        assert reply["session_id"] == session, reply
        saved = reply["result"]
        assert saved["session_id"] == session, saved
        if name:
            (root / (name + ".json")).write_text(json.dumps(saved, indent=2))
        return saved

    def inspect():
        result = request({"op": "inspect", "session_id": session})
        assert result["session_id"] == session, result
        return result

    def helm(name, *arguments):
        process = subprocess.Popen([str(binaries / "helm"), "connect", "--directory",
            str(directory), "--no-start", *arguments], env=env, cwd=workspace,
            stdin=subprocess.PIPE, stdout=log_file(name + ".out"), stderr=log_file(name + ".err"))
        clients.append(process)
        return process

    def saw_output(client, name, text):
        assert not server.errors, server.errors
        assert client.poll() is None, (root / (name + ".err")).read_text()
        return text in (root / (name + ".out")).read_text()

    def finished():
        assert not server.errors, server.errors
        saved = snapshot()
        state = saved.get("run", {}).get("state")
        assert state not in ("failed", "cancelled", "cancel_requested"), saved.get("run")
        return saved if state == "completed" and saved.get("pending_cleanup_run") is None else None

    def check_history(saved, turns):
        assert [m["content"] for m in saved["messages"] if m["role"] == "user"] == server.prompts[:turns]
        assert [m["content"] for m in saved["messages"] if m["role"] == "assistant"] == server.replies[:turns]
        assert len(saved["turns"]) == turns
        assert all(t["phase"] == "completed" for t in saved["turns"]), saved["turns"]
        assert len(server.requests) == turns, "reconnect replayed provider work"

    def owned_pids():
        pids = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                argv = (entry / "cmdline").read_bytes().split(b"\0")
                if (argv[:3] == [os.fsencode(binaries / "voyage"), b"serve", b"--directory"]
                        and len(argv) > 3 and Path(os.fsdecode(argv[3])).parent == directory / "sessions"):
                    pids.append(int(entry.name))
            except (FileNotFoundError, ProcessLookupError):
                pass
        return pids

    try:
        supervisor = subprocess.Popen([str(binaries / "vessel"), "local-serve", "--directory",
            str(directory), "--voyage-binary", str(binaries / "voyage")], env=env,
            cwd=workspace, stdin=subprocess.DEVNULL, stdout=log_file("vessel.log"), stderr=subprocess.STDOUT)
        wait_for(lambda: (directory / "process-http.json").exists(), "Vessel discovery")
        request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
                 "workspace": str(workspace), "config_path": str(config)})
        first = helm("first", "run", session, server.prompts[0])
        wait_for(lambda: saw_output(first, "first", server.prefix), "real Helm receives partial output")
        assert server.waiting.is_set() and not server.release.is_set()
        before = snapshot("before-disconnect")
        assert before["run"]["state"] == "running", before["run"]
        owner = inspect()
        pids = owned_pids()
        assert len(pids) == 1, pids
        evidence["running_owner"] = owner
        evidence["running_pid"] = pids[0]
        # SIGINT exercises Helm's documented detach path, not cancellation or a kill.
        first.send_signal(signal.SIGINT)
        assert first.wait(timeout=10) == 0, (root / "first.err").read_text()
        assert f"Detached from voyage {session}" in (root / "first.err").read_text()
        evidence["client_exits"].append({"client": "first", "returncode": first.returncode})
        after = snapshot("after-disconnect")
        assert after["run"]["state"] == "running", after["run"]
        assert after["run"]["run_id"] == before["run"]["run_id"]
        still = inspect()
        assert still["incarnation"] == owner["incarnation"], still
        assert owned_pids() == pids and supervisor.poll() is None
        assert not server.release.is_set() and len(server.requests) == 1
        # The detached client cannot be responsible for processing this response.
        server.release.set()
        wait_for(finished, "completion without Helm")
        wait_for(lambda: inspect()["state"] == "suspended", "idle suspension without Helm")
        completed = snapshot("completed-disconnected")
        check_history(completed, 1)
        assert completed["run"]["run_id"] == before["run"]["run_id"]
        wait_for(lambda: not owned_pids(), "completed owner exits normally")
        # A genuinely new Helm process reconnects and renders retained run output.
        second = helm("reconnected", "chat", session)
        wait_for(lambda: saw_output(second, "reconnected", server.replies[0]), "Helm renders completed output")
        assert f"Voyage {session}." in (root / "reconnected.err").read_text()
        check_history(snapshot("reconnected"), 1)
        second.stdin.write((server.prompts[1] + "\n").encode())
        second.stdin.flush()
        wait_for(lambda: saw_output(second, "reconnected", server.replies[1]), "Helm receives follow-up")
        wait_for(finished, "follow-up completion")
        wait_for(lambda: inspect()["state"] == "suspended", "follow-up suspension")
        check_history(snapshot("follow-up"), 2)
        second.stdin.close()  # EOF also exercises a normal client detach.
        assert second.wait(timeout=10) == 0, (root / "reconnected.err").read_text()
        assert f"Detached from voyage {session}" in (root / "reconnected.err").read_text()
        evidence["client_exits"].append({"client": "reconnected", "returncode": second.returncode})
        assert not server.errors, server.errors
    finally:
        server.release.set()
        try:
            for client in clients:
                if client.poll() is None:
                    client.terminate()
                    try:
                        client.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        client.kill()
                        client.wait(timeout=10)
                        raise AssertionError("Helm required forced cleanup")
                if client.stdin and not client.stdin.closed:
                    client.stdin.close()
            for pid in owned_pids():
                try:
                    descriptor = os.pidfd_open(pid)
                    try:
                        if pid in owned_pids():
                            signal.pidfd_send_signal(descriptor, signal.SIGTERM)
                    finally:
                        os.close(descriptor)
                except ProcessLookupError:
                    pass
            wait_for(lambda: not owned_pids(), "fixture-owned voyage cleanup")
            evidence["remaining_voyage_pids"] = owned_pids()
        finally:
            try:
                if supervisor is not None:
                    supervisor.terminate()
                    try:
                        supervisor.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        supervisor.kill()
                        supervisor.wait(timeout=10)
                        raise AssertionError("Vessel required forced cleanup")
                    evidence["supervisor_exit"] = supervisor.returncode
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                for handle in logs:
                    handle.close()
                (root / "provider-requests.json").write_text(json.dumps(server.requests, indent=2))
                (root / "provider-errors.json").write_text(json.dumps(server.errors, indent=2))
                (root / "lifecycle.json").write_text(json.dumps(evidence, indent=2))
                assert not thread.is_alive(), "provider listener did not stop"
    assert not server.errors, server.errors
    print("PASS: real Helm detach, independent completion, retained reconnect, contextual follow-up and cleanup")


if __name__ == "__main__":
    main()
