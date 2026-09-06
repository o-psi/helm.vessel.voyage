#!/usr/bin/env python3
"""Actual configured MCP metadata confidentiality across all native providers.

Synthetic MCP environment only. A failing peer records bounded reason codes, never
prints received metadata or credentials. Run all cases even on an unfixed binary.
"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from github_configured import stream, tool_results

SECRET = 'mcp-metadata-雪"\\\nsecret'
SAFE_CHOICE = 'stable-雪"\\choice'
SCHEMA = {"type": "object", "properties": {"choice": {"type": "string", "enum": [SAFE_CHOICE],
           "default": SAFE_CHOICE, "description": "Choose the fixture value"}},
          "required": ["choice"], "additionalProperties": False}
CASES = ["description", "schema_description", "nested", "enum", "default", "key"]


def definition(case, secret):
    value = {"name": "echo", "description": "Read structured fixture metadata", "inputSchema": copy.deepcopy(SCHEMA)}
    schema = value["inputSchema"]
    if case == "description":
        value["description"] = "before " + secret + " after"
    elif case == "schema_description":
        schema["properties"]["choice"]["description"] = secret
    elif case == "nested":
        schema["properties"]["choice"]["examples"] = [{"nested": [secret]}]
    elif case in ["enum", "default"]:
        schema["properties"]["choice"][case] = [secret] if case == "enum" else secret
    elif case == "key":
        schema["properties"][secret] = {"type": "string"}
    else:
        raise AssertionError("unknown fixture case")
    return value


def contains_secret(value):
    if isinstance(value, str):
        return SECRET in value
    if isinstance(value, list):
        return any(contains_secret(item) for item in value)
    if isinstance(value, dict):
        return any(contains_secret(key) or contains_secret(item) for key, item in value.items())
    return False


def peer(trace, case):
    secret = os.environ["MCP_METADATA_SECRET"]
    assert secret == SECRET, "configured MCP environment binding absent"
    for line in sys.stdin:
        request = json.loads(line)
        method = request["method"]
        with open(trace, "a") as output:
            output.write(json.dumps({"method": method, "id": request.get("id")}) + "\n")
        if "id" not in request:
            continue
        if method == "initialize":
            result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "metadata-fixture", "version": "1"}}
        elif method == "tools/list":
            result = {"tools": [definition(case, secret)]}
        elif method == "tools/call":
            assert request["params"]["name"] == "echo", "remote tool identity changed"
            assert request["params"]["arguments"] == {"choice": SAFE_CHOICE}, "executable argument changed"
            result = {"content": [{"type": "text", "text": "MCP_METADATA_EFFECT_OK"}]}
        else:
            raise AssertionError("unexpected MCP method")
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    failures = []
    with tempfile.TemporaryDirectory(prefix="helm-mcp-metadata-") as temporary:
        root = Path(temporary)
        state = {"case": None, "requests": [], "errors": []}

        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                try:
                    length = int(self.headers["Content-Length"])
                    if length > 2 * 1024 * 1024:
                        raise AssertionError("provider request exceeds fixture bound")
                    body = json.loads(self.rfile.read(length))
                    state["requests"].append(body)
                    if contains_secret(body):
                        state["errors"].append("provider payload contains configured MCP secret")
                    definitions = [item.get("function", item) for item in body.get("tools", [])]
                    tool = next(item for item in definitions if item["name"] == "mcp_fixture_echo")
                    if state["case"] == "description":
                        if tool["description"] != "before [REDACTED] after":
                            state["errors"].append("natural description projection incorrect")
                        if tool.get("parameters", tool.get("input_schema")) != SCHEMA:
                            state["errors"].append("nonsecret executable schema changed")
                    call = None
                    if state["case"] == "description" and len(state["requests"]) == 1:
                        call = ("metadata-call", "mcp_fixture_echo", {"choice": SAFE_CHOICE})
                    elif state["case"] == "description":
                        if not any("MCP_METADATA_EFFECT_OK" in item for item in tool_results(body)):
                            state["errors"].append("usable tool did not return effect evidence")
                    encoded = stream(self.path, "Metadata fixture finished" if call is None else "", call)
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(encoded)))
                    self.end_headers()
                    self.wfile.write(encoded)
                except Exception as error:
                    state["errors"].append("provider fixture " + type(error).__name__)
                    self.send_error(500)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for provider in ["openai-responses", "openai-chat", "anthropic"]:
                for case in CASES:
                    state.update(case=case, requests=[], errors=[])
                    work = root / (provider + "-" + case)
                    work.mkdir()
                    trace = work / "mcp-trace.jsonl"
                    config = work / "helm.toml"
                    config.write_text('\n'.join([
                        "provider=" + json.dumps(provider), 'model="fixture"',
                        f'base_url="http://127.0.0.1:{server.server_port}/v1"',
                        'api_key_env="METADATA_PROVIDER_KEY"', 'access="unrestricted"',
                        'command_timeout_secs=3', 'provider_retry_attempts=1',
                        '[mcp_servers.fixture]', 'command=' + json.dumps(sys.executable),
                        'args=' + json.dumps(["-u", str(Path(__file__).resolve()), "--peer", str(trace), case]),
                        '[mcp_servers.fixture.env]', "MCP_METADATA_SECRET=" + json.dumps(SECRET),
                    ]))
                    env = dict(os.environ, HOME=str(work / "home"), XDG_CONFIG_HOME=str(work / "config"),
                               XDG_DATA_HOME=str(work / "data"), NO_COLOR="1", METADATA_PROVIDER_KEY="synthetic-provider-key")
                    env.pop("MCP_METADATA_SECRET", None)
                    command = [str(helm), "--config", str(config), "--workspace", str(work)]
                    outcome = subprocess.run(command + ["run", "Exercise the configured metadata fixture"],
                                             env=env, text=True, capture_output=True, timeout=25)
                    errors = list(state["errors"])
                    transcript = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
                    if not any(item["method"] == "tools/list" for item in transcript):
                        errors.append("configured MCP discovery did not run")
                    effects = sum(item["method"] == "tools/call" for item in transcript)
                    succeeded = case == "description"
                    if (outcome.returncode == 0) != succeeded:
                        errors.append("unexpected Helm outcome")
                    if len(state["requests"]) != (2 if succeeded else 0):
                        errors.append("unexpected provider dispatch count")
                    if effects != (1 if succeeded else 0):
                        errors.append("unexpected executable effect count")
                    inspect = subprocess.run(command + ["inference", "inspect"], env=env, text=True,
                                             capture_output=True, timeout=10)
                    if inspect.returncode:
                        errors.append("inference inspection failed")
                    else:
                        evidence = json.loads(inspect.stdout)
                        if evidence["status"]["consumed"] != (2 if succeeded else 0):
                            errors.append("unexpected inference permit consumption")
                        if len(evidence["attempts"]) != (2 if succeeded else 0):
                            errors.append("unexpected inference attempt records")
                    if SECRET in outcome.stdout + outcome.stderr + inspect.stdout + inspect.stderr:
                        errors.append("secret in CLI diagnostics")
                    for session in (work / "data" / "helm" / "sessions").glob("*.json"):
                        if contains_secret(json.loads(session.read_text())):
                            errors.append("secret in canonical session")
                    if errors:
                        failures.append(provider + "/" + case + ": " + "; ".join(sorted(set(errors))))
                    else:
                        print(provider + "/" + case + ": PASS")
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)
    if failures:
        raise AssertionError("\n".join(failures))
    print("MCP metadata confidentiality: 18 configured native cases PASS")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--peer":
        peer(*sys.argv[2:])
    else:
        main()
