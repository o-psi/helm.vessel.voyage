#!/usr/bin/env python3
"""Offline subprocess regression: plain-chat cancellation and idle SIGINT exit."""
from __future__ import annotations

import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from completion_readiness import Case


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.server.requests += 1
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        if self.server.requests == 1:
            output = [{"type": "function_call", "id": "fc_read", "call_id": "read",
                       "name": "read_file", "arguments": json.dumps({"path": "evidence.txt"})}]
            frame = {"type": "response.completed", "response": {
                "output": output, "usage": {"input_tokens": 7, "output_tokens": 3}}}
        else:
            frame = {"type": "response.output_text.delta", "delta": "unfinished streamed response"}
        self.wfile.write(("data: " + json.dumps(frame) + "\n\n").encode())
        self.wfile.flush()
        if self.server.requests > 1:
            self.server.release.wait(20)


def await_saved(case, predicate, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            saved = case.latest_session()
            if predicate(saved):
                return saved
        except (AssertionError, IndexError, KeyError):
            pass
        time.sleep(0.02)
    raise AssertionError(description)


def check_case(case, active):
    proc = subprocess.Popen(case.command("chat", "--plain"), cwd=case.root, env=case.env,
                            text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE)
    lines = queue.Queue()

    def read_output():
        for line in proc.stdout:
            lines.put(line)
        lines.put(None)

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()

    def wait_line(contains):
        deadline = time.monotonic() + 10
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"no plain stdout containing {contains!r}"
            line = lines.get(timeout=remaining)
            assert line is not None, "plain process exited before expected output"
            if contains in line:
                return

    def send(prompt):
        proc.stdin.write(prompt + "\n")
        proc.stdin.flush()

    try:
        send("/session")
        wait_line("input")
        if active:
            send("exercise cancellation")
            await_saved(case, lambda saved: saved["run_summaries"][-1]["partial_output"] ==
                        "unfinished streamed response", "partial checkpoint not observed")
            proc.send_signal(signal.SIGINT)
            saved = await_saved(case, lambda saved: saved["run_summaries"][-1]["phase"] ==
                                "interrupted", "cooperative interruption not saved")
            assert saved["usage"] == {"input_tokens": 7, "output_tokens": 3}, saved["usage"]
            assert len(saved["messages"]) == 3, saved["messages"]
            assert saved["messages"][2]["role"] == "tool", saved["messages"]
            assert saved["run_summaries"][-1]["partial_output"] == "unfinished streamed response"
            assert case.ledger(saved["completion_runs"][-1])["state"]["decision"]["outcome"] == "interrupted"
            send("/session")
            wait_line("7 input, 3 output tokens")
        # Allow the next idle read to start after /session output. Keep stdin OPEN
        # until wait returns: closing the pipe could hide swallowed idle SIGINT.
        time.sleep(0.05)
        proc.send_signal(signal.SIGINT)
        proc.wait(timeout=5)
        stderr = proc.stderr.read()
        assert proc.returncode == 0, (proc.returncode, stderr)
        print(("active cancellation, durable usage/partial, subsequent idle exit" if active
               else "initial idle SIGINT exit") + ": passed", flush=True)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)
        proc.stdin.close()
        reader.join(timeout=5)
        proc.stdout.close()
        proc.stderr.close()


def main():
    repository = Path(__file__).resolve().parents[2]
    helm = Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve()
    assert helm.is_file(), f"Build Helm first or set HELM_BIN: {helm}"
    assert os.name == "posix", "this subprocess fixture requires POSIX SIGINT semantics"
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    server.requests = 0
    server.release = threading.Event()
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-plain-interrupt-") as temporary:
            root = Path(temporary)
            case = Case(root, helm, server.server_port)
            (root / "evidence.txt").write_text("durable evidence", encoding="utf-8")
            check_case(case, active=False)
            check_case(case, active=True)
            assert server.requests == 2, server.requests
    finally:
        server.release.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
