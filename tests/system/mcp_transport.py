#!/usr/bin/env python3
"""Configured MCP through actual Helm and a synthetic native Responses provider."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from github_configured import stream, tool_results


def peer(trace, mode):
    for line in sys.stdin:
        request = json.loads(line)
        with open(trace, "a") as output:
            output.write(json.dumps(request) + "\n")
        method = request["method"]
        if "id" not in request:
            continue
        if method == "initialize":
            result = {"protocolVersion": "2099-01-01" if mode == "version" else "2025-06-18",
                      "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}
        elif method == "tools/list":
            result = {"tools": [{"name": "echo", "description": "Synthetic local echo",
                                 "inputSchema": {"type": "object", "properties": {}}}]}
        elif mode == "timeout":
            continue
        elif mode == "malformed":
            print("INVALID_SYNTHETIC_PRIVATE_DIAGNOSTIC", flush=True)
            continue
        else:
            result = {"content": [{"type": "text", "text": "MCP雪é"}]}
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    with tempfile.TemporaryDirectory(prefix="helm-mcp-transport-") as temporary:
        root = Path(temporary)
        state = {"requests": [], "failures": [], "mode": "success"}

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                try:
                    assert self.path == "/v1/responses"
                    body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                    state["requests"].append(body)
                    count = len(state["requests"])
                    assert any(tool.get("name") == "mcp_fixture_echo" for tool in body["tools"])
                    assert "INVALID_SYNTHETIC_PRIVATE_DIAGNOSTIC" not in json.dumps(body)
                    call = None
                    if count == 1:
                        call = ("mcp-first", "mcp_fixture_echo", {})
                    else:
                        results = tool_results(body)
                        assert results
                        if state["mode"] == "success":
                            assert any("MCP雪é" in result for result in results)
                        elif count == 2:
                            assert any("timed out" in result or "malformed" in result for result in results)
                            call = ("mcp-second", "mcp_fixture_echo", {})
                        else:
                            assert any("retired" in result for result in results)
                    encoded = stream(self.path, "MCP fixture complete" if call is None else "", call)
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(encoded)))
                    self.end_headers()
                    self.wfile.write(encoded)
                except Exception as error:
                    state["failures"].append(str(error))
                    self.send_error(500)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for mode in ["success", "timeout", "malformed", "version"]:
                state.update(requests=[], failures=[], mode=mode)
                trace = root / (mode + ".jsonl")
                config = root / (mode + ".toml")
                config.write_text('\n'.join([
                    'provider="openai-responses"', 'model="fixture"',
                    f'base_url="http://127.0.0.1:{server.server_port}/v1"',
                    'api_key_required=false', 'access="unrestricted"',
                    'command_timeout_secs=1', 'provider_retry_attempts=1',
                    '[mcp_servers.fixture]', 'command=' + json.dumps(sys.executable),
                    'args=' + json.dumps(["-u", str(Path(__file__).resolve()), "--peer", str(trace), mode]),
                ]))
                env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                           XDG_DATA_HOME=str(root / "data"), NO_COLOR="1")
                outcome = subprocess.run([str(helm), "--config", str(config), "--workspace", str(root),
                                          "run", "Exercise the synthetic MCP fixture"],
                                         env=env, text=True, capture_output=True, timeout=20)
                assert not state["failures"], state["failures"]
                assert (outcome.returncode == 0) == (mode != "version"), outcome.stderr[-3000:]
                transcript = [json.loads(line) for line in trace.read_text().splitlines()]
                calls = [item for item in transcript if item["method"] == "tools/call"]
                cancellations = [item for item in transcript if item["method"] == "notifications/cancelled"]
                if mode == "version":
                    assert len(transcript) == 1 and not state["requests"]
                else:
                    assert len(calls) == 1, "retired transport dispatched a new effect"
                    assert len(state["requests"]) == (2 if mode == "success" else 3)
                if mode == "timeout":
                    assert len(cancellations) == 1
                    assert cancellations[0]["params"]["requestId"] == calls[0]["id"]
                assert "INVALID_SYNTHETIC_PRIVATE_DIAGNOSTIC" not in outcome.stdout + outcome.stderr
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    print("configured MCP native invocation, timeout retirement and negotiation: PASS")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--peer":
        peer(*sys.argv[2:])
    else:
        main()
