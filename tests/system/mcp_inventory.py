#!/usr/bin/env python3
"""Plain /tools must redact known MCP metadata and render control bytes safely."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

SECRET = 'inventory-秘密"\\\nsecret'
DESCRIPTION = 'before ' + SECRET + ' after \x1b[31mRED\x1b[0m\u202evisible'


def peer(trace):
    assert os.environ["INVENTORY_SECRET"] == SECRET
    for line in sys.stdin:
        request = json.loads(line)
        with open(trace, "a") as output:
            output.write(request["method"] + "\n")
        if "id" not in request:
            continue
        if request["method"] == "initialize":
            result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "fixture", "version": "1"}}
        else:
            assert request["method"] == "tools/list", "inventory executed a tool"
            result = {"tools": [{"name": "echo", "description": DESCRIPTION,
                                  "inputSchema": {"type": "object"}}]}
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)


def main():
    helm = Path(os.environ.get("HELM_BIN", "target/release/helm")).resolve()
    with tempfile.TemporaryDirectory(prefix="helm-mcp-inventory-") as temporary:
        root = Path(temporary)
        trace = root / "trace"
        config = root / "helm.toml"
        config.write_text('\n'.join([
            'provider="openai-responses"', 'model="fixture"', 'api_key_required=false',
            'base_url="http://127.0.0.1:1/v1"', 'access="unrestricted"',
            '[mcp_servers.fixture]', 'command=' + json.dumps(sys.executable),
            'args=' + json.dumps(["-u", str(Path(__file__).resolve()), "--peer", str(trace)]),
            '[mcp_servers.fixture.env]', 'INVENTORY_SECRET=' + json.dumps(SECRET),
        ]))
        env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                   XDG_DATA_HOME=str(root / "data"), NO_COLOR="1")
        env.pop("INVENTORY_SECRET", None)
        result = subprocess.run([str(helm), "--config", str(config), "--workspace", str(root),
                                 "chat", "--plain"], input="/tools\n/exit\n", env=env,
                                text=True, capture_output=True, timeout=20)
        assert result.returncode == 0, "plain inventory failed"
        assert trace.read_text().splitlines() == ["initialize", "notifications/initialized", "tools/list"]
        assert "mcp_fixture_echo" in result.stdout and "before [REDACTED] after" in result.stdout
        assert "RED" in result.stdout and "visible" in result.stdout
        assert SECRET not in result.stdout + result.stderr, "known MCP secret printed"
        assert "\x1b" not in result.stdout + result.stderr, "raw terminal escape printed"
        assert "\u202e" not in result.stdout + result.stderr, "raw bidi control printed"
        for session in (root / "data" / "helm" / "sessions").glob("*.json"):
            for message in json.loads(session.read_text()).get("messages", []):
                assert SECRET not in message.get("content", ""), "inventory secret persisted as conversation"
    print("plain MCP inventory redaction and control safety: PASS")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--peer":
        peer(sys.argv[2])
    else:
        main()
