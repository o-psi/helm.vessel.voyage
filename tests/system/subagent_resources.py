#!/usr/bin/env python3
"""Offline Responses regression: nested scheduling, removed budgets, and durable results."""

import itertools
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


WORKERS = 10
MARKER = "resource-fixture:"


def run_resources(helm: Path) -> None:
    class Fixture(BaseHTTPRequestHandler):
        phase = "create"
        failures = []
        workers = []
        coordinator_id = None
        leaf_id = None
        serial = itertools.count()
        calls = {}
        lock = threading.Lock()
        denied = False
        delayed_seconds = 0.0

        def log_message(self, *_args):
            pass

        def do_POST(self):
            try:
                assert self.path == "/v1/responses", self.path
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                outputs = [item for item in body["input"]
                           if item.get("type") == "function_call_output"]
                users = " ".join(str(item.get("content", "")) for item in body["input"]
                                 if item.get("role") == "user")
                match = re.search(r"resource-fixture:(coordinator|worker-\d+|leaf)", users)
                actor = match.group(1) if match else "root"
                with Fixture.lock:
                    Fixture.calls[actor] = Fixture.calls.get(actor, 0) + 1
                if actor != "root":
                    assert Fixture.phase == "create", "restart lookup executed a child"
                if actor == "coordinator":
                    output = self.coordinator(outputs)
                elif actor.startswith("worker-"):
                    output = self.worker(actor, outputs)
                elif actor == "leaf":
                    assert not outputs
                    output = self.message("leaf-evidence-α")
                elif Fixture.phase == "create":
                    output = self.root_create(outputs)
                else:
                    output = self.root_lookup(outputs)
            except Exception as error:
                with Fixture.lock:
                    Fixture.failures.append(repr(error))
                output = self.message("fixture-failed")
            frame = {"type": "response.completed", "response": {
                "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}
            encoded = f"data: {json.dumps(frame)}\n\n".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            try:
                self.wfile.write(encoded)
            except (BrokenPipeError, ConnectionResetError) as error:
                with Fixture.lock:
                    Fixture.failures.append(f"provider response disconnected: {error!r}")

        @staticmethod
        def message(text):
            return [{"type": "message", "role": "assistant",
                     "content": [{"type": "output_text", "text": text}]}]

        @staticmethod
        def call(args, name="subagent"):
            with Fixture.lock:
                serial = next(Fixture.serial)
            return [{"type": "function_call", "id": f"fc_{serial}",
                     "call_id": f"call_{serial}", "name": name,
                     "arguments": json.dumps(args)}]

        @staticmethod
        def result(output):
            value = json.loads(output["output"])
            assert value["status"] == "completed", value
            return value["result"]["summary"]

        @staticmethod
        def all_ids():
            return [Fixture.coordinator_id, *Fixture.workers, Fixture.leaf_id]

        def coordinator(self, outputs):
            step = len(outputs)
            if 1 <= step <= WORKERS:
                raw = outputs[-1]["output"]
                assert raw.startswith("{"), raw
                spawned = json.loads(raw)
                assert spawned["status"] == "queued", spawned
                Fixture.workers.append(spawned["id"])
            if step < WORKERS:
                return self.call({"action": "spawn", "name": f"worker-{step}",
                                  "task": f"{MARKER}worker-{step}"})
            if step == WORKERS:
                assert len(set(Fixture.workers)) == WORKERS
                return self.call({"action": "wait_many", "ids": Fixture.workers})
            assert step == WORKERS + 1, step
            results = json.loads(outputs[-1]["output"])
            assert results["ids"] == Fixture.workers, results
            assert len(results["results"]) == WORKERS, results
            for index, result in enumerate(results["results"]):
                assert result["status"] == "completed", result
                assert result["result"]["summary"] == f"worker-{index}-evidence", result
            return self.message("coordinator-complete")

        def worker(self, actor, outputs):
            if actor == "worker-0":
                if not outputs:
                    return self.call({"action": "spawn", "name": "leaf",
                                      "task": f"{MARKER}leaf"})
                if len(outputs) == 1:
                    Fixture.leaf_id = json.loads(outputs[-1]["output"])["id"]
                    return self.call({"action": "wait", "id": Fixture.leaf_id})
                assert len(outputs) == 2
                assert self.result(outputs[-1]) == "leaf-evidence-α"
            elif actor == "worker-1":
                assert not outputs
                # Deliberately exceed command_timeout_secs while model work is active.
                started = time.monotonic()
                time.sleep(1.5)
                Fixture.delayed_seconds = time.monotonic() - started
            elif actor == "worker-2":
                if not outputs:
                    return self.call({"path": "must-not-exist.txt", "content": "unauthorized"},
                                     name="write_file")
                assert len(outputs) == 1
                assert "denied:" in outputs[-1]["output"], outputs
                assert "`write_file` action is disabled in read-only access mode" in outputs[-1]["output"], outputs
                Fixture.denied = True
            else:
                assert not outputs
            return self.message(f"{actor}-evidence")

        def root_create(self, outputs):
            if not outputs:
                return self.call({"action": "spawn", "name": "coordinator",
                                  "task": f"{MARKER}coordinator"})
            if len(outputs) == 1:
                Fixture.coordinator_id = json.loads(outputs[-1]["output"])["id"]
                return self.call({"action": "wait", "id": Fixture.coordinator_id})
            if len(outputs) == 2:
                assert self.result(outputs[-1]) == "coordinator-complete"
                return self.call({"action": "archive", "limit": 100})
            assert len(outputs) == 3
            self.check_page(outputs[-1])
            return self.message("resource-fixture-created")

        def check_page(self, output):
            page = json.loads(output["output"])
            assert page["next_after"] is None, page
            assert {agent["id"] for agent in page["agents"]} == set(self.all_ids()), page
            assert all(agent["status"] == "completed" for agent in page["agents"]), page

        def root_lookup(self, outputs):
            step = len(outputs)
            if step == 0:
                return self.call({"action": "list"})
            if step == 1:
                assert json.loads(outputs[-1]["output"]) == []
                return self.call({"action": "archive", "limit": 100})
            if step == 2:
                self.check_page(outputs[-1])
                return self.call({"action": "status", "id": Fixture.leaf_id})
            if step == 3:
                record = json.loads(outputs[-1]["output"])
                assert record["archived"] is True, record
                assert record["parent_id"] == Fixture.workers[0], record
                assert record["result"] == "leaf-evidence-α", record
                return self.call({"action": "wait_many", "ids": Fixture.workers})
            assert step == 4, step
            results = json.loads(outputs[-1]["output"])["results"]
            assert len(results) == WORKERS
            for index, result in enumerate(results):
                assert result["status"] == "completed", result
                assert result["result"]["summary"] == f"worker-{index}-evidence", result
            return self.message("resource-fixture-restored")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-subagent-resources-") as temporary:
            root = Path(temporary)
            config = root / "config.toml"
            config.write_text(
                'provider = "openai-responses"\nmodel = "fixture"\n'
                'api_key_env = "HELM_FIXTURE_KEY"\naccess = "read-only"\n'
                f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                'command_timeout_secs = 1\nsubagent_max_agents = 1\n'
                'subagent_max_concurrency = 1\nprovider_retry_attempts = 1\n')
            env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
                       XDG_DATA_HOME=str(root / "data"), HELM_FIXTURE_KEY="offline-fixture")
            command = [str(helm), "--config", str(config), "--workspace", str(root), "run"]
            first = subprocess.run(command + ["Exercise nested delegated resource behavior."],
                                   cwd=root, env=env, capture_output=True, text=True, timeout=60)
            assert first.returncode == 0 and "resource-fixture-created" in first.stdout, first.stderr + first.stdout + repr(Fixture.failures)
            assert not Fixture.failures, Fixture.failures
            assert Fixture.denied and not (root / "must-not-exist.txt").exists()
            assert Fixture.delayed_seconds > 1, Fixture.delayed_seconds
            assert len(set(Fixture.all_ids())) == WORKERS + 2
            archive_dir = root / "data" / "helm" / "subagents"
            archives = list(archive_dir.glob("*.archive/*.json"))
            assert len(archives) == WORKERS + 2, archives
            records = {record["id"]: record for record in
                       (json.loads(path.read_text())["record"] for path in archives)}
            assert set(records) == set(Fixture.all_ids())
            assert all(record["status"] == "completed" for record in records.values())
            assert all(records[worker]["parent_id"] == Fixture.coordinator_id for worker in Fixture.workers)
            assert records[Fixture.leaf_id]["parent_id"] == Fixture.workers[0]
            assert records[Fixture.leaf_id]["result"] == "leaf-evidence-α"
            assert all("max_runtime_secs" not in record["budget"] and
                       "max_children" not in record["budget"] for record in records.values())
            child_calls = {actor: count for actor, count in Fixture.calls.items() if actor != "root"}
            assert len(child_calls) == WORKERS + 2, child_calls
            archive_contents = {path: path.read_bytes() for path in archives}
            Fixture.phase = "lookup"
            second = subprocess.run(command + ["Look up the existing delegated results."],
                                    cwd=root, env=env, capture_output=True, text=True, timeout=30)
            assert second.returncode == 0 and "resource-fixture-restored" in second.stdout, second.stderr + second.stdout + repr(Fixture.failures)
            assert not Fixture.failures, Fixture.failures
            assert child_calls == {actor: count for actor, count in Fixture.calls.items() if actor != "root"}
            assert all(path.read_bytes() == contents for path, contents in archive_contents.items())
        print("subagent resources: ok (concurrency=1, nested wait/wait_many, 10 children, "
              "command-timeout independence, read-only denial, ignored capacity, restart lookup)")
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    repository = Path(__file__).resolve().parents[2]
    run_resources(Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve())
