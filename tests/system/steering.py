#!/usr/bin/env python3
"""Offline Responses provider plus real POSIX TUI: steering, tools, failure, cancel."""
from __future__ import annotations

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
    requests: list[dict] = []
    entered = threading.Event()
    release = threading.Event()
    case = "fifo"

    def log_message(self, *_args):
        pass

    def do_GET(self):
        # Automatic-title discovery must stay independent of the steering fixture.
        payload = b'{"data":[]}'
        try:
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        first = len(self.requests) == 1
        self.entered.set()
        if first and self.case != "tool":
            assert self.release.wait(15), "fixture release timeout"
        if first and self.case == "failure":
            self.send_error(500, "injected failure")
            return
        if first and self.case == "tool":
            output = [{"type": "function_call", "id": "fc_wait", "call_id": "wait-1",
                       "name": "shell", "arguments": json.dumps({"command": "while [ ! -f release-tool ]; do sleep 0.05; done"})}]
        else:
            text = "initial answer" if first else "steered answer 日本語"
            output = [{"type": "message", "id": "msg_answer", "role": "assistant",
                       "content": [{"type": "output_text", "text": text}]}]
        payload = ("data: " + json.dumps({"type": "response.completed", "response": {
            "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        try:
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            assert self.case == "cancel"


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for case in ("fifo", "tool", "failure", "cancel"):
            Fixture.case = case
            Fixture.requests = []
            Fixture.entered.clear()
            Fixture.release.clear()
            with tempfile.TemporaryDirectory(prefix="helm-steering-") as raw:
                root = Path(raw)
                config = root / "config.toml"
                config.write_text(f'''provider = "openai-responses"
model = "fixture"
api_key_env = "HELM_FIXTURE_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
command_timeout_secs = 10
access = "unrestricted"
''')
                env = os.environ.copy()
                env.update(TERM="xterm-256color", HELM_FIXTURE_KEY="offline-fixture-key",
                           HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                           XDG_DATA_HOME=str(root / "data"))
                command = [str(helm), "--config", str(config), "--workspace", str(root), "chat"]
                pid, master = pty.fork()
                if pid == 0:
                    os.execve(str(helm), command, env)
                output = bytearray()
                reaped = False

                def saved():
                    paths = list((root / "data" / "helm" / "sessions").glob("*.json"))
                    return json.loads(paths[0].read_text()) if paths else {"messages": []}

                def wait_until(predicate, description):
                    deadline = time.monotonic() + 15
                    while not predicate():
                        assert time.monotonic() < deadline, (case, description, saved(), bytes(output[-3000:]))
                        if select.select([master], [], [], 0.05)[0]:
                            output.extend(os.read(master, 65536))

                try:
                    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
                    wait_until(lambda: b"HELM" in output, "startup")
                    os.write(master, b"original task\r")
                    wait_until(Fixture.entered.is_set, "first provider request")
                    if case == "tool":
                        wait_until(lambda: b"shell" in output, "tool execution")
                    # Resize and paste Unicode/newlines while the run is active.
                    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 18, 48, 0, 0))
                    os.kill(pid, signal.SIGWINCH)
                    time.sleep(0.2)  # let Crossterm finish resize before the paste sequence
                    os.write(master, "\x1b[200~first steering\n日本語\x1b[201~\r".encode())
                    wait_until(lambda: sum("steering" in m for m in saved()["messages"]) == 1, "first durable steering")
                    os.write(master, b"/not-a-command second steering\r")
                    wait_until(lambda: sum("steering" in m for m in saved()["messages"]) == 2, "second durable steering")
                    assert all(m["steering"]["status"] == "queued" for m in saved()["messages"] if "steering" in m)
                    # Persistence intentionally precedes enqueue. A fresh resize
                    # redraw can only be processed after the Enter handler has
                    # enqueued the second message and set its acknowledgement.
                    # Waiting for disk alone can release the provider too early.
                    acknowledgement_start = len(output)
                    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 19, 49, 0, 0))
                    wait_until(lambda: b"Steering queued" in output[acknowledgement_start:],
                               "second steering enqueue acknowledgement")
                    # Buffered bytes from the prior 18-row frame are not a
                    # barrier. The new 19-row frame ends with this empty-composer
                    # cursor on row 15, proving the resize was processed.
                    wait_until(lambda: b"\x1b[?25h\x1b[15;1H" in output[acknowledgement_start:],
                               "post-enqueue resized frame completion")
                    if case == "cancel":
                        os.write(master, b"\x1b")
                        wait_until(lambda: any(m.get("steering", {}).get("status") == "not_applied" for m in saved()["messages"]), "cancel delivery outcome")
                    Fixture.release.set()
                    (root / "release-tool").touch()
                    if case in ("failure", "cancel"):
                        wait_until(lambda: all(m.get("steering", {}).get("status") == "not_applied" for m in saved()["messages"] if "steering" in m), "failed delivery outcome")
                        assert len(Fixture.requests) == 1
                    else:
                        wait_until(lambda: any(m["role"] == "assistant" and m["content"] == "steered answer 日本語" for m in saved()["messages"]), "canonical completion")
                        assert len(Fixture.requests) == 2
                        request = Fixture.requests[1]
                        users = [item["content"] for item in request["input"] if item.get("role") == "user"]
                        assert users == ["original task", "first steering\n日本語", "/not-a-command second steering"], users
                        assert request["tools"] == Fixture.requests[0]["tools"]
                        assert '"steering"' not in json.dumps(request), "local receipts leaked into provider JSON"
                    messages = saved()["messages"]
                    assert [m["content"] for m in messages if m["role"] == "user"] == ["original task", "first steering\n日本語", "/not-a-command second steering"]
                    receipts = [m["steering"] for m in messages if "steering" in m]
                    assert len({r["id"] for r in receipts}) == 2
                    assert all(r["status"] == ("applied" if case in ("fifo", "tool") else "not_applied") for r in receipts)
                    os.write(master, b"\x11")
                    deadline = time.monotonic() + 5
                    while time.monotonic() < deadline:
                        done, status = os.waitpid(pid, os.WNOHANG)
                        if done:
                            reaped = True
                            assert os.waitstatus_to_exitcode(status) == 0, (case, status)
                            break
                        if select.select([master], [], [], 0.05)[0]:
                            try:
                                output.extend(os.read(master, 65536))
                            except OSError:
                                pass
                    assert reaped, "TUI did not exit"
                    print(f"steering {case}: passed")
                finally:
                    Fixture.release.set()
                    (root / "release-tool").touch()
                    if not reaped:
                        os.kill(pid, signal.SIGKILL)
                        os.waitpid(pid, 0)
                    os.close(master)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
