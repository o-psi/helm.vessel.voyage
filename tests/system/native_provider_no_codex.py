#!/usr/bin/env python3
"""Exercise Helm's native Responses loop while a sentinel `codex` must stay unused."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def completed(output: list[dict[str, object]], input_tokens: int = 3) -> bytes:
    frame = {
        "type": "response.completed",
        "response": {
            "output": output,
            "usage": {"input_tokens": input_tokens, "output_tokens": 1},
        },
    }
    return f"data: {json.dumps(frame)}\n\n".encode()


def event(frame: dict[str, object]) -> bytes:
    return f"data: {json.dumps(frame)}\n\n".encode()


class ProviderFixture(BaseHTTPRequestHandler):
    requests: list[dict[str, object]] = []
    model_requests: list[dict[str, object]] = []

    def do_GET(self) -> None:  # noqa: N802 -- stdlib callback name
        if not self.path.startswith("/v1/models?"):
            self.send_error(404)
            return
        self.model_requests.append(
            {
                "authorization": self.headers.get("Authorization"),
                "account": self.headers.get("ChatGPT-Account-Id"),
            }
        )
        encoded = json.dumps(
            {"models": [{"slug": "fixture-subscription"}]}
        ).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_POST(self) -> None:  # noqa: N802 -- stdlib callback name
        if self.path != "/v1/responses":
            self.send_error(404)
            return
        size = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(size))
        self.requests.append(
            {
                "authorization": self.headers.get("Authorization"),
                "account": self.headers.get("ChatGPT-Account-Id"),
                "body": body,
            }
        )
        request_number = len(self.requests)
        if request_number == 1:
            # The ChatGPT Responses stream sends authoritative output items in
            # output_item.done events while response.completed may omit them.
            # A function result is invalid unless Helm retains this call item.
            encoded = b"".join(
                [
                    event(
                        {
                            "type": "response.output_item.done",
                            "output_index": 0,
                            "item": {
                                "type": "reasoning",
                                "id": "rs_fixture_first",
                                "encrypted_content": "encrypted-fixture-first",
                                "summary": [],
                            },
                        }
                    ),
                    event(
                        {
                            "type": "response.output_item.added",
                            "output_index": 1,
                            "item": {
                                "type": "function_call",
                                "id": "fc_fixture_first",
                                "call_id": "call_fixture_first",
                                "name": "list_directory",
                            },
                        }
                    ),
                    event(
                        {
                            "type": "response.function_call_arguments.delta",
                            "output_index": 1,
                            "delta": '{"path":".","recursive":false}',
                        }
                    ),
                    event(
                        {
                            "type": "response.output_item.done",
                            "output_index": 1,
                            "item": {
                                "type": "function_call",
                                "id": "fc_fixture_first",
                                "call_id": "call_fixture_first",
                                "name": "list_directory",
                                "arguments": '{"path":".","recursive":false}',
                                "status": "completed",
                            },
                        }
                    ),
                    completed([], input_tokens=3),
                ]
            )
        elif request_number == 2:
            encoded = completed(
                [
                    {
                        "type": "reasoning",
                        "id": "rs_fixture_second",
                        "encrypted_content": "encrypted-fixture-second",
                        "summary": [],
                    },
                    {
                        "type": "message",
                        "id": "msg_fixture_second",
                        "role": "assistant",
                        "content": [
                            {"type": "output_text", "text": "native-provider-ok"}
                        ],
                    },
                ]
            )
        elif request_number == 3:
            encoded = completed(
                [
                    {
                        "type": "message",
                        "id": "msg_fixture_resume",
                        "role": "assistant",
                        "content": [
                            {"type": "output_text", "text": "resume-provider-ok"}
                        ],
                    }
                ],
                input_tokens=9,
            )
        elif request_number == 4:
            encoded = completed(
                [
                    {
                        "type": "message",
                        "id": "msg_fixture_subscription",
                        "role": "assistant",
                        "content": [
                            {"type": "output_text", "text": "subscription-native-ok"}
                        ],
                    }
                ]
            )
        else:
            self.send_error(500, "unexpected extra model request")
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def run_helm(
    helm: Path, config: Path, workspace: Path, environment: dict[str, str], *args: str
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(helm), "--config", str(config), "--workspace", str(workspace), *args],
        cwd=workspace,
        env=environment,
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )


def main() -> None:
    repository = Path(__file__).resolve().parents[2]
    helm = Path(os.environ.get("HELM_BIN", repository / "target/debug/helm")).resolve()
    if not helm.is_file():
        raise SystemExit(f"Helm binary not found: {helm}; build it or set HELM_BIN")

    server = ThreadingHTTPServer(("127.0.0.1", 0), ProviderFixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-native-provider-") as temporary:
            root = Path(temporary)
            sentinel_path = root / "sentinel-path"
            sentinel_path.mkdir()
            codex_marker = root / "codex-was-invoked"
            codex = sentinel_path / "codex"
            codex.write_text(
                f"#!/bin/sh\nprintf invoked > '{codex_marker}'\nexit 99\n",
                encoding="utf-8",
            )
            codex.chmod(0o700)
            (root / "fixture-file.txt").write_text("tool-visible", encoding="utf-8")
            guidance = root / "AGENTS.md"
            guidance.write_text("project-guidance-initial-雪", encoding="utf-8")
            config = root / "config.toml"
            config.write_text(
                "\n".join(
                    [
                        'provider = "openai-responses"',
                        'model = "fixture-native"',
                        'api_key_env = "HELM_FIXTURE_KEY"',
                        f'base_url = "http://127.0.0.1:{server.server_port}/v1"',
                        "provider_retry_attempts = 1",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            environment = os.environ.copy()
            environment.update(
                {
                    "PATH": str(sentinel_path),
                    "HELM_FIXTURE_KEY": "offline-fixture-not-a-real-secret",
                    "HOME": str(root / "home"),
                    "XDG_CONFIG_HOME": str(root / "config"),
                    "XDG_DATA_HOME": str(root / "data"),
                }
            )
            first = run_helm(
                helm,
                config,
                root,
                environment,
                "run",
                "List this directory with the provided tool, then report success.",
            )
            if first.returncode != 0:
                raise AssertionError(
                    "native Helm tool loop failed\n"
                    f"stdout: {first.stdout}\nstderr: {first.stderr}"
                )
            assert "native-provider-ok" in first.stdout, first.stdout
            match = re.search(r"\[session ([0-9a-f-]{36})\]", first.stderr)
            assert match, first.stderr
            session_id = match.group(1)
            session_path = root / "data" / "helm" / "sessions" / f"{session_id}.json"
            saved = json.loads(session_path.read_text(encoding="utf-8"))
            serialized = json.dumps(saved)
            assert "encrypted-fixture-first" in serialized
            assert "encrypted-fixture-second" in serialized
            assert "provider_state" in serialized

            assert "project-guidance-initial" not in serialized
            guidance.write_text("project-guidance-refreshed-雪", encoding="utf-8")

            resumed = run_helm(
                helm,
                config,
                root,
                environment,
                "run",
                "--resume",
                session_id,
                "Confirm that this resumed session retained its context.",
            )
            if resumed.returncode != 0:
                raise AssertionError(
                    "native Helm restart/resume failed\n"
                    f"stdout: {resumed.stdout}\nstderr: {resumed.stderr}"
                )
            assert "resume-provider-ok" in resumed.stdout, resumed.stdout
            saved_text = session_path.read_text(encoding="utf-8")
            assert "project-guidance-initial" not in saved_text
            assert "project-guidance-refreshed" not in saved_text
            assert not codex_marker.exists(), "native provider attempted to execute codex"

            token_path = root / "data" / "helm" / "chatgpt-oauth.json"
            token_path.parent.mkdir(parents=True, exist_ok=True)
            token_path.write_text(
                json.dumps(
                    {
                        "access_token": "offline-subscription-token",
                        "refresh_token": "offline-subscription-refresh",
                        "expires_at": 9_999_999_999,
                        "account_id": "offline-subscription-account",
                    }
                ),
                encoding="utf-8",
            )
            token_path.chmod(0o600)
            oauth_config = root / "oauth-config.toml"
            oauth_config.write_text(
                "\n".join(
                    [
                        'provider = "chatgpt-oauth"',
                        'model = "fixture-subscription"',
                        f'chatgpt_base_url = "http://127.0.0.1:{server.server_port}/v1"',
                        "provider_retry_attempts = 1",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            models = run_helm(
                helm, oauth_config, root, environment, "models", "--json"
            )
            assert models.returncode == 0, models.stderr
            assert "fixture-subscription" in models.stdout, models.stdout
            subscription = run_helm(
                helm,
                oauth_config,
                root,
                environment,
                "run",
                "--no-save",
                "Confirm native subscription transport.",
            )
            assert subscription.returncode == 0, subscription.stderr
            assert "subscription-native-ok" in subscription.stdout, subscription.stdout
            assert not codex_marker.exists(), "subscription provider executed codex"

        assert len(ProviderFixture.requests) == 4, ProviderFixture.requests
        second_input = ProviderFixture.requests[1]["body"]["input"]
        call_index = next(
            index
            for index, item in enumerate(second_input)
            if item.get("type") == "function_call"
            and item.get("call_id") == "call_fixture_first"
        )
        output_index = next(
            index
            for index, item in enumerate(second_input)
            if item.get("type") == "function_call_output"
            and item.get("call_id") == "call_fixture_first"
        )
        assert call_index < output_index, second_input
        for request in ProviderFixture.requests[:3]:
            assert request["authorization"] == "Bearer offline-fixture-not-a-real-secret"
            assert request["account"] is None
        subscription_request = ProviderFixture.requests[3]
        assert subscription_request["authorization"] == "Bearer offline-subscription-token"
        assert subscription_request["account"] == "offline-subscription-account"
        assert ProviderFixture.model_requests == [
            {
                "authorization": "Bearer offline-subscription-token",
                "account": "offline-subscription-account",
            }
        ]
        for index, request in enumerate(ProviderFixture.requests):
            body = request["body"]
            assert isinstance(body, dict)
            expected_model = (
                "fixture-subscription" if index == 3 else "fixture-native"
            )
            wire_text = json.dumps(body, ensure_ascii=False)
            expected_guidance = (
                "project-guidance-initial-雪" if index < 2
                else "project-guidance-refreshed-雪"
            )
            assert expected_guidance in wire_text, body
            if index >= 2:
                assert "project-guidance-initial" not in wire_text, body
            assert body["model"] == expected_model
            assert body["stream"] is True
            assert body["store"] is False
            assert "reasoning.encrypted_content" in body["include"]
        second_input = ProviderFixture.requests[1]["body"]["input"]
        assert any(
            item.get("encrypted_content") == "encrypted-fixture-first"
            for item in second_input
        )
        assert any(item.get("type") == "function_call_output" for item in second_input)
        resumed_input = ProviderFixture.requests[2]["body"]["input"]
        assert any(
            item.get("encrypted_content") == "encrypted-fixture-second"
            for item in resumed_input
        )
        print("native provider independence: ok (tool loop + restart, codex sentinel untouched)")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
    from no_turn_limit import run_no_turn_limit

    repository = Path(__file__).resolve().parents[2]
    run_no_turn_limit(Path(os.environ.get("HELM_BIN", repository / "target/debug/helm")).resolve())
