#!/usr/bin/env python3
"""Offline CLI regression: real read tools continue beyond the former turn cap."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def run_no_turn_limit(helm: Path) -> None:
    class Fixture(BaseHTTPRequestHandler):
        calls = 0
        failures = []

        def log_message(self, *_args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            Fixture.calls += 1
            turn = Fixture.calls
            results = [item for item in body["input"] if item.get("type") == "function_call_output"]
            if len(results) != turn - 1 or any("long-loop-fixture" not in item["output"] for item in results):
                Fixture.failures.append(f"invalid tool history at cycle {turn}")
            output = ([{
                "type": "function_call", "id": f"fc_{turn}", "call_id": f"call_{turn}",
                "name": "read_file", "arguments": json.dumps({"path": "fixture.txt"}),
            }] if turn <= 70 else [{
                "type": "message", "role": "assistant",
                "content": [{"type": "output_text", "text": "long-loop-complete"}],
            }])
            frame = {"type": "response.completed", "response": {
                "output": output, "usage": {"input_tokens": 1, "output_tokens": 1},
            }}
            encoded = f"data: {json.dumps(frame)}\n\n".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-no-turn-limit-") as temporary:
            root = Path(temporary)
            (root / "fixture.txt").write_text("long-loop-fixture")
            config = root / "config.toml"
            config.write_text(
                'provider = "openai-responses"\nmodel = "fixture"\n'
                'api_key_env = "HELM_FIXTURE_KEY"\naccess = "read-only"\n'
                f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                # This test isolates turn-count behavior; budget the synthetic model
                # for the complete 71-request history. Context rejection has its own fixture.
                'context_window = 262144\n'
                'max_turns = 1\nprovider_retry_attempts = 1\n'
            )
            env = dict(os.environ, HOME=str(root / "home"),
                       XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
                       HELM_FIXTURE_KEY="offline-fixture")
            command = [str(helm), "--config", str(config), "--workspace", str(root)]
            result = subprocess.run(command + ["run", "Read the fixture repeatedly, then finish."],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode == 0, result.stderr
            assert "long-loop-complete" in result.stdout, result.stdout
            assert Fixture.calls == 71, Fixture.calls
            assert not Fixture.failures, Fixture.failures
            sessions = list((root / "data" / "helm" / "sessions").glob("*.json"))
            assert len(sessions) == 1, sessions
            saved = json.loads(sessions[0].read_text())
            assert len([m for m in saved["messages"] if m["role"] == "tool"]) == 70
            obsolete = subprocess.run(command + ["--set", "max_turns=64", "config"],
                                      cwd=root, env=env, capture_output=True, text=True, timeout=5)
            assert obsolete.returncode != 0
            assert "max_turns" in obsolete.stderr
        print("no turn limit: ok (71 CLI cycles, tool history, legacy config, obsolete override)")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    repository = Path(__file__).resolve().parents[2]
    run_no_turn_limit(Path(os.environ.get("HELM_BIN", repository / "target/debug/helm")).resolve())
