#!/usr/bin/env python3
"""Offline native-provider flow: automatic archival, restart, and original-ID lookup."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def run_archive(helm: Path) -> None:
    class Fixture(BaseHTTPRequestHandler):
        phase = "create"
        calls = 0
        child_calls = 0
        ids = []
        failures = []

        def log_message(self, *_args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            outputs = [item for item in body["input"] if item.get("type") == "function_call_output"]
            # Child requests carry their explicit task, never the parent prompt.
            child = any("archive-fixture-child" in str(item.get("content", ""))
                        for item in body["input"] if item.get("role") == "user")
            try:
                if child:
                    Fixture.child_calls += 1
                    output = self.message("archived-evidence-α")
                else:
                    Fixture.calls += 1
                    actions = (self.creation_actions(outputs) if Fixture.phase == "create"
                               else self.lookup_actions(outputs))
                    output = actions
            except Exception as error:
                Fixture.failures.append(repr(error))
                output = self.message("fixture-failed")
            frame = {"type": "response.completed", "response": {
                "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}
            encoded = f"data: {json.dumps(frame)}\n\n".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        @staticmethod
        def message(text):
            return [{"type": "message", "role": "assistant",
                     "content": [{"type": "output_text", "text": text}]}]

        @staticmethod
        def call(args):
            key = f"{Fixture.phase}_{Fixture.calls}"
            return [{"type": "function_call", "id": f"fc_{key}", "call_id": f"call_{key}",
                     "name": "subagent", "arguments": json.dumps(args)}]

        def creation_actions(self, outputs):
            step = len(outputs)
            if step and step % 2 == 1:
                spawned = json.loads(outputs[-1]["output"])
                Fixture.ids.append(spawned["id"])
                return self.call({"action": "wait", "id": spawned["id"]})
            if step:
                assert json.loads(outputs[-1]["output"])["result"]["summary"] == "archived-evidence-α"
            if step < 6:
                return self.call({"action": "spawn", "name": f"child-{step}",
                                  "task": "archive-fixture-child"})
            return self.message("created-and-auto-archived")

        def lookup_actions(self, outputs):
            step = len(outputs)
            if step == 0:
                return self.call({"action": "list"})
            if step == 1:
                assert json.loads(outputs[-1]["output"]) == []
                return self.call({"action": "archive", "limit": 2})
            if step == 2:
                page = json.loads(outputs[-1]["output"])
                assert len(page["agents"]) == 2 and page["next_after"]
                self.server.first_page = [item["id"] for item in page["agents"]]
                return self.call({"action": "archive", "limit": 2, "after": page["next_after"]})
            if step == 3:
                page = json.loads(outputs[-1]["output"])
                assert len(page["agents"]) == 1 and page["next_after"] is None
                assert sorted(self.server.first_page + [page["agents"][0]["id"]]) == sorted(Fixture.ids)
                return self.call({"action": "status", "id": Fixture.ids[0]})
            if step == 4:
                record = json.loads(outputs[-1]["output"])
                assert record["archived"] is True and record["result"] == "archived-evidence-α"
                return self.call({"action": "wait_many", "ids": Fixture.ids})
            if step == 5:
                result = json.loads(outputs[-1]["output"])
                assert len(result["results"]) == 3
                assert all(item["result"]["summary"] == "archived-evidence-α" for item in result["results"])
                return self.message("referenced " + Fixture.ids[0] + ": archived-evidence-α")
            raise AssertionError("unexpected extra provider request")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-subagent-archive-") as temporary:
            root = Path(temporary)
            config = root / "config.toml"
            config.write_text(
                'provider = "openai-responses"\nmodel = "fixture"\n'
                'api_key_env = "HELM_FIXTURE_KEY"\naccess = "read-only"\n'
                f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                'subagent_max_agents = 1\nsubagent_max_concurrency = 1\n'
                'provider_retry_attempts = 1\n')
            env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                       XDG_DATA_HOME=str(root / "data"), HELM_FIXTURE_KEY="offline-fixture")
            command = [str(helm), "--config", str(config), "--workspace", str(root), "run"]
            first = subprocess.run(command + ["Delegate three tasks sequentially."], cwd=root, env=env,
                                   capture_output=True, text=True, timeout=30)
            assert first.returncode == 0 and "created-and-auto-archived" in first.stdout, first.stderr + first.stdout
            assert not Fixture.failures, Fixture.failures
            assert Fixture.child_calls == 3 and len(set(Fixture.ids)) == 3
            archives = list((root / "data" / "helm" / "subagents").glob("*.archive/*.json"))
            assert len(archives) == 3, archives
            for path in archives:
                assert json.loads(path.read_text())["record"]["result"] == "archived-evidence-α"
            Fixture.phase = "lookup"
            second = subprocess.run(command + ["Find and cite prior child results."], cwd=root, env=env,
                                    capture_output=True, text=True, timeout=30)
            assert second.returncode == 0 and "referenced " + Fixture.ids[0] in second.stdout, second.stderr + second.stdout
            assert not Fixture.failures, Fixture.failures
            assert Fixture.child_calls == 3, "lookup must not execute children"
        print("subagent archive: ok (automatic completion, capacity=1, restart, pagination, status, late waits, read-only citation)")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    repository = Path(__file__).resolve().parents[2]
    run_archive(Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve())
