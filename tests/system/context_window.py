#!/usr/bin/env python3
"""Offline context preflight, tool followup and non-destructive resume regression."""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


LARGE = "old-evidence-雪:" + "x" * 90_000
CALL_ID = "context-read-1"
RESULT = "context-fixture-complete"


def event(value: dict) -> bytes:
    return ("data: " + json.dumps(value) + "\n\n").encode()


def response(provider: str, tool: bool) -> bytes:
    arguments = json.dumps({"path": "evidence.txt"})
    if provider == "openai-responses":
        output = ([{"type": "function_call", "id": "fc_context", "call_id": CALL_ID,
                    "name": "read_file", "arguments": arguments}] if tool else
                  [{"type": "message", "id": "msg_context", "role": "assistant",
                    "content": [{"type": "output_text", "text": RESULT}]}])
        return event({"type": "response.completed", "response": {"output": output,
                      "usage": {"input_tokens": 1, "output_tokens": 1}}})
    if provider == "openai-chat":
        delta = ({"tool_calls": [{"index": 0, "id": CALL_ID, "type": "function",
                  "function": {"name": "read_file", "arguments": arguments}}]} if tool else
                 {"content": RESULT})
        return event({"choices": [{"delta": delta}]}) + b"data: [DONE]\n\n"
    frames = [{"type": "message_start", "message": {"usage": {"input_tokens": 1}}}]
    if tool:
        frames.extend([
            {"type": "content_block_start", "index": 0, "content_block": {
                "type": "tool_use", "id": CALL_ID, "name": "read_file"}},
            {"type": "content_block_delta", "index": 0, "delta": {
                "type": "input_json_delta", "partial_json": arguments}},
        ])
    else:
        frames.append({"type": "content_block_delta", "index": 0,
                       "delta": {"type": "text_delta", "text": RESULT}})
    frames.extend([{"type": "message_delta", "usage": {"output_tokens": 1}},
                   {"type": "message_stop"}])
    return b"".join(event(frame) for frame in frames)


