#!/usr/bin/env python3
"""Actual-process session ownership, metadata contention and in-process switches."""
from __future__ import annotations
import json
import os
import re
import select
import time
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
            # Existing-session --no-save still executes effects and needs the same
            # fence, while leaving canonical session bytes unchanged.
            Fixture.held = threading.Event()
            Fixture.release = threading.Event()
            before = case.saved(session["id"])
            owner = subprocess.Popen(case.command("run", "--no-save", "--resume", session["id"], "hold"), cwd=case.root, env=case.env,
                                     text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                assert Fixture.held.wait(10)
                contender = subprocess.run(case.command("chat", "--plain", "--resume", str(case.root / "data/helm/sessions" / (session["id"] + ".json"))),
                    cwd=case.root, env=case.env, input="/clear\n/exit\n", text=True, capture_output=True, timeout=6)
                assert contender.returncode != 0 and "session busy" in contender.stderr.lower(), contender
                assert case.saved(session["id"]) == before
            finally:
                Fixture.release.set()
                stdout, stderr = owner.communicate(timeout=10)
            assert owner.returncode == 0 and case.saved(session["id"]) == before, (stdout, stderr)

            # An idle frontend owns its current session. /new must transfer the
            # lease, not merely change the in-memory ID behind a bound store.
            chat = subprocess.Popen(case.command("chat", "--plain", "--resume", session["id"]), cwd=case.root, env=case.env,
                                    text=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=1)
            def command_line(command):
                chat.stdin.write(command + "\n")
                chat.stdin.flush()
                assert select.select([chat.stdout], [], [], 6)[0], "plain command did not respond: " + command
                return chat.stdout.readline()
            try:
                assert session["id"] in command_line("/session")
                assert "new session" in command_line("/new switched-owner")
                line = command_line("/session")
                new_id = re.search(r"[0-9a-f-]{36}", line).group(0)
                assert new_id != session["id"]
                assert "renamed" in command_line("/name switched-owner")
                released = subprocess.run(case.command("chat", "--plain", "--resume", session["id"]), cwd=case.root, env=case.env,
                    input="/name released-old-owner\n/exit\n", text=True, capture_output=True, timeout=6)
                assert released.returncode == 0, released.stderr
                busy = subprocess.run(case.command("chat", "--plain", "--resume", "switched-owner"), cwd=case.root, env=case.env,
                    input="/name wrong\n/exit\n", text=True, capture_output=True, timeout=6)
                assert busy.returncode != 0 and "session busy" in busy.stderr.lower(), busy
                chat.stdin.write("new-session-run\n")
                chat.stdin.flush()
                deadline = time.monotonic() + 10
                while not any(m["content"] == "fenced-result" for m in case.saved(new_id)["messages"]):
                    assert time.monotonic() < deadline, "new session never completed"
                    time.sleep(0.01)
                chat.stdin.write("/exit\n")
                chat.stdin.flush()
                stdout, stderr = chat.communicate(timeout=10)
                assert chat.returncode == 0, (stdout, stderr)
                assert case.saved(new_id)["run_summaries"][-1]["phase"] == "completed"
            finally:
                if chat.poll() is None:
                    chat.kill()
                    chat.communicate(timeout=5)
            assert Fixture.requests == ["seed", "hold", "hold", "new-session-run"], Fixture.requests
            print("session fencing: metadata contenders rejected before effects; no-save fencing, new-session transfer and final acceptance preserved")
    finally:
        Fixture.release.set()
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    main()
