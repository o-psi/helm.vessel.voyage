#!/usr/bin/env python3
"""Offline CLI regression for run-owned readiness; final reconciliation is separate."""
from __future__ import annotations

import hashlib
import itertools
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


MODEL = "fixture-completion-ownership"
MARKER = "ownership-fixture:"


def message(text: str) -> list[dict]:
    return [{"type": "message", "role": "assistant",
             "content": [{"type": "output_text", "text": text}]}]


class Fixture(BaseHTTPRequestHandler):
    case: Case
    requests: list[dict] = []
    failures: list[str] = []
    counts: dict[str, int] = {}
    references: dict[str, dict] = {}
    todos: dict[str, str] = {}
    agents: dict[str, str] = {}
    snapshots: dict[str, list[dict]] = {}
    serial = itertools.count()
    lock = threading.Lock()
    hold_started = threading.Event()
    release_hold = threading.Event()

    def log_message(self, *_args: object) -> None:
        pass

    def do_GET(self) -> None:
        # Do not advertise the utility title model: title dispatch is independent
        # of run ownership and must not distort exact request-count assertions.
        if self.path != "/v1/models":
            self.send_error(404)
            return
        encoded = json.dumps({"data": [{"id": MODEL}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_POST(self) -> None:
        try:
            assert self.path == "/v1/responses", self.path
            assert self.headers.get("Authorization") == "Bearer offline-ownership-key"
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert body["model"] == MODEL, body["model"]
            users = [(index, str(item.get("content", "")))
                     for index, item in enumerate(body["input"]) if item.get("role") == "user"]
            start, prompt = users[-1]
            actor = re.search(r"ownership-fixture:([a-z-]+)", prompt).group(1)
            outputs = [item["output"] for item in body["input"][start:]
                       if item.get("type") == "function_call_output"]
            with self.lock:
                step = self.counts.get(actor, 0)
                self.counts[actor] = step + 1
                self.requests.append({"actor": actor, "body": body})
            # The frontend must durably publish a run reference before dispatch,
            # including when the very first provider response has not arrived.
            saved = self.case.latest_session()
            if actor not in ("branch", "leaf"):
                accepted = [item["content"] for item in saved["messages"] if item["role"] == "user"]
                assert accepted and MARKER + actor in accepted[-1], "provider dispatched before accepted prompt publication"
            reference = saved["completion_runs"][-1]
            assert reference["session_id"] == saved["id"], reference
            assert self.case.ledger_path(reference).is_file(), reference
            self.references.setdefault(actor, reference)
            assert self.references[actor] == reference, (actor, reference)
            output = self.respond(actor, step, outputs)
        except Exception as error:
            with self.lock:
                self.failures.append(repr(error))
            output = message("fixture-failed")
        encoded = ("data: " + json.dumps({"type": "response.completed", "response": {
            "output": output, "usage": {"input_tokens": 1, "output_tokens": 1}}}) + "\n\n").encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        try:
            self.wfile.write(encoded)
        except (BrokenPipeError, ConnectionResetError) as error:
            self.failures.append(f"fixture response disconnected: {error!r}")

    @classmethod
    def call(cls, name: str, args: dict) -> list[dict]:
        with cls.lock:
            serial = next(cls.serial)
        return [{"type": "function_call", "id": f"fc_ownership_{serial}",
                 "call_id": f"ownership_{serial}", "name": name,
                 "arguments": json.dumps(args)}]

    @classmethod
    def snapshot(cls, actor: str, outputs: list[str], total: int) -> dict:
        value = json.loads(outputs[-1])
        assert value["run_id"] == cls.references[actor]["run_id"], value
        assert value["total"] == total, value
        assert value["accounted"] == 0, value
        cls.snapshots.setdefault(actor, []).append(value)
        return value

    def respond(self, actor: str, step: int, outputs: list[str]) -> list[dict]:
        if actor == "create":
            if step == 0:
                return self.call("todo", {"action": "create", "title": "first-run-owned"})
            if step == 1:
                self.todos[actor] = json.loads(outputs[-1])["id"]
                return self.call("completion", {"action": "snapshot"})
            assert step == 2, (actor, step)
            snapshot = self.snapshot(actor, outputs, 1)
            assert snapshot["unresolved"][0]["obligation"] == {"todo": self.todos[actor]}, snapshot
            return message("first-run-finished-with-unresolved-todo")
        if actor == "resume":
            if step == 0:
                return self.call("completion", {"action": "snapshot"})
            if step == 1:
                self.snapshot(actor, outputs, 0)
                return self.call("todo", {"action": "create", "title": "second-run-owned"})
            if step == 2:
                self.todos[actor] = json.loads(outputs[-1])["id"]
                return self.call("completion", {"action": "snapshot"})
            assert step == 3, (actor, step)
            snapshot = self.snapshot(actor, outputs, 1)
            assert snapshot["unresolved"][0]["obligation"] == {"todo": self.todos[actor]}, snapshot
            return message("resumed-run-has-independent-ownership")
        if actor == "empty":
            assert step == 0 and not outputs, (actor, step, outputs)
            return message("empty-run-needs-no-plan")
        if actor == "tree":
            if step == 0:
                return self.call("completion", {"action": "snapshot"})
            if step == 1:
                self.snapshot(actor, outputs, 0)
                return self.call("subagent", {"action": "spawn", "name": "owned-branch",
                                              "task": MARKER + "branch"})
            if step == 2:
                self.agents["branch"] = json.loads(outputs[-1])["id"]
                return self.call("subagent", {"action": "wait", "id": self.agents["branch"]})
            if step == 3:
                assert "branch-owned-evidence" in outputs[-1], outputs[-1]
                return self.call("completion", {"action": "snapshot"})
            assert step == 4, (actor, step)
            snapshot = self.snapshot(actor, outputs, 4)
            assert {json.dumps(item["obligation"], sort_keys=True) for item in snapshot["unresolved"]} == {
                json.dumps({"agent": value}, sort_keys=True) for value in self.agents.values()
            } | {json.dumps({"todo": self.todos[key]}, sort_keys=True) for key in ("branch", "leaf")}
            return message("root-observed-all-descendant-obligations")
        if actor == "branch":
            if step == 0:
                return self.call("todo", {"action": "create", "title": "branch-owned-todo"})
            if step == 1:
                self.todos[actor] = json.loads(outputs[-1])["id"]
                return self.call("subagent", {"action": "spawn", "name": "owned-leaf",
                                              "task": MARKER + "leaf"})
            if step == 2:
                self.agents["leaf"] = json.loads(outputs[-1])["id"]
                return self.call("subagent", {"action": "wait", "id": self.agents["leaf"]})
            if step == 3:
                assert "leaf-owned-evidence" in outputs[-1], outputs[-1]
                return self.call("completion", {"action": "snapshot"})
            assert step == 4, (actor, step)
            self.snapshot(actor, outputs, 4)
            return message("branch-owned-evidence")
        if actor == "leaf":
            if step == 0:
                return self.call("todo", {"action": "create", "title": "leaf-owned-todo"})
            if step == 1:
                self.todos[actor] = json.loads(outputs[-1])["id"]
                return self.call("completion", {"action": "snapshot"})
            assert step == 2, (actor, step)
            self.snapshot(actor, outputs, 4)
            return message("leaf-owned-evidence")
        if actor == "hold":
            assert step == 0 and not outputs
            self.hold_started.set()
            assert self.release_hold.wait(15), "fixture owner was never released"
            return message("owner-released")
        raise AssertionError(f"unexpected provider dispatch: {actor}")


class Case:
    def __init__(self, root: Path, helm: Path, port: int):
        self.root = root
        self.helm = helm
        self.config = root / "config.toml"
        self.config.write_text(f'''provider = "openai-responses"
model = "{MODEL}"
api_key_env = "HELM_OWNERSHIP_KEY"
base_url = "http://127.0.0.1:{port}/v1"
provider_retry_attempts = 1
max_tokens = 1024
context_window = 65536
subagent_max_concurrency = 1
approval = "never"
''', encoding="utf-8")
        self.env = os.environ.copy()
        self.env.update(HELM_OWNERSHIP_KEY="offline-ownership-key", HOME=str(root / "home"),
                        XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"))
        self.key = hashlib.sha256(os.fsencode(root.resolve())).hexdigest()

    def command(self, *args: str) -> list[str]:
        return [str(self.helm), "--config", str(self.config), "--workspace", str(self.root), *args]

    def invoke(self, *args: str, succeeds: bool = True) -> subprocess.CompletedProcess:
        result = subprocess.run(self.command(*args), cwd=self.root, env=self.env,
                                text=True, capture_output=True, timeout=30)
        assert (result.returncode == 0) == succeeds, (args, result.stdout, result.stderr)
        assert not Fixture.failures, Fixture.failures
        return result

    def run(self, actor: str, resume: str | None = None) -> tuple[str, dict]:
        args = ["run"] + (["--resume", resume] if resume else []) + [MARKER + actor]
        result = self.invoke(*args)
        match = re.search(r"\[session ([0-9a-f-]{36})\]", result.stderr)
        assert match, result.stderr
        session_id = match.group(1)
        return session_id, self.saved(session_id)

    def saved(self, session_id: str) -> dict:
        return json.loads((self.root / "data/helm/sessions" / f"{session_id}.json").read_text())

    def latest_session(self) -> dict:
        paths = list((self.root / "data/helm/sessions").glob("*.json"))
        assert paths, "provider dispatched before session publication"
        return json.loads(max(paths, key=lambda path: path.stat().st_mtime_ns).read_text())

    def ledger_path(self, reference: dict) -> Path:
        return self.root / "data/helm/completion" / self.key / "ledgers" / (
            f'{reference["session_id"]}-{reference["run_id"]}.json')

    def ledger(self, reference: dict) -> dict:
        envelope = json.loads(self.ledger_path(reference).read_text())
        assert envelope["scope"]["workspace"] == str(self.root.resolve()), envelope["scope"]
        assert envelope["scope"]["session_id"] == reference["session_id"], envelope["scope"]
        value = json.loads(envelope["ledger"])
        assert value["run_id"] == reference["run_id"], value
        return value


def main() -> None:
    repository = Path(__file__).resolve().parents[2]
    helm = Path(os.environ.get("HELM_BIN", repository / "target/release/helm")).resolve()
    assert helm.is_file(), f"Build Helm first or set HELM_BIN: {helm}"
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="helm-completion-readiness-") as raw:
            case = Case(Path(raw), helm, server.server_port)
            Fixture.case = case
            session_id, first = case.run("create")
            assert len(first["completion_runs"]) == 1, first["completion_runs"]
            assert len([item for item in first["messages"] if item["role"] == "user"]) == 1
            first_reference = first["completion_runs"][0]
            first_ledger = case.ledger(first_reference)
            assert [entry["obligation"] for entry in first_ledger["entries"]] == [{"todo": Fixture.todos["create"]}]
            assert first_ledger["entries"][0]["dispositions"] == [], first_ledger
            _, resumed = case.run("resume", session_id)
            assert resumed["messages"][:len(first["messages"])] == first["messages"]
            assert len(resumed["completion_runs"]) == 2, resumed["completion_runs"]
            assert len([item for item in resumed["messages"] if item["role"] == "user"]) == 2
            second_reference = resumed["completion_runs"][1]
            assert first_reference["run_id"] != second_reference["run_id"]
            assert [entry["obligation"] for entry in case.ledger(second_reference)["entries"]] == [{"todo": Fixture.todos["resume"]}]
            assert case.ledger(first_reference) == first_ledger, "later run mutated earlier obligations"
            count = len(Fixture.requests)
            ledger_path = case.ledger_path(first_reference)
            original = ledger_path.read_bytes()
            try:
                ledger_path.unlink()
                failure = case.invoke("run", "--resume", session_id, MARKER + "missing", succeeds=False)
                assert "completion" in failure.stderr.lower(), failure.stderr
                assert len(Fixture.requests) == count, "missing known ledger reached provider"
                assert case.saved(session_id) == resumed, "failed recovery published another run"
                ledger_path.write_bytes(b"{broken-json")
                ledger_path.chmod(0o600)
                failure = case.invoke("run", "--resume", session_id, MARKER + "corrupt", succeeds=False)
                assert "completion" in failure.stderr.lower(), failure.stderr
                assert len(Fixture.requests) == count, "corrupt known ledger reached provider"
            finally:
                ledger_path.write_bytes(original)
                ledger_path.chmod(0o600)
            _, empty = case.run("empty")
            assert Fixture.counts["empty"] == 1, Fixture.counts
            assert case.ledger(empty["completion_runs"][0])["entries"] == []
            _, tree = case.run("tree")
            tree_reference = tree["completion_runs"][0]
            assert Fixture.references["tree"] == Fixture.references["branch"] == Fixture.references["leaf"] == tree_reference
            tree_ledger = case.ledger(tree_reference)
            assert len(tree_ledger["entries"]) == 4, tree_ledger
            assert all(not entry["dispositions"] for entry in tree_ledger["entries"]), tree_ledger
            records = {}
            for path in (case.root / "data/helm/subagents").rglob("*.json"):
                value = json.loads(path.read_text())
                if "record" in value:
                    records[value["record"]["id"]] = value["record"]
                records.update(value.get("agents", {}))
            for actor in ("branch", "leaf"):
                record = records[Fixture.agents[actor]]
                assert record["completion"] == tree_reference, record
                assert record["status"] == "completed", record
                assert record["result"] == f"{actor}-owned-evidence", record
            assert records[Fixture.agents["leaf"]]["parent_id"] == Fixture.agents["branch"]
            todos = json.loads((case.root / "data/helm/todos" / f"{case.key}.json").read_text())["items"]
            assert len(todos) == 4, todos
            assert all(item["status"] == "pending" for item in todos.values()), todos
            # Hold one real frontend open while another process tries the same
            # workspace. The second must fail before recovery or provider dispatch.
            before = len(Fixture.requests)
            owner = subprocess.Popen(case.command("run", MARKER + "hold"), cwd=case.root,
                                     env=case.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                assert Fixture.hold_started.wait(10), "owner never reached provider"
                failure = case.invoke("run", MARKER + "busy-attempt", succeeds=False)
                assert "busy" in failure.stderr.lower(), failure.stderr
                assert len(Fixture.requests) == before + 1, "second runtime dispatched despite owner lease"
            finally:
                Fixture.release_hold.set()
                try:
                    stdout, stderr = owner.communicate(timeout=10)
                except subprocess.TimeoutExpired:
                    owner.kill()
                    owner.communicate(timeout=5)
                    raise
            assert owner.returncode == 0 and "owner-released" in stdout, (stdout, stderr)
            assert not Fixture.failures, Fixture.failures
            assert Fixture.counts == {"create": 3, "resume": 4, "empty": 1,
                                      "tree": 5, "branch": 5, "leaf": 3, "hold": 1}, Fixture.counts
            print("completion readiness: CLI/resume isolation, durable references, fail-closed recovery, nested ownership and runtime lease passed")
    finally:
        Fixture.release_hold.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
