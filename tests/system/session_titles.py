#!/usr/bin/env python3
"""Offline native-provider title selection, scheduling, isolation, and persistence."""
from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


MAIN_MODEL = "fixture-conversation"
TITLE_MODEL = "gpt-5.6-luna"


class Fixture(BaseHTTPRequestHandler):
    requests: list[dict] = []
    advertised = True
    fail_title = False
    discovery_requests = 0
    authenticated = True

    def log_message(self, *_args: object) -> None:
        pass

    def reply(self, value: object, status: int = 200) -> None:
        payload = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:
        if self.path != "/v1/models":
            self.send_error(404)
            return
        type(self).discovery_requests += 1
        type(self).authenticated &= self.headers.get("Authorization") == "Bearer offline-fixture-key"
        models = [MAIN_MODEL] + ([TITLE_MODEL] if self.advertised else [])
        self.reply({"data": [{"id": model} for model in models]})

    def do_POST(self) -> None:
        if self.path != "/v1/chat/completions":
            self.send_error(404)
            return
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.requests.append(body)
        type(self).authenticated &= self.headers.get("Authorization") == "Bearer offline-fixture-key"
        if body["model"] == TITLE_MODEL:
            if self.fail_title:
                self.reply({"error": {"message": "offline title failure"}}, 500)
                return
            number = len([r for r in self.requests if r["model"] == TITLE_MODEL])
            content = f"Generated topic {number}"
            usage = {"prompt_tokens": 11, "completion_tokens": 2}
        elif body["model"] == MAIN_MODEL:
            content = "conversation-answer"
            usage = {"prompt_tokens": 3, "completion_tokens": 1}
        else:
            self.reply({"error": {"message": "unexpected model"}}, 400)
            return
        if body.get("stream"):
            frame = {"choices": [{"delta": {"content": content}, "finish_reason": "stop"}],
                     "usage": usage}
            payload = (f"data: {json.dumps(frame)}\n\ndata: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        else:
            self.reply({"choices": [{"message": {"role": "assistant", "content": content},
                                     "finish_reason": "stop"}], "usage": usage})


def titles() -> list[dict]:
    return [r for r in Fixture.requests if r["model"] == TITLE_MODEL]


class Case:
    def __init__(self, root: Path, helm: Path, port: int):
        self.root = root
        self.helm = helm
        self.config = root / "config.toml"
        self.config.write_text(f'''provider = "openai-chat"
model = "{MAIN_MODEL}"
api_key_env = "HELM_FIXTURE_KEY"
base_url = "http://127.0.0.1:{port}/v1"
provider_retry_attempts = 1
access = "read-only"
''', encoding="utf-8")
        self.env = os.environ.copy()
        self.env.update(HELM_FIXTURE_KEY="offline-fixture-key", HOME=str(root / "home"),
                        XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        Fixture.requests = []
        Fixture.advertised = True
        Fixture.fail_title = False
        Fixture.discovery_requests = 0
        Fixture.authenticated = True

    def invoke(self, *args: str, stdin: str | None = None) -> subprocess.CompletedProcess:
        result = subprocess.run(
            [str(self.helm), "--config", str(self.config), "--workspace", str(self.root), *args],
            cwd=self.root, env=self.env, input=stdin, text=True, capture_output=True, timeout=20,
        )
        assert result.returncode == 0, (args, result.stdout, result.stderr)
        assert "Generated topic" not in result.stdout, result.stdout
        return result

    def run(self, resume: str | None = None) -> str:
        args = ["run"] + (["--resume", resume] if resume else [])
        result = self.invoke(*args, "Discuss the offline example")
        assert "conversation-answer" in result.stdout, result.stdout
        match = re.search(r"\[session ([0-9a-f-]{36})\]", result.stderr)
        assert match, result.stderr
        return match.group(1)

    def path(self, session: str) -> Path:
        return self.root / "data" / "helm" / "sessions" / f"{session}.json"

    def saved(self, session: str) -> dict:
        return json.loads(self.path(session).read_text(encoding="utf-8"))

    def assert_isolated(self, session: str, turns: int, title_count: int) -> None:
        saved = self.saved(session)
        assert saved["model"] == MAIN_MODEL, saved
        assert saved["model_history"] == [], saved
        assert saved["usage"] == {"input_tokens": 3 * turns, "output_tokens": turns}, saved
        assert saved["title_state"]["usage"] == {
            "input_tokens": 11 * title_count, "output_tokens": 2 * title_count}, saved
        assert "Generated topic" not in json.dumps(saved["messages"]), saved
        assert TITLE_MODEL not in json.dumps(saved["messages"]), saved
        assert len([m for m in saved["messages"] if m["role"] == "user"]) == turns, saved
        for request in Fixture.requests:
            if request["model"] == TITLE_MODEL:
                assert not request.get("tools"), request
                assert request.get("stream") is False, request
                assert 0 < request["max_completion_tokens"] <= 256, request
                assert len(json.dumps(request["messages"])) < 20000, request
            else:
                assert request["model"] == MAIN_MODEL, request
                assert request.get("tools"), "title call stripped main conversation tools"
                assert "Generated topic" not in json.dumps(request["messages"]), request


def main() -> None:
    repository = Path(__file__).resolve().parents[2]
    helm = Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve()
    assert helm.is_file(), f"Build Helm or set HELM_BIN: {helm}"
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for name in ["fibonacci", "missing", "failure", "no-save", "manual", "legacy"]:
            with tempfile.TemporaryDirectory(prefix=f"helm-titles-{name}-") as temporary:
                case = Case(Path(temporary), helm, server.server_port)
                if name == "fibonacci":
                    session = None
                    expected = 0
                    for turn in range(1, 9):
                        session = case.run(session)
                        expected += int(turn in {1, 2, 3, 5, 8})
                        assert len(titles()) == expected, (turn, expected, [r["model"] for r in Fixture.requests])
                        saved = case.saved(session)
                        assert saved["name"] == f"Generated topic {expected}", saved
                        assert saved["title_state"]["completed_runs"] == turn, saved
                        case.assert_isolated(session, turn, expected)
                    assert Fixture.discovery_requests > 0
                elif name == "missing":
                    Fixture.advertised = False
                    session = case.run()
                    assert Fixture.discovery_requests > 0
                    assert not titles(), Fixture.requests
                    assert len(Fixture.requests) == 1, Fixture.requests
                    assert case.saved(session)["title_state"]["completed_runs"] == 1
                    case.assert_isolated(session, 1, 0)
                elif name == "failure":
                    session = case.run()
                    initial_name = case.saved(session)["name"]
                    Fixture.fail_title = True
                    case.run(session)
                    assert len(titles()) == 2, Fixture.requests
                    assert case.saved(session)["name"] == initial_name
                    case.assert_isolated(session, 2, 1)
                elif name == "no-save":
                    case.invoke("run", "--no-save", "Discuss the offline example")
                    assert not titles(), Fixture.requests
                    assert not list((case.root / "data" / "helm" / "sessions").glob("*.json"))
                else:
                    session = case.run()
                    if name == "legacy":
                        saved = case.saved(session)
                        saved.pop("title_state")
                        saved["name"] = "Manually chosen legacy title"
                        case.path(session).write_text(json.dumps(saved), encoding="utf-8")
                        case.run(session)
                        assert case.saved(session)["name"] == "Manually chosen legacy title"
                    else:
                        case.invoke("chat", "--plain", "--resume", session,
                                    stdin="/name My chosen title\nFollow up\n/clear\nStart again\n/exit\n")
                        saved = case.saved(session)
                        assert saved["name"] == "My chosen title", saved
                        assert saved["title_state"]["completed_runs"] == 1, saved
                        assert saved["title_state"]["automatic"] is False, saved
                    assert len(titles()) == 1, Fixture.requests
                assert Fixture.authenticated, "provider credentials were not retained"
        print("session titles: Fibonacci resume, separate model/usage/history, missing model, failure, no-save, manual and legacy preservation passed")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
