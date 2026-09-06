#!/usr/bin/env python3
"""Configured native GitHub registry, credential isolation and offline CLI flows.

Only the synthetic model provider has a loopback endpoint. No GitHub endpoint is
overridden; the sole model GitHub action lists its local private journal.
"""
import base64
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading


TOKEN = "synthetic-github-configured-secret-91837"
FORMS = [TOKEN, base64.b64encode(TOKEN.encode()).decode(), TOKEN.encode().hex()]


def tool_results(body):
    results = []
    for message in body.get("input", []) + body.get("messages", []):
        if message.get("type") == "function_call_output":
            results.append(message["output"])
        elif message.get("role") == "tool":
            results.append(message["content"])
        elif isinstance(message.get("content"), list):
            results.extend(block["content"] for block in message["content"]
                           if block.get("type") == "tool_result")
    return results


def stream(path, text, call=None):
    if path.endswith("responses"):
        output = [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": text}]}]
        if call:
            output.append({"type": "function_call", "id": "fc" + call[0], "call_id": call[0],
                           "name": call[1], "arguments": json.dumps(call[2])})
        frames = [{"type": "response.completed", "response": {"output": output, "usage": {"input_tokens": 3, "output_tokens": 2}}}]
        return "".join("data: " + json.dumps(frame) + "\n\n" for frame in frames).encode()
    if path.endswith("chat/completions"):
        delta = {"role": "assistant", "content": text}
        if call:
            delta["tool_calls"] = [{"index": 0, "id": call[0], "type": "function",
                                    "function": {"name": call[1], "arguments": json.dumps(call[2])}}]
        frame = {"choices": [{"index": 0, "delta": delta, "finish_reason": "tool_calls" if call else "stop"}],
                 "usage": {"prompt_tokens": 3, "completion_tokens": 2}}
        return ("data: " + json.dumps(frame) + "\n\ndata: [DONE]\n\n").encode()
    assert path.endswith("messages"), "unexpected fixture provider endpoint"
    frames = [{"type": "message_start", "message": {"id": "fixture", "type": "message", "role": "assistant", "model": "fixture", "content": [], "usage": {"input_tokens": 3, "output_tokens": 0}}},
              {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
              {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}},
              {"type": "content_block_stop", "index": 0}]
    if call:
        frames.extend([{"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": call[0], "name": call[1], "input": {}}},
                       {"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": json.dumps(call[2])}},
                       {"type": "content_block_stop", "index": 1}])
    frames.extend([{"type": "message_delta", "delta": {"stop_reason": "tool_use" if call else "end_turn"}, "usage": {"output_tokens": 2}}, {"type": "message_stop"}])
    return "".join("event: " + frame["type"] + "\ndata: " + json.dumps(frame) + "\n\n" for frame in frames).encode()


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    with tempfile.TemporaryDirectory(prefix="helm-github-configured-") as temporary:
        root = Path(temporary)
        work = root / "work"
        work.mkdir()
        subprocess.run(["git", "init", "--quiet", str(work)], check=True, capture_output=True, timeout=10)
        env = dict(os.environ, HOME=str(root / "home"), XDG_DATA_HOME=str(root / "data"),
                   XDG_CONFIG_HOME=str(root / "config"), HELM_GITHUB_TOKEN=TOKEN,
                   HELM_GITHUB_PROVIDER_FIXTURE_KEY="synthetic-provider-key")
        for name in ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "GH_TOKEN", "GITHUB_TOKEN", "OPENAI_ORG_ID", "OPENAI_PROJECT_ID"]:
            env.pop(name, None)
        config = root / "helm.toml"
        requests, failures = [], []
        mode = {"enabled": False}

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                try:
                    length = int(self.headers["Content-Length"])
                    assert length <= 2 * 1024 * 1024
                    body = json.loads(self.rfile.read(length))
                    raw = json.dumps(body)
                    assert not any(secret in raw for secret in FORMS), "credential leaked into provider request"
                    names = [tool.get("name") or tool.get("function", {}).get("name") for tool in body.get("tools", [])]
                    assert ("github" in names) == mode["enabled"], "configured registry mismatch"
                    requests.append(body)
                    count = len(requests)
                    call = None
                    text = "Configured fixture complete"
                    if mode["enabled"] and count == 1:
                        text = " ".join(FORMS)
                        call = ("github1", "github", {"action": "list", "offset": 0})
                    elif mode["enabled"] and count == 2:
                        assert any(isinstance(result, str) and result.strip() == "[]" for result in tool_results(body)), "GitHub list did not return scoped empty journal"
                        call = ("shell2", "shell", {"command": 'if [ "${HELM_GITHUB_TOKEN+x}" = x ]; then printf GITHUB_CREDENTIAL_LEAK; else printf GITHUB_CREDENTIAL_ABSENT; fi'})
                    elif mode["enabled"] and count == 3:
                        outputs = json.dumps(tool_results(body))
                        assert "GITHUB_CREDENTIAL_ABSENT" in outputs and "GITHUB_CREDENTIAL_LEAK" not in outputs, "shell credential isolation failed"
                    data = stream(self.path, text, call)
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except Exception as error:
                    failures.append(type(error).__name__ + ": " + str(error))
                    self.send_error(500)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()

        def configure(enabled=False, extra=""):
            flag = "" if enabled is None else f"github_enabled = {str(enabled).lower()}\n"
            config.write_text(f'provider = "openai-responses"\nmodel = "fixture"\napi_key_env = "HELM_GITHUB_PROVIDER_FIXTURE_KEY"\nbase_url = "http://127.0.0.1:{server.server_port}/v1"\nprovider_retry_attempts = 1\naccess = "unrestricted"\n' + flag + extra)

        def run(*args, success=True, environ=None):
            result = subprocess.run([str(helm), "--workspace", str(work), "--config", str(config), *args],
                                    env=environ or env, cwd=work, capture_output=True, text=True, timeout=30)
            assert (result.returncode == 0) == success, "unexpected configured fixture exit status"
            assert not any(secret in result.stdout + result.stderr for secret in FORMS), "credential leaked into diagnostics"
            assert not failures, failures
            return result

        try:
            for provider in ["openai-responses", "openai-chat", "anthropic"]:
                requests.clear()
                mode["enabled"] = True
                configure(True)
                run("--provider", provider, "run", "Exercise configured local GitHub list and isolated shell")
                assert len(requests) == 3, "configured tools did not execute full follow-up loop"
            for enabled, token in [(None, True), (True, False)]:
                requests.clear()
                mode["enabled"] = False
                configure(enabled)
                isolated = env.copy()
                if not token:
                    isolated.pop("HELM_GITHUB_TOKEN")
                run("run", "Check unavailable GitHub capability", environ=isolated)
                assert len(requests) == 1
                run("github", "auth", success=False, environ=isolated)
                assert len(requests) == 1, "static capability failure contacted provider"
            configure(False, 'inherit_env = ["HELM_GITHUB_TOKEN"]\n')
            run("github", "list", success=False)
            configure(False)
            offline = env.copy()
            offline.pop("HELM_GITHUB_PROVIDER_FIXTURE_KEY")
            before = len(requests)
            run("github", "list", environ=offline)
            run("github", "remotes", environ=offline)
            sessions = list((root / "data" / "helm" / "sessions").glob("*.json"))
            assert sessions, "native runs did not persist voyages"
            session_id = json.loads(sessions[0].read_text())["id"]
            run("github", "--session", session_id, "references", environ=offline)
            assert len(requests) == before, "offline CLI contacted provider"
            for session in sessions:
                assert not any(secret in session.read_text() for secret in FORMS), "credential persisted in session"
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("configured GitHub native tools, credential isolation and offline CLI: PASS")


if __name__ == "__main__":
    main()
