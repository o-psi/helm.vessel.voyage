"""Offline Linux happy path: two voyages overlapping useful file work.

Runs real Vessel-supervised voyage processes against a scripted local provider.
Does not test model intelligence, real provider access, or Helm's terminal UI.
"""
import argparse
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


def wait_for(observe, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("timed out waiting for runtime state")


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            assert self.path == "/v1/chat/completions", self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert body["stream"] is True
            self.server.requests.append(body)
            messages = body["messages"]
            users = [m["content"] for m in messages if m["role"] == "user"]
            assert len(users) == 1, users
            label = next(k for k, v in self.server.prompts.items() if v == users[0])
            with self.server.lock:
                self.server.steps[label] += 1
                step = self.server.steps[label]
            if step == 1:
                names = {t["function"]["name"] for t in body["tools"]}
                assert "write_file" in names, names
                delta = self.tool("write-" + label, "write_file", {
                    "path": label + ".txt", "content": self.server.contents[label]})
            elif step == 2:
                result = messages[-1]
                assert result["role"] == "tool" and result["tool_call_id"] == "write-" + label, result
                assert (self.server.workspace / (label + ".txt")).read_text() == self.server.contents[label]
                # Hold completion only AFTER useful native tool work. Both must get here.
                self.server.arrived[label].set()
                assert self.server.release.wait(30), "overlap barrier timed out"
                delta = {"content": self.server.replies[label]}
            else:
                raise AssertionError(f"unexpected provider request {label}:{step}")
            reason = "tool_calls" if "tool_calls" in delta else "stop"
            events = [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                      {"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]}]
            payload = ("".join("data: " + json.dumps(event) + "\n\n" for event in events)
                       + "data: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
        except Exception as error:
            self.server.errors.append(f"request {len(self.server.requests)}: {error!r}")
            payload = json.dumps({"error": {"message": "fixture assertion failed"}}).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    @staticmethod
    def tool(call_id, name, arguments):
        return {"tool_calls": [{"index": 0, "id": call_id, "type": "function",
                                "function": {"name": name, "arguments": json.dumps(arguments)}}]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    binaries = args.bin_dir.resolve()
    for name in ("vessel", "voyage"):
        assert (binaries / name).is_file(), f"missing binary: {binaries / name}"
    assert Path("/proc").is_dir() and hasattr(os, "pidfd_open"), "requires Linux with pidfds"
    root = Path(tempfile.mkdtemp(prefix="voyage-two-voyages-"))
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
    server.requests, server.errors = [], []
    labels = ("orchard", "harbor")
    server.workspace = workspace
    server.lock = threading.Lock()
    server.steps = dict.fromkeys(labels, 0)
    server.arrived = {label: threading.Event() for label in labels}
    server.release = threading.Event()
    server.contents = {label: label + " report " + uuid.uuid4().hex + "\n" for label in labels}
    server.prompts = {label: "Write " + label + ".txt containing exactly: " + server.contents[label]
                      for label in labels}
    server.replies = {label: "Saved the " + label + " report." for label in labels}
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / "config.toml"
    config.write_text('provider = "openai-chat"\nmodel = "fixture-model"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\naccess = "unrestricted"\ncontext_window = 0\n')
    config.chmod(0o600)
    log = (root / "vessel.log").open("wb")
    supervisor = None
    sessions = {label: str(uuid.uuid4()) for label in labels}

    def request(value):
        credential = json.loads((directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": value}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as incoming:
            reply = json.load(incoming)
        assert reply.get("error") is None, reply
        return reply["result"]

    def command(session, value):
        reply = request({**value, "session_id": session})
        assert reply.get("error") is None, reply
        return reply["result"]

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

    def signal_owned(sig):
        for pid in owned_pids():
            try:
                descriptor = os.pidfd_open(pid)
                try:
                    # Recheck ownership after opening a stable process handle.
                    if pid in owned_pids():
                        signal.pidfd_send_signal(descriptor, sig)
                finally:
                    os.close(descriptor)
            except ProcessLookupError:
                pass

    try:
        supervisor = subprocess.Popen([str(binaries / "vessel"), "local-serve", "--directory",
            str(directory), "--voyage-binary", str(binaries / "voyage")], env=env,
            cwd=workspace, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / "process-http.json").exists())
        for session in sessions.values():
            request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
                     "workspace": str(workspace), "config_path": str(config)})
        for label, session in sessions.items():
            snapshot = command(session, {"op": "snapshot"})
            command(session, {"op": "submit", "command_id": str(uuid.uuid4()),
                     "expected_revision": snapshot["revision"],
                     "expires_at_ms": int(time.time() * 1000) + 60000, "prompt": server.prompts[label]})

        def both_worked():
            assert not server.errors, server.errors
            return all(event.is_set() for event in server.arrived.values())

        wait_for(both_worked, timeout=25)
        pids = owned_pids()
        assert len(pids) == 2 and len(set(pids)) == 2, pids
        overlaps = {}
        for label, session in sessions.items():
            snapshot = command(session, {"op": "snapshot"})
            assert snapshot["run"]["state"] == "running", snapshot["run"]
            assert (workspace / (label + ".txt")).read_text() == server.contents[label]
            overlaps[label] = {"session_id": session, "snapshot": snapshot}
        (root / "overlap.json").write_text(json.dumps({"pids": pids, "voyages": overlaps}, indent=2))
        server.release.set()
        for label, session in sessions.items():
            def finished():
                assert not server.errors, server.errors
                saved = command(session, {"op": "snapshot"})
                state = saved.get("run", {}).get("state")
                assert state not in ("failed", "cancelled"), saved.get("run")
                return saved if state == "completed" and saved.get("pending_cleanup_run") is None else None

            wait_for(finished)
            wait_for(lambda: request({"op": "inspect", "session_id": session})["state"] == "suspended")
            # Suspension retires the executor: these reads verify separately retained conversations.
            snapshot = command(session, {"op": "snapshot"})
            (root / (label + "-retained.json")).write_text(json.dumps(snapshot, indent=2))
            assert snapshot["session_id"] == session
            messages = snapshot["messages"]
            assert [m["content"] for m in messages if m["role"] == "user"] == [server.prompts[label]]
            assert [m["content"] for m in messages if m["role"] == "assistant" and not m.get("tool_calls")] == [server.replies[label]]
            tools = [m for m in messages if m["role"] == "tool"]
            assert [m["tool_call_id"] for m in tools] == ["write-" + label], tools
            assert all(m["tool_outcome"]["execution"] == "succeeded" for m in tools), tools
            assert len(snapshot["turns"]) == 1 and snapshot["turns"][0]["phase"] == "completed"
            peer = next(other for other in labels if other != label)
            assert server.contents[peer].strip() not in json.dumps(messages), "peer conversation leaked"
            assert (workspace / (label + ".txt")).read_text() == server.contents[label]
        assert server.steps == dict.fromkeys(labels, 2), server.steps
        assert not server.errors, server.errors
    finally:
        server.release.set()
        try:
            signal_owned(signal.SIGTERM)
            try:
                wait_for(lambda: not owned_pids(), timeout=10)
            except AssertionError:
                signal_owned(signal.SIGKILL)
                wait_for(lambda: not owned_pids(), timeout=10)
                raise AssertionError("voyage required forced cleanup")
        finally:
            try:
                if supervisor is not None:
                    supervisor.terminate()
                    try:
                        supervisor.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        supervisor.kill()
                        supervisor.wait(timeout=10)
                        raise AssertionError("supervisor required forced cleanup")
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                log.close()
                (root / "cleanup.json").write_text(json.dumps({
                    "remaining_owned_voyage_pids": owned_pids(),
                    "supervisor_returncode": supervisor.poll() if supervisor is not None else None,
                    "provider_thread_alive": thread.is_alive()}, indent=2))
                (root / "provider-requests.json").write_text(json.dumps(server.requests, indent=2))
                (root / "provider-errors.json").write_text(json.dumps(server.errors, indent=2))
    print("PASS: two live voyages overlap actual file writes, complete, retain separate conversations, and clean up")


if __name__ == "__main__":
    main()
