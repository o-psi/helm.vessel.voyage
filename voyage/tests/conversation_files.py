"""Offline Linux happy path: conversation, native file tools, and retained context.

Runs real Vessel-supervised voyage processes against a scripted local provider.
Does not test model intelligence, real provider access, or Helm's terminal UI.
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
            step = len(self.server.requests)
            messages = body["messages"]
            users = [m["content"] for m in messages if m["role"] == "user"]
            assert users[0] == self.server.prompts[0], users
            if step == 1:
                names = {t["function"]["name"] for t in body["tools"]}
                assert {"read_file", "apply_patch"} <= names, names
                delta = self.tool("read-notes", "read_file", {"path": "notes.txt"})
            elif step == 2:
                result = messages[-1]
                assert result["role"] == "tool" and result["tool_call_id"] == "read-notes", result
                assert self.server.original in result["content"], result
                digest = hashlib.sha256(self.server.original.encode()).hexdigest()
                assert "sha256: " + digest in result["content"], result
                delta = self.tool("edit-notes", "apply_patch", {
                    "path": "notes.txt", "base_sha256": digest,
                    "patch": "--- notes.txt\n+++ notes.txt\n@@ -1,3 +1,3 @@\n"
                             " Shopping list\n-milk\n+oat milk\n " + self.server.marker + "\n"})
            elif step == 3:
                result = messages[-1]
                assert result["role"] == "tool" and result["tool_call_id"] == "edit-notes", result
                assert self.server.notes.read_text() == self.server.expected
                delta = {"content": self.server.replies[0]}
            elif step == 4:
                assert users == self.server.prompts, users
                assert any(m["role"] == "assistant" and m.get("content") == self.server.replies[0]
                           for m in messages), "follow-up omitted the first reply"
                for call_id in ("read-notes", "edit-notes"):
                    assert any(m["role"] == "tool" and m.get("tool_call_id") == call_id
                               for m in messages), "follow-up omitted tool history"
                delta = self.tool("verify-notes", "read_file", {"path": "notes.txt"})
            elif step == 5:
                result = messages[-1]
                assert result["role"] == "tool" and result["tool_call_id"] == "verify-notes", result
                assert self.server.expected in result["content"], result
                delta = {"content": self.server.replies[1]}
            else:
                raise AssertionError(f"unexpected provider request {step}")
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
    root = Path(tempfile.mkdtemp(prefix="voyage-conversation-files-"))
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
    server.marker = "keep-" + uuid.uuid4().hex
    server.original = "Shopping list\nmilk\n" + server.marker + "\n"
    server.expected = "Shopping list\noat milk\n" + server.marker + "\n"
    server.notes = workspace / "notes.txt"
    server.notes.write_text(server.original)
    server.prompts = ["Read notes.txt and change milk to oat milk, preserving the other lines.",
                      "What did you change earlier? Read the file again and tell me the preserved marker."]
    server.replies = ["Changed milk to oat milk and kept the other lines.",
                      "Earlier I changed milk to oat milk. The preserved marker is " + server.marker + "."]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / "config.toml"
    config.write_text('provider = "openai-chat"\nmodel = "fixture-model"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\naccess = "unrestricted"\ncontext_window = 0\n')
    config.chmod(0o600)
    log = (root / "vessel.log").open("wb")
    supervisor = None
    session = str(uuid.uuid4())

    def request(value):
        credential = json.loads((directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": value}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as incoming:
            reply = json.load(incoming)
        assert reply.get("error") is None, reply
        return reply["result"]

    def command(value):
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

    try:
        supervisor = subprocess.Popen([str(binaries / "vessel"), "local-serve", "--directory",
            str(directory), "--voyage-binary", str(binaries / "voyage")], env=env,
            cwd=workspace, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / "process-http.json").exists())
        request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
                 "workspace": str(workspace), "config_path": str(config)})
        for turn, prompt in enumerate(server.prompts):
            snapshot = command({"op": "snapshot"})
            command({"op": "submit", "command_id": str(uuid.uuid4()),
                     "expected_revision": snapshot["revision"],
                     "expires_at_ms": int(time.time() * 1000) + 60000, "prompt": prompt})

            def finished():
                assert not server.errors, server.errors
                saved = command({"op": "snapshot"})
                state = saved.get("run", {}).get("state")
                assert state not in ("failed", "cancelled"), saved.get("run")
                return saved if state == "completed" and saved.get("pending_cleanup_run") is None else None

            snapshot = wait_for(finished)
            wait_for(lambda: request({"op": "inspect", "session_id": session})["state"] == "suspended")
            # Read again after suspension, from retained state rather than only the live response.
            snapshot = command({"op": "snapshot"})
            (root / f"turn-{turn + 1}.json").write_text(json.dumps(snapshot, indent=2))
            messages = snapshot["messages"]
            assert [m["content"] for m in messages if m["role"] == "user"] == server.prompts[:turn + 1]
            assert [m["content"] for m in messages if m["role"] == "assistant" and not m.get("tool_calls")] == server.replies[:turn + 1]
            tools = [m for m in messages if m["role"] == "tool"]
            expected_ids = ["read-notes", "edit-notes"] + (["verify-notes"] if turn else [])
            assert [m["tool_call_id"] for m in tools] == expected_ids, tools
            assert all(m["tool_outcome"]["execution"] == "succeeded" for m in tools), tools
            assert all(t["phase"] == "completed" for t in snapshot["turns"]), snapshot["turns"]
            assert len(snapshot["turns"]) == turn + 1
            assert server.notes.read_text() == server.expected
            assert len(server.requests) == (3 if turn == 0 else 5)
        assert not server.errors, server.errors
    finally:
        try:
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
            wait_for(lambda: not owned_pids())
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
                (root / "provider-requests.json").write_text(json.dumps(server.requests, indent=2))
                (root / "provider-errors.json").write_text(json.dumps(server.errors, indent=2))
    print("PASS: conversation, file read/patch, unchanged surrounding lines, retained follow-up context and cleanup")


if __name__ == "__main__":
    main()
