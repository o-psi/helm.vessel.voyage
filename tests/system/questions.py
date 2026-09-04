#!/usr/bin/env python3
"""Offline native-provider / real TUI question flow (POSIX PTY only)."""
from __future__ import annotations

import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Fixture(BaseHTTPRequestHandler):
    requests: list[dict] = []

    def log_message(self, *_args: object) -> None:
        pass

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        if len(self.requests) == 1:
            output = [{"type": "function_call", "id": "fc_question", "call_id": "question-1",
                       "name": "questions", "arguments": json.dumps({"question": "Which format?", "options": ["JSON", "Markdown"]})}]
        else:
            output = [{"type": "message", "id": "msg_done", "role": "assistant",
                       "content": [{"type": "output_text", "text": "question-flow-complete"}]}]
        payload = ("data: " + json.dumps({"type": "response.completed", "response": {
            "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main() -> None:
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for case, keys, expected in [
            ("selected", b"\x1b[B\r", {"status": "selected", "index": 1, "answer": "Markdown"}),
            ("custom", b"\t\t\x1b[200~CSV \xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e\x1b[201~\r", {"status": "custom", "answer": "CSV 日本語"}),
            ("cancelled", b"\x1b", {"status": "cancelled"}),
            ("timeout", b"", None),
            ("plain", b"", {"status": "unavailable"}),
        ]:
            with tempfile.TemporaryDirectory(prefix="helm-questions-") as raw:
                root = Path(raw)
                config = root / "config.toml"
                config.write_text(f'''provider = "openai-responses"
model = "fixture"
api_key_env = "HELM_FIXTURE_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
command_timeout_secs = {2 if case == 'timeout' else 10}
access = "read-only"
''')
                env = os.environ.copy()
                env.update(TERM="xterm-256color", HELM_FIXTURE_KEY="offline-fixture-key",
                           HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                           XDG_DATA_HOME=str(root / "data"))
                command = [str(helm), "--config", str(config), "--workspace", str(root)]
                Fixture.requests = []
                if case == "plain":
                    result = subprocess.run(command + ["chat", "--plain"], input=b"ask a question\n/exit\n",
                                            env=env, capture_output=True, timeout=15)
                    assert result.returncode == 0, result.stderr.decode()
                else:
                    pid, master = pty.fork()
                    if pid == 0:
                        os.execve(str(helm), command + ["chat"], env)
                    output = bytearray()
                    def wait_for(text: bytes, timeout: float = 15) -> None:
                        deadline = time.monotonic() + timeout
                        while text not in output:
                            assert time.monotonic() < deadline, (case, text, bytes(output[-3000:]))
                            if select.select([master], [], [], 0.1)[0]:
                                output.extend(os.read(master, 65536))
                    reaped = False
                    try:
                        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
                        wait_for(b"HELM")
                        os.write(master, b"ask a question\r")
                        wait_for(b"custom")
                        # Reflow while awaiting input, then answer through real key/paste routing.
                        fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 18, 48, 0, 0))
                        os.kill(pid, signal.SIGWINCH)
                        time.sleep(0.2)  # allow the resize event to finish before sending a key sequence
                        os.write(master, keys)
                        wait_for(b"question-flow-complete")
                        # Streaming is provisional: Ctrl+Q while the run is still
                        # active cancels it. Wait for canonical completion, not
                        # merely its text, before testing normal exit persistence.
                        deadline = time.monotonic() + 15
                        while True:
                            completed = False
                            for path in (root / "data" / "helm" / "sessions").glob("*.json"):
                                messages = json.loads(path.read_text())["messages"]
                                completed = (
                                    any(m["role"] == "tool" for m in messages)
                                    and any(m["role"] == "assistant" and
                                            m["content"] == "question-flow-complete"
                                            for m in messages)
                                )
                                if completed:
                                    break
                            if completed:
                                break
                            assert time.monotonic() < deadline, (case, "turn was not persisted", bytes(output[-3000:]))
                            if select.select([master], [], [], 0.1)[0]:
                                output.extend(os.read(master, 65536))
                        os.write(master, b"\x11")  # Ctrl+Q
                        deadline = time.monotonic() + 5
                        while time.monotonic() < deadline:
                            done, status = os.waitpid(pid, os.WNOHANG)
                            if done:
                                reaped = True
                                assert os.waitstatus_to_exitcode(status) == 0, (case, status)
                                break
                            if select.select([master], [], [], 0.1)[0]:
                                try:
                                    output.extend(os.read(master, 65536))
                                except OSError:
                                    pass
                        assert reaped, f"{case}: TUI did not exit"
                    finally:
                        if not reaped:
                            os.kill(pid, signal.SIGKILL)
                            os.waitpid(pid, 0)
                        os.close(master)
                assert len(Fixture.requests) == 2, (case, len(Fixture.requests))
                assert any(tool["name"] == "questions" for tool in Fixture.requests[0]["tools"])
                results = [item for item in Fixture.requests[1]["input"] if item.get("type") == "function_call_output"]
                assert len(results) == 1 and results[0]["call_id"] == "question-1", (case, results)
                if expected is None:
                    assert "timed out" in results[0]["output"], results
                else:
                    assert results[0]["output"].startswith("{"), (case, results, bytes(output[-6000:]))
                    assert json.loads(results[0]["output"]) == expected, (case, results)
                sessions = list((root / "data" / "helm" / "sessions").glob("*.json"))
                assert len(sessions) == 1, (case, sessions)
                saved = json.loads(sessions[0].read_text())
                tool = next(m for m in saved["messages"] if m["role"] == "tool")
                assert tool["content"] == results[0]["output"]
                print(f"questions {case}: native provider, input routing, continuation and persistence passed")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
