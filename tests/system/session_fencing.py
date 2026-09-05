#!/usr/bin/env python3
"""Actual-process session ownership, metadata contention and in-process switches."""
from __future__ import annotations
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_readiness import Case, MODEL


class Fixture(BaseHTTPRequestHandler):
    requests = []
    held = threading.Event()
    release = threading.Event()

    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({"data": [{"id": MODEL}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        prompt = [m["content"] for m in body["input"] if m.get("role") == "user"][-1]
        self.requests.append(prompt)
        if prompt == "hold":
            self.held.set()
            self.release.wait(20)
        data = ("data: " + json.dumps({"type": "response.completed", "response": {
            "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "fenced-result"}]}],
            "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        try:
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    helm = Path(os.environ.get("HELM_BIN", Path(__file__).resolve().parents[2] / "target/release/helm")).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-session-fencing-") as raw:
            case = Case(Path(raw), helm, server.server_port)
            seed = subprocess.run(case.command("run", "seed"), cwd=case.root, env=case.env, text=True, capture_output=True, timeout=15)
            assert seed.returncode == 0, seed.stderr
            session = case.latest_session()
            owner = subprocess.Popen(case.command("run", "--resume", session["id"], "hold"), cwd=case.root, env=case.env,
                                     text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                assert Fixture.held.wait(10), "owner never reached provider"
                before = case.saved(session["id"])
                contender = subprocess.run(case.command("chat", "--plain", "--resume", session["id"]), cwd=case.root,
                    env=case.env, input="/name overwritten-by-contender\n/exit\n", text=True, capture_output=True, timeout=6)
                assert contender.returncode != 0 and "session busy" in contender.stderr.lower(), (contender.stdout, contender.stderr)
                assert case.saved(session["id"]) == before, "contender changed active canonical session"
                assert Fixture.requests == ["seed", "hold"], Fixture.requests
            finally:
                Fixture.release.set()
                stdout, stderr = owner.communicate(timeout=10)
            assert owner.returncode == 0, (stdout, stderr)
            assert case.saved(session["id"])["run_summaries"][-1]["phase"] == "completed"
            print("session fencing: metadata contender rejected before effects; owner checkpoints and final acceptance preserved")
    finally:
        Fixture.release.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
