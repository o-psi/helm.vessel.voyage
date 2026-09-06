#!/usr/bin/env python3
"""Real CLI, native HTTP transports and durable completion acceptance regression."""
from __future__ import annotations
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_readiness import Case, MODEL

PROPOSAL = "premature-proposal-雪"
FINAL = "verified-final: evidence says 42"
GUIDANCE = "Final proposal"


def event(value):
    return ("data: " + json.dumps(value) + "\n\n").encode()


def response(provider, value, serial):
    tool = isinstance(value, tuple)
    name, args = value if tool else (None, None)
    call_id = f"gate_{serial}"
    if provider == "openai-responses":
        output = ([{"type": "function_call", "id": "fc_" + call_id, "call_id": call_id,
                    "name": name, "arguments": json.dumps(args)}] if tool else
                  [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": value}]}])
        return event({"type": "response.completed", "response": {"output": output,
                      "usage": {"input_tokens": 1, "output_tokens": 1}}})
    if provider == "openai-chat":
        delta = ({"tool_calls": [{"index": 0, "id": call_id, "type": "function", "function": {
            "name": name, "arguments": json.dumps(args)}}]} if tool else {"content": value})
        return event({"choices": [{"delta": delta}]}) + b"data: [DONE]\n\n"
    frames = [{"type": "message_start", "message": {"usage": {"input_tokens": 1}}}]
    if tool:
        frames += [{"type": "content_block_start", "index": 0, "content_block": {
            "type": "tool_use", "id": call_id, "name": name}},
            {"type": "content_block_delta", "index": 0, "delta": {
                "type": "input_json_delta", "partial_json": json.dumps(args)}}]
    else:
        frames += [{"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": value}}]
    frames += [{"type": "message_delta", "usage": {"output_tokens": 1}}, {"type": "message_stop"}]
    return b"".join(event(frame) for frame in frames)


def normalize(provider, body):
    messages = body.get("input", body.get("messages", []))
    outputs = []
    systems = [str(body.get("system", "")), str(body.get("instructions", ""))]
    for item in messages:
        if item.get("role") == "system":
            systems.append(str(item["content"]))
        if item.get("type") == "function_call_output":
            outputs.append(item["output"])
        if item.get("role") == "tool":
            outputs.append(item["content"])
        if isinstance(item.get("content"), list):
            for block in item["content"]:
                if block.get("type") == "tool_result":
                    content = block["content"]
                    outputs.append(content if isinstance(content, str) else "".join(p.get("text", "") for p in content))
    return outputs, "\n".join(systems)


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({"data": [{"id": MODEL}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        case = self.server.case
        status = 200
        try:
            expected = {"openai-chat": "/v1/chat/completions", "openai-responses": "/v1/responses", "anthropic": "/v1/messages"}[case.provider]
            assert self.path == expected, self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            step = len(case.requests)
            case.requests.append(body)
            outputs, systems = normalize(case.provider, body)
            human_text = []
            for message in body.get("input", body.get("messages", [])):
                if message.get("role") != "user":
                    continue
                content = message.get("content", "")
                if isinstance(content, list):
                    content = "".join(block.get("text", "") for block in content if block.get("type") in ("text", "input_text"))
                if content:
                    human_text.append(content)
            expected_humans = (["gate-fixture:verified"] if case.mode == "resume" else []) + ["gate-fixture:" + case.mode]
            assert human_text == expected_humans, ("synthetic human guidance", human_text)
            saved = case.latest_session()
            assert [m["content"] for m in saved["messages"] if m["role"] == "user"][-1] == "gate-fixture:" + case.mode
            assert saved["run_summaries"][-1]["phase"] in ("provisional", "reconciling")
            if step >= 2:
                # Check disk at the actual provider boundary, not only after exit.
                assert any(m["content"] == PROPOSAL for m in saved["messages"]), "reconciliation dispatched before canonical proposal checkpoint"
                assert "reconcil" in systems.lower() and case.todo in systems, systems
                assert not any(m["role"] == "system" for m in saved["messages"])
            value = case.respond(step, outputs)
            if value == "FAIL":
                status, payload = 400, b'{"error":{"message":"offline injected provider failure"}}'
            else:
                payload = response(case.provider, value, step)
        except Exception as error:
            case.failures.append(repr(error))
            payload = response(case.provider, "fixture-failed", 999)
        self.send_response(status)
        self.send_header("Content-Type", "text/event-stream" if status == 200 else "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        try:
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            if case.mode != "cancel":
                case.failures.append("unexpected provider disconnect")


class GateCase(Case):
    def __init__(self, root, helm, port, provider, mode):
        super().__init__(root, helm, port)
        self.provider, self.mode = provider, mode
        self.config.write_text(self.config.read_text().replace('provider = "openai-responses"', f'provider = "{provider}"'))
        (root / "evidence.txt").write_text("Measured records: 42\n", encoding="utf-8")
        self.requests, self.failures = [], []
        self.hold, self.release = threading.Event(), threading.Event()
        self.todo = None
        self.snapshot = None

    def respond(self, step, outputs):
        if self.mode in ("empty", "resume"):
            assert step == 0
            return FINAL
        if step == 0:
            return "todo", {"action": "create", "title": "Verify evidence"}
        if step == 1:
            self.todo = json.loads(outputs[-1])["id"]
            return PROPOSAL
        if self.mode == "ignore":
            assert step == 2
            return "still falsely claiming everything completed"
        if self.mode == "failure":
            assert step == 2
            return "FAIL"
        if self.mode == "cancel":
            assert step == 2
            self.hold.set()
            assert self.release.wait(15)
            return "must never be accepted after cancellation"
        if step == 2:
            return "completion", {"action": "read", "kind": "todo", "id": self.todo}
        if step == 3:
            record = json.loads(outputs[-1])
            assert record["id"] == self.todo and record["status"] == "pending"
            if self.mode == "verified":
                return "read_file", {"path": "evidence.txt"}
            if self.mode == "blocked":
                return "todo", {"action": "block", "id": self.todo, "blockers": ["external fixture prerequisite unavailable"]}
            return "completion", {"action": "snapshot"}
        if self.mode == "verified" and step == 4:
            assert "Measured records: 42" in outputs[-1], outputs[-1]
            return "todo", {"action": "evidence", "id": self.todo, "text": "Read evidence.txt: measured records 42"}
        if self.mode == "verified" and step == 5:
            return "todo", {"action": "status", "id": self.todo, "status": "completed"}
        snapshot_step = {"verified": 6, "blocked": 4, "deferred": 3}[self.mode]
        if step == snapshot_step:
            return "completion", {"action": "snapshot"}
        if step == snapshot_step + 1:
            self.snapshot = json.loads(outputs[-1])
            assert self.snapshot["total"] == 1 and self.snapshot["accounted"] == 0
            return "completion", {"action": "read", "kind": "todo", "id": self.todo}
        if step == snapshot_step + 2:
            record = json.loads(outputs[-1])
            assert record["id"] == self.todo
            disposition = {"verified": "completed_with_evidence", "blocked": "blocked_with_impact", "deferred": "deferred_with_impact"}[self.mode]
            return "completion", {"action": "account", "kind": "todo", "id": self.todo,
                "revision": self.snapshot["revision"], "fingerprint": self.snapshot["fingerprint"],
                "disposition": disposition, "reason": "Verified arithmetic evidence 42" if self.mode == "verified" else "External prerequisite unavailable; operator must complete pending verification"}
        assert step == snapshot_step + 3, (self.mode, step, outputs)
        assert not outputs[-1].startswith("Error"), outputs[-1]
        return FINAL if self.mode == "verified" else "Verification remains " + self.mode + "; operator action required"

    def execute(self):
        proc = subprocess.Popen(self.command("run", "gate-fixture:" + self.mode), cwd=self.root,
                                env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            if self.mode == "cancel":
                assert self.hold.wait(15), (self.failures, "cancel never reached reconciliation")
                proc.send_signal(signal.SIGINT)
            stdout, stderr = proc.communicate(timeout=20)
        finally:
            self.release.set()
            if proc.poll() is None:
                proc.kill()
                proc.communicate(timeout=5)
        assert not self.failures, self.failures
        success = self.mode in ("empty", "verified")
        assert (proc.returncode == 0) == success, (self.mode, proc.returncode, stdout, stderr)
        saved = self.latest_session()
        phase = "completed" if success else "interrupted" if self.mode in ("failure", "cancel") else "incomplete"
        assert saved["run_summaries"][-1]["phase"] == phase, saved["run_summaries"]
        assert not saved["run_summaries"][-1].get("partial_output"), "canonical proposal duplicated as unfinished partial text"
        assert not any(m["role"] == "system" for m in saved["messages"])
        assert len([m for m in saved["messages"] if m["role"] == "user"]) == 1
        texts = [m["content"] for m in saved["messages"] if m["role"] == "assistant" and not m.get("tool_calls")]
        assert texts.count(PROPOSAL) == (0 if self.mode == "empty" else 1), texts
        assert texts.count(FINAL) == int(success), texts
        expected = {"empty": 1, "verified": 10, "blocked": 8, "deferred": 7, "ignore": 3, "failure": 3, "cancel": 3}
        assert len(self.requests) == expected[self.mode], (self.mode, len(self.requests))
        if self.mode not in ("failure", "cancel"):
            decision = self.ledger(saved["completion_runs"][-1])["state"]["decision"]
            assert decision["outcome"] == ("completed" if success else "incomplete"), decision
        if self.mode == "verified":
            # Restart is a fresh run; canonical proposals and classification survive.
            before = saved
            self.requests = []
            self.mode = "resume"
            result = subprocess.run(self.command("run", "--resume", saved["id"], "gate-fixture:resume"), cwd=self.root,
                                    env=self.env, text=True, capture_output=True, timeout=20)
            assert result.returncode == 0, result.stderr
            assert not self.failures, self.failures
            after = self.latest_session()
            assert after["messages"][:len(before["messages"])] == before["messages"]
            assert after["run_summaries"][0] == before["run_summaries"][0]
            assert len(after["completion_runs"]) == 2 and len(self.requests) == 1


def main():
    helm = Path(os.environ.get("HELM_BIN", Path(__file__).resolve().parents[2] / "target/release/helm")).resolve()
    assert helm.is_file(), helm
    server = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for provider in ("openai-chat", "openai-responses", "anthropic"):
            for mode in ("empty", "verified", "blocked", "deferred", "ignore", "failure", "cancel"):
                with tempfile.TemporaryDirectory(prefix="helm-gate-") as raw:
                    case = GateCase(Path(raw), helm, server.server_port, provider, mode)
                    server.case = case
                    case.execute()
                    print(f"completion gate: {provider} {mode} passed", flush=True)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
