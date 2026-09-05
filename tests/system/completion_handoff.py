#!/usr/bin/env python3
"""Offline PTY: drain a live child before same-workspace plain-mode handoff."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Fixture(BaseHTTPRequestHandler):
    child_started = threading.Event()
    root_waiting = threading.Event()
    release_child = threading.Event()
    requests = []

    def log_message(self, *_args):
        pass

    def do_GET(self):
        payload = json.dumps({"data": [{"id": "handoff-fixture"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        users = [item.get("content", "") for item in body["input"] if item.get("role") == "user"]
        prompt = str(users[-1])
        if "hold-handoff-child" in prompt:
            self.child_started.set()
            self.release_child.wait(30)
            text = "child-finished"
            output = []
        elif "after-handoff" in prompt:
            text = "handoff-success"
            output = []
        elif any(item.get("type") == "function_call_output" for item in body["input"]):
            assert self.child_started.wait(10), "child was not dispatched"
            self.root_waiting.set()
            assert self.release_child.wait(30), "root handoff was never released"
            text = "root-ready"
            output = []
        else:
            text = ""
            output = [{"type": "function_call", "id": "fc_spawn", "call_id": "spawn-child",
                       "name": "subagent", "arguments": json.dumps({"action": "spawn", "name": "handoff-child", "task": "hold-handoff-child"})}]
        if not output:
            output = [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}]
        payload = ("data: " + json.dumps({"type": "response.completed", "response": {"output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass  # The handoff deliberately cancels the pending child request.


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-handoff-") as raw:
            root = Path(raw)
            config = root / "config.toml"
            config.write_text(f'provider = "openai-responses"\nmodel = "handoff-fixture"\napi_key_env = "HELM_FIXTURE_KEY"\nbase_url = "http://127.0.0.1:{server.server_port}/v1"\naccess = "read-only"\nprovider_retry_attempts = 1\n')
            env = dict(os.environ, TERM="xterm-256color", HELM_FIXTURE_KEY="offline-key", HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
            pid, master = pty.fork()
            if pid == 0:
                os.execve(str(helm), [str(helm), "--config", str(config), "--workspace", str(root), "chat"], env)
            output = bytearray()
            reaped = False

            def drain():
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        pass

            def wait_until(predicate, description):
                deadline = time.monotonic() + 20
                while not predicate():
                    assert time.monotonic() < deadline, (description, bytes(output[-5000:]))
                    drain()

            def saved():
                paths = list((root / "data/helm/sessions").glob("*.json"))
                return json.loads(paths[0].read_text()) if paths else None

            try:
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
                wait_until(lambda: b"HELM" in output, "initial TUI")
                os.write(master, b"start-handoff\r")
                wait_until(lambda: Fixture.root_waiting.is_set() and Fixture.child_started.is_set() and saved(), "root and child active before handoff")
                before = saved()
                assert Fixture.child_started.is_set()
                # Active Enter is steering; Escape is the explicit cancellation route.
                os.write(master, b"\x1b")
                wait_until(lambda: saved() and saved().get("run_summaries") and saved()["run_summaries"][0]["phase"] == "interrupted", "root cancellation and owned shutdown")
                before = saved()
                os.write(master, b"/plain\r")
                wait_until(lambda: b"Type /help for commands." in output, "plain child startup")
                os.write(master, b"after-handoff\n")
                wait_until(lambda: saved() and any(m["content"] == "handoff-success" for m in saved()["messages"]), "successful resumed turn")
                after = saved()
                assert before["id"] == after["id"]
                assert after["messages"][:len(before["messages"])] == before["messages"]
                assert after["run_summaries"][0]["phase"] == "interrupted"
                assert after["run_summaries"][-1]["phase"] == "completed"
                assert len(after["completion_runs"]) == len(before["completion_runs"]) + 1
                assert b"runtime is busy" not in output
                archives = list((root / "data/helm/subagents").rglob("*.json"))
                assert any('"cancelled"' in path.read_text() for path in archives), archives
                os.write(master, b"/exit\n")
                def exited():
                    nonlocal reaped
                    done, status = os.waitpid(pid, os.WNOHANG)
                    if done:
                        reaped = True
                        assert os.waitstatus_to_exitcode(status) == 0
                    return reaped
                wait_until(exited, "parent and child clean exit")
                print("completion handoff: live child cancelled, writer lease released, session resumed and new run persisted")
            finally:
                Fixture.release_child.set()
                if not reaped:
                    os.kill(pid, signal.SIGKILL)
                    os.waitpid(pid, 0)
                os.close(master)
    finally:
        Fixture.release_child.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