class Fixture(BaseHTTPRequestHandler):
    requests: list[dict] = []
    provider = "openai-responses"
    next_tool = False

    def log_message(self, *_args: object) -> None:
        pass

    def do_POST(self) -> None:
        expected = {"openai-responses": "/v1/responses", "openai-chat": "/v1/chat/completions",
                    "anthropic": "/v1/messages"}[self.provider]
        if self.path != expected:
            self.send_error(404)
            return
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        payload = response(self.provider, self.next_tool)
        type(self).next_tool = False
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def exercise(helm: Path, server: ThreadingHTTPServer, provider: str) -> None:
    Fixture.provider = provider
    Fixture.requests = []
    Fixture.next_tool = False
    with tempfile.TemporaryDirectory(prefix="helm-context-") as raw:
        root = Path(raw)
        (root / "evidence.txt").write_text(LARGE, encoding="utf-8")
        (root / "AGENTS.md").write_text("always-preserved-guidance-雪", encoding="utf-8")
        config = root / "config.toml"
        config.write_text(f'''provider = "{provider}"
model = "offline-context-model"
api_key_env = "HELM_CONTEXT_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
context_window = 65536
max_tokens = 1024
approval = "never"
''', encoding="utf-8")
        env = os.environ.copy()
        env.update(HELM_CONTEXT_KEY="offline-fixture-key", HOME=str(root / "home"),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))

        def run(*args: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run([str(helm), "--config", str(config), "--workspace", str(root),
                                   *args], cwd=root, env=env, capture_output=True, text=True,
                                  timeout=30, check=False)

        def saved_session(result: subprocess.CompletedProcess[str]) -> tuple[Path, dict]:
            match = re.search(r"\[session ([0-9a-f-]{36})\]", result.stderr)
            assert match, result.stderr
            path = root / "data" / "helm" / "sessions" / f"{match.group(1)}.json"
            return path, json.loads(path.read_text(encoding="utf-8"))

        def rejected(result: subprocess.CompletedProcess[str], count: int) -> None:
            assert result.returncode != 0, (result.returncode, result.stdout, result.stderr)
            assert "context" in result.stderr.lower(), result.stderr
            assert "context_window" in result.stderr, result.stderr
            assert len(Fixture.requests) == count, "Rejected request reached provider"

        # The newest prompt must be rejected before any HTTP dispatch. This is
        # below the per-argument OS limit; UTF-8 is still present in the fixture.
        rejected(run("run", "--no-save", LARGE), 0)
        oversized = run("run", LARGE)
        rejected(oversized, 0)
        _, saved_oversized = saved_session(oversized)
        assert [m["content"] for m in saved_oversized["messages"]] == [LARGE]
        assert saved_oversized["title_state"]["completed_runs"] == 0
        for override in ("context_window=0", "max_tokens=65536", "max_tokens=0"):
            result = run("--set", override, "run", "--no-save", "small prompt")
            assert result.returncode != 0, (override, result)
            assert len(Fixture.requests) == 0, override

        # Real read_file execution adds evidence that cannot fit the active turn.
        # A permissive fixture would answer a second call, making this assertion
        # detect a missing followup preflight rather than an HTTP server error.
        Fixture.next_tool = True
        failed = run("run", "Read evidence.txt with read_file.")
        rejected(failed, 1)
        failed_path, saved_failure = saved_session(failed)
        assert saved_failure["title_state"]["completed_runs"] == 0
        assert [m["role"] for m in saved_failure["messages"]] == ["user", "assistant", "tool"]
        assert saved_failure["messages"][1]["tool_calls"][0]["id"] == CALL_ID
        assert saved_failure["messages"][2]["tool_call_id"] == CALL_ID
        assert saved_failure["messages"][2]["content"] == LARGE
        if provider == "openai-responses":
            assert "provider_state" in saved_failure["messages"][1]
        recovered = run("run", "--resume", saved_failure["id"], "Recover without repeating the tool.")
        assert recovered.returncode == 0, recovered
        assert len(Fixture.requests) == 2
        restored = json.loads(failed_path.read_text(encoding="utf-8"))
        assert restored["messages"][:3] == saved_failure["messages"]
        assert len(restored["messages"]) == 5
        assert sum(m["role"] == "tool" for m in restored["messages"]) == 1
        assert LARGE not in json.dumps(Fixture.requests[-1], ensure_ascii=False)

        # Plain chat uses the same failure snapshot and continues without losing
        # its rejected prompt or claiming a completed turn.
        plain = subprocess.run([str(helm), "--config", str(config), "--workspace", str(root),
                                "chat", "--plain"], input=LARGE + "\n/exit\n", cwd=root,
                               env=env, capture_output=True, text=True, timeout=30, check=False)
        assert plain.returncode == 0, plain
        _, saved_plain = saved_session(plain)
        assert [m["content"] for m in saved_plain["messages"]] == [LARGE]
        assert saved_plain["title_state"]["completed_runs"] == 0
        assert len(Fixture.requests) == 2

        # Save a complete tool group under a larger budget, then lower the budget
        # on resume. Only the provider projection may discard this older turn.
        Fixture.next_tool = True
        first = run("--set", "context_window=262144", "run",
                    "old-turn-marker: Read evidence.txt with read_file.")
        assert first.returncode == 0 and RESULT in first.stdout, first
        assert len(Fixture.requests) == 4, Fixture.requests
        assert LARGE in json.dumps(Fixture.requests[-1], ensure_ascii=False)
        match = re.search(r"\[session ([0-9a-f-]{36})\]", first.stderr)
        assert match, first.stderr
        session_id = match.group(1)
        session_path = root / "data" / "helm" / "sessions" / f"{session_id}.json"
        saved_before = json.loads(session_path.read_text(encoding="utf-8"))["messages"]
        assert any(message["role"] == "tool" and LARGE in message["content"]
                   for message in saved_before), saved_before
        resumed = run("run", "--resume", session_id, "newest-turn-marker: Confirm success.")
        assert resumed.returncode == 0 and RESULT in resumed.stdout, resumed
        assert len(Fixture.requests) == 5, Fixture.requests
        wire = json.dumps(Fixture.requests[-1], ensure_ascii=False)
        assert "newest-turn-marker" in wire and "always-preserved-guidance-雪" in wire, wire
        assert "old-turn-marker" not in wire and CALL_ID not in wire and LARGE not in wire, wire
        assert "older messages omitted" in wire, wire
        saved_after = json.loads(session_path.read_text(encoding="utf-8"))["messages"]
        assert saved_after[:len(saved_before)] == saved_before, "Resume mutated canonical history"
        assert len(saved_after) == len(saved_before) + 2, saved_after
        assert "Context projection:" not in json.dumps(saved_after), saved_after
        print(f"context window: {provider}: rejection, configuration, tool followup, resume passed")


def main() -> None:
    repository = Path(__file__).resolve().parents[2]
    helm = Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve()
    assert helm.is_file(), f"Build Helm first or set HELM_BIN: {helm}"
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for provider in ("openai-chat", "openai-responses", "anthropic"):
            exercise(helm, server, provider)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
