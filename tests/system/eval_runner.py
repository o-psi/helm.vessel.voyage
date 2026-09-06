#!/usr/bin/env python3
"""Offline actual-Helm regression for evaluator CLI, writes and timeout evidence."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("voyage_eval", ROOT / "eval/run.py")
evaluator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluator)


def frame(value):
    return ("data: " + json.dumps(value) + "\n\n").encode()


class Fixture(BaseHTTPRequestHandler):
    errors = []
    requests = []
    release = threading.Event()
    evidence = None

    def log_message(self, *_):
        pass

    def do_GET(self):
        data = b'{"data":[{"id":"eval-fixture"}]}'
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        try:
            assert self.path == "/v1/chat/completions", self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            prompt = [m["content"] for m in body["messages"] if m["role"] == "user"][-1]
            self.requests.append(prompt)
            assert body["model"] == "eval-fixture"
            names = {tool["function"]["name"] for tool in body["tools"]}
            assert {"todo", "subagent", "completion", "questions", "read_file", "write_file"} <= names, names
            if prompt == "timeout":
                assert json.loads(self.evidence.read_text())["results"][0]["passed"]
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                self.wfile.write(frame({"choices": [{"delta": {"content": "timeout-visible-雪"}}]}))
                self.wfile.flush()
                self.release.wait(15)
                return
            if prompt == "failure":
                prior = json.loads(self.evidence.read_text())["results"]
                assert len(prior) == 2 and prior[-1]["timed_out"] and not prior[-1]["passed"]
                self.send_response(400)
                self.end_headers()
                self.wfile.write(b'{"error":{"message":"expected-error-marker"}}')
                return
            if prompt == "write" and not any(m["role"] == "tool" for m in body["messages"]):
                delta = {"tool_calls": [{"index": 0, "id": "write-eval", "type": "function", "function": {
                    "name": "write_file", "arguments": json.dumps({"path": "result.txt", "content": "verified artifact 42"})}}]}
            else:
                assert prompt in ("write", "after"), prompt
                if prompt == "after":
                    assert len(json.loads(self.evidence.read_text())["results"]) == 3
                delta = {"content": "verified artifact 42"}
            data = frame({"choices": [{"delta": delta}]}) + b"data: [DONE]\n\n"
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            self.errors.append(repr(error))
            self.send_error(500)


def main():
    helm = Path(os.environ.get("HELM_BIN", ROOT / "target/release/helm")).resolve()
    assert helm.is_file(), helm
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-eval-runner-") as raw:
            root = Path(raw)
            config = root / "config/helm/config.toml"
            config.parent.mkdir(parents=True)
            config.write_text(f'''provider = "openai-chat"
model = "eval-fixture"
api_key_env = "HELM_EVAL_FIXTURE_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
access = "read-only"
''')
            evidence = root / "evidence/result.json"
            Fixture.evidence = evidence
            scenarios = [
                {"id": "write", "category": "coding", "prompt": "write", "expect_output": ["42"], "expect_files": {"result.txt": "42"}},
                {"id": "timeout", "category": "interruption", "prompt": "timeout", "expect_output": ["timeout-visible-雪"]},
                {"id": "failure", "category": "research", "prompt": "failure", "expect_output": ["expected-error-marker"]},
                {"id": "after", "category": "research", "prompt": "after", "expect_output": ["42"]},
            ]
            with patch.dict(os.environ, {"HOME": str(root / "home"), "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_DATA_HOME": str(root / "data"), "HELM_EVAL_FIXTURE_KEY": "offline-key"}):
                # A relative binary path must remain valid after switching cwd to
                # each disposable workspace. The CLI accepts the documented flag.
                status = evaluator.run_live(scenarios, os.path.relpath(helm), evidence, timeout=2)
            results = json.loads(evidence.read_text())["results"]
            assert status == 1 and [r["passed"] for r in results] == [True, False, False, True], results
            assert not Fixture.errors, Fixture.errors
            assert Fixture.requests == ["write", "write", "timeout", "failure", "after"], Fixture.requests
            assert results[0]["checks"]["file:result.txt:42"]
            assert results[1]["timed_out"] and results[1]["exit_code"] is None
            assert "timeout-visible-雪" in results[1]["stdout"], results[1]
            assert results[1]["checks"]["output:timeout-visible-雪"] is True
            assert not results[2]["timed_out"] and results[2]["exit_code"] != 0
            assert results[2]["checks"]["output:expected-error-marker"] is False, "stderr diagnostics satisfied answer check"
            assert not list(evidence.parent.glob("result.json.*")), "temporary evidence files leaked"
            assert not list((root / "data/helm/sessions").glob("*.json")), "--no-save unexpectedly published sessions"
            missing = root / "missing.json"
            assert evaluator.run_live(scenarios[:1], str(root / "nonexistent-helm"), missing) == 1
            failed = json.loads(missing.read_text())["results"][0]
            assert failed["exit_code"] is None and "could not start" in failed["error"]
            for value in ("0", "-1", "nan", "inf"):
                check = subprocess.run(["python3", str(ROOT / "eval/run.py"), "validate", "--timeout-seconds", value], capture_output=True, text=True)
                assert check.returncode == 2 and "positive finite" in check.stderr, check
            print("eval runner: actual CLI write, relative binary, partial timeout, nonzero false-positive, incremental evidence and continued scenarios passed")
    finally:
        Fixture.release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
