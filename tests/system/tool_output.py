#!/usr/bin/env python3
"""Offline native-provider / real TUI tool previews and details (POSIX PTY)."""
from __future__ import annotations

import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import termios
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


COMMAND = "cat fixture.txt; cat diagnostic.txt >&2; exit 7"
TAIL = b"full-output-tail-marker"
STDOUT = "preview first line\n    indented second line\n" + "ordinary output\n" * 15 + TAIL.decode() + "\n"
STDERR = "fixture fatal diagnostic\n"
EXPECTED = f"exit: 7\nstdout:\n{STDOUT}\nstderr:\n{STDERR}"


class Fixture(BaseHTTPRequestHandler):
    requests: list[dict] = []

    def log_message(self, *_args: object) -> None:
        pass

    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        if len(self.requests) == 1:
            output = [{"type": "function_call", "id": "fc_shell", "call_id": "shell-1",
                       "name": "shell", "arguments": json.dumps({"command": COMMAND})}]
        else:
            output = [{"type": "message", "id": "msg_done", "role": "assistant",
                       "content": [{"type": "output_text", "text": "tool-output-flow-complete"}]}]
        payload = ("data: " + json.dumps({"type": "response.completed", "response": {
            "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main() -> None:
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    assert helm.is_file(), f"Build Helm first or set HELM_BIN: {helm}"
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-tool-output-") as raw:
            root = Path(raw)
            (root / "fixture.txt").write_text(STDOUT)
            (root / "diagnostic.txt").write_text(STDERR)
            config = root / "config.toml"
            config.write_text(f'''provider = "openai-responses"
model = "fixture"
api_key_env = "HELM_FIXTURE_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
command_timeout_secs = 10
approval = "never"
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

            def read_output(timeout: float = 0.1) -> None:
                if select.select([master], [], [], timeout)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        pass

            def wait_for(marker: bytes, start: int = 0, timeout: float = 15) -> None:
                deadline = time.monotonic() + timeout
                while marker not in output[start:]:
                    assert time.monotonic() < deadline, (marker, bytes(output[-6000:]))
                    read_output()

            def resize(rows: int, cols: int) -> None:
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
                os.kill(pid, signal.SIGWINCH)

            def saved_messages() -> list[dict]:
                paths = list((root / "data" / "helm" / "sessions").glob("*.json"))
                return json.loads(paths[0].read_text())["messages"] if paths else []

            try:
                # Enough rows for both complete details and previews, so absence
                # of the tail before expansion proves it was actually omitted.
                resize(60, 120)
                wait_for(b"HELM")
                os.write(master, b"exercise tool output\r")
                wait_for(b"tool-output-flow-complete")
                deadline = time.monotonic() + 15
                while not any(m["role"] == "assistant" and m["content"] == "tool-output-flow-complete"
                              for m in saved_messages()):
                    assert time.monotonic() < deadline, "Turn was not persisted"
                    read_output()
                wait_for(b"exit 7")
                wait_for(b"fixture fatal diagnostic")
                wait_for(b"Failed")
                wait_for(b"preview first line")
                wait_for("… more".encode())
                assert TAIL not in output, "Collapsed preview leaked the result tail"

                expanded_start = len(output)
                os.write(master, b"\x0f")  # Ctrl+O
                # Ratatui may reuse unchanged spaces between words, emitting
                # cursor moves rather than a contiguous status sentence.
                wait_for(b"collapses", expanded_start)
                wait_for(TAIL, expanded_start)
                wait_for(b"Arguments", expanded_start)
                wait_for(b"Result", expanded_start)

                # Exercise actual terminal resize and reflow while expanded.
                resize_start = len(output)
                resize(55, 76)
                wait_for(TAIL, resize_start)
                wait_for(b"fixture fatal diagnostic", resize_start)

                collapse_start = len(output)
                os.write(master, b"\x0f")
                wait_for(b"previews", collapse_start)
                wait_for("… more".encode(), collapse_start)
                assert TAIL not in output[collapse_start:], "Collapsing redrew full output"

                assert len(Fixture.requests) == 2, Fixture.requests
                results = [item for item in Fixture.requests[1]["input"]
                           if item.get("type") == "function_call_output"]
                assert len(results) == 1 and results[0]["call_id"] == "shell-1", results
                assert results[0]["output"] == EXPECTED, results
                tool = next(m for m in saved_messages() if m["role"] == "tool")
                assert tool["content"] == EXPECTED, tool
                # Exit 7 remains a successful invocation in canonical history;
                # the TUI independently exposes the command failure.
                assert tool["tool_success"] is True, tool

                os.write(master, b"\x11")  # Ctrl+Q
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    done, status = os.waitpid(pid, os.WNOHANG)
                    if done:
                        reaped = True
                        assert os.waitstatus_to_exitcode(status) == 0, status
                        break
                    read_output()
                assert reaped, "TUI did not quit gracefully"
                read_output(0)
                assert b"\x1b[?1049l" in output, "TUI did not restore the main terminal screen"
                print("tool output: native provider, failed shell previews, expansion, resize, collapse, persistence and terminal restoration passed")
            finally:
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
