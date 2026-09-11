"""Offline Linux #64 native context-compaction process regressions.

Run ONLY in a coordinator-assigned binary/runtime slot:
    python3 -B voyage/tests/context_compaction.py --bin-dir /path/to/binaries
Uses real supervised voyages and loopback HTTP; no operator config or paid provider.
The deterministic provider tests runtime behavior, not model summarization quality.
Evidence stays in the shared fixture's private temporary directory. Source drafting
can be checked with compile(text, path, 'exec') without imports or bytecode writes.
"""
import argparse
import http.server
import json
from pathlib import Path
import select
import sys
import threading
import time
import uuid
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests"))
from voyage_fixture import Fixture, wait_for


class ContextFixture(Fixture):
    """Reuse process ownership/cleanup, with the current public Vessel contract."""
    def request(self, command, *, envelope=False):
        credential = json.loads((self.directory / "process-http.json").read_text())
        request = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=encoded({"protocol": 1, "command": command}),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=30) as incoming:
            payload = incoming.read(4 * 1024 * 1024 + 1)
        assert 0 < len(payload) <= 4 * 1024 * 1024
        reply = json.loads(payload)
        assert reply["protocol"] == 1
        if envelope:
            return reply
        assert reply.get("error") is None, reply
        return reply["result"]

    def raw_command(self, session, command):
        routing = {"session_id": session}
        if command["op"] == "cancel":
            routing["incarnation"] = self.request({"op": "inspect", "session_id": session})["incarnation"]
        envelope = self.request({**routing, **command}, envelope=True)
        if envelope.get("error") is not None:
            return {"error": envelope["error"], "result": None}
        reply = envelope["result"]
        self.sessions[session] = reply["incarnation"]
        return {"error": None, "result": reply["result"]}

    def new(self, configured=True):
        session = str(uuid.uuid4())
        self.sessions[session] = None
        command = {"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
                   "workspace": str(self.workspace), "config_path": str(self.config)}
        info = self.request(command)
        assert info["state"] in ("live", "suspended"), info
        self.sessions[session] = info["incarnation"]
        return session


CALL = "effect-once"
ANSWER = "Fixture completed without repeating the effect."
PARTIAL = "Visible partial answer before context rejection."
CONTEXT_ERROR = {"error": {"type": "invalid_request_error",
                           "code": "context_length_exceeded",
                           "message": "maximum context length exceeded: prompt is too long"}}
MODES = {"chat": ("openai-chat", "/v1/chat/completions"),
         "responses": ("openai-responses", "/v1/responses"),
         "anthropic": ("anthropic", "/v1/messages")}


def encoded(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def events(mode, *, tool=False, partial=False, call_id=CALL):
    arguments = {"command": "printf 'effect\\n' >> counter.txt; cat payload.txt"}
    if mode == "chat":
        delta = ({"tool_calls": [{"index": 0, "id": call_id, "type": "function",
                  "function": {"name": "shell", "arguments": json.dumps(arguments)}}]}
                 if tool else {"content": PARTIAL if partial else ANSWER})
        result = [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]}]
        if partial:
            return result + [CONTEXT_ERROR]
        return result + [{"choices": [{"index": 0, "delta": {},
                          "finish_reason": "tool_calls" if tool else "stop"}]}]
    if mode == "responses":
        if partial:
            return [{"type": "response.output_text.delta", "delta": PARTIAL},
                    {"type": "response.failed", "response": {
                        "status": "failed", "error": CONTEXT_ERROR["error"]}}]
        output = ([{"type": "function_call", "call_id": call_id, "name": "shell",
                    "arguments": json.dumps(arguments)}] if tool else
                  [{"type": "message", "role": "assistant", "content": [
                      {"type": "output_text", "text": ANSWER}]}])
        return [{"type": "response.completed", "response": {
            "id": "fixture-response", "status": "completed", "output": output,
            "usage": {"input_tokens": 1, "output_tokens": 1}}}]
    result = [{"type": "message_start", "message": {
        "id": "fixture-message", "type": "message", "role": "assistant",
        "model": "fixture-model", "content": [],
        "usage": {"input_tokens": 1, "output_tokens": 0}}}]
    block = ({"type": "tool_use", "id": call_id, "name": "shell", "input": {}}
             if tool else {"type": "text", "text": ""})
    delta = ({"type": "input_json_delta", "partial_json": json.dumps(arguments)}
             if tool else {"type": "text_delta", "text": PARTIAL if partial else ANSWER})
    result += [{"type": "content_block_start", "index": 0, "content_block": block},
               {"type": "content_block_delta", "index": 0, "delta": delta}]
    if partial:
        return result + [{"type": "error", **CONTEXT_ERROR}]
    return result + [{"type": "content_block_stop", "index": 0},
                     {"type": "message_delta", "delta": {
                         "stop_reason": "tool_use" if tool else "end_turn"},
                      "usage": {"output_tokens": 1}}, {"type": "message_stop"}]


def validate_request(body, mode, prompt):
    """Reject dangling/duplicated tool identities, but permit whole-group summaries."""
    assert body["stream"] is True
    if mode != "anthropic":
        assert not {"max_tokens", "max_completion_tokens", "max_output_tokens"}.intersection(body), "default output cap was sent"
    calls, results = [], []
    if mode == "responses":
        items = body["input"]
        users = [item for item in items if item.get("role") == "user"]
        for item in items:
            if item.get("type") == "function_call":
                calls.append(item["call_id"])
                assert isinstance(json.loads(item["arguments"]), dict)
            if item.get("type") == "function_call_output":
                results.append(item["call_id"])
                assert item["call_id"] in calls, "orphan Responses result"
    else:
        items = body["messages"]
        users = [item for item in items if item.get("role") == "user"]
        for item in items:
            if mode == "chat":
                for call in item.get("tool_calls", []):
                    calls.append(call["id"])
                    assert isinstance(json.loads(call["function"]["arguments"]), dict)
                if item.get("role") == "tool":
                    results.append(item["tool_call_id"])
                    assert item["tool_call_id"] in calls, "orphan Chat result"
            elif isinstance(item.get("content"), list):
                for block in item["content"]:
                    if block["type"] == "tool_use":
                        calls.append(block["id"])
                        assert isinstance(block["input"], dict)
                    if block["type"] == "tool_result":
                        results.append(block["tool_use_id"])
                        assert block["tool_use_id"] in calls, "orphan Anthropic result"
    assert prompt in json.dumps(users), "task text was lost"
    assert len(calls) == len(set(calls)), "duplicate call identity"
    assert calls == results, "incomplete/reordered tool group"


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        scenario = self.server.scenario
        try:
            assert self.path == MODES[scenario.mode][1], self.path
            self.connection.settimeout(10)
            size = int(self.headers["Content-Length"])
            assert 0 < size < 4 * 1024 * 1024, size
            body = json.loads(self.rfile.read(size))
            scenario.bodies.append(body)
            step = len(scenario.bodies)
            validate_request(body, scenario.mode, scenario.prompt)
            if scenario.kind == "long":
                completed = len(scenario.counter.read_text().splitlines()) if scenario.counter.exists() else 0
                if completed == 6 and not scenario.rejected:
                    scenario.rejected = len(encoded(body))
                    scenario.check_reduction = True
                    self.reject()
                else:
                    if scenario.check_reduction:
                        assert completed == 6, "effect repeated during context recovery"
                        assert len(encoded(body)) < scenario.rejected * 0.8
                        scenario.check_reduction = False
                    self.stream(events(scenario.mode, tool=completed < 12, call_id=f"loop-{completed}"))
            elif scenario.kind == "irreducible":
                assert step == 1, "unchanged irreducible request was retried"
                self.reject()
            elif step == 1:
                self.stream(events(scenario.mode, tool=True))
            elif step == 2:
                assert scenario.counter.read_text() == "effect\n"
                wire = encoded(body)
                if scenario.kind == "automatic":
                    assert len(wire) < len(scenario.payload.encode()) // 2, "large result not prepared"
                    assert scenario.middle.encode() not in wire
                    self.stream(events(scenario.mode))
                elif scenario.kind == "partial":
                    self.stream(events(scenario.mode, partial=True))
                elif scenario.kind == "manual":
                    assert scenario.middle.encode() in wire, "manual baseline already reduced"
                    self.stream(events(scenario.mode))
                else:
                    assert scenario.middle.encode() in wire, "rejection baseline already reduced"
                    self.reject()
            elif step == 3 and scenario.kind in ("rejection", "cancel", "manual"):
                old, new = encoded(scenario.bodies[1]), encoded(body)
                assert len(new) < len(old) * 0.75, (len(old), len(new))
                assert scenario.middle.encode() not in new, "omitted middle still sent"
                assert scenario.counter.read_text() == "effect\n", "tool effect repeated"
                if scenario.kind == "cancel":
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.end_headers()
                    self.wfile.flush()
                    scenario.held.set()
                    deadline = time.monotonic() + 20
                    while time.monotonic() < deadline and not scenario.stop.is_set():
                        if select.select([self.connection], [], [], 0.1)[0]:
                            assert self.connection.recv(1) == b""
                            scenario.disconnected.set()
                            return
                    raise AssertionError("cancel did not close held reduced request")
                self.stream(events(scenario.mode))
            elif step == 4 and scenario.kind == "rejection":
                assert len(encoded(body)) < len(encoded(scenario.bodies[1])) * 0.75
                assert scenario.middle.encode() not in encoded(body)
                assert scenario.counter.read_text() == "effect\n"
                self.stream(events(scenario.mode))
            else:
                raise AssertionError(f"unexpected request/replay {step} in {scenario.kind}")
        except (BrokenPipeError, ConnectionResetError) as error:
            if not scenario.stop.is_set():
                scenario.errors.append(repr(error))
        except Exception as error:
            scenario.errors.append(repr(error))
            self.send_error(500)

    def reject(self):
        error = CONTEXT_ERROR if self.server.scenario.mode != "anthropic" else {
            "type": "error", "error": {"type": "invalid_request_error", "message": "prompt is too long: 210000 tokens > 200000 maximum"}}
        payload = encoded(error)
        self.send_response(400)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def stream(self, values):
        payload = b"".join(b"data: " + encoded(value) + b"\n\n" for value in values)
        if self.server.scenario.mode == "chat":
            payload += b"data: [DONE]\n\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class Scenario:
    def __init__(self, fixture, mode, kind):
        self.mode, self.kind = mode, kind
        self.bodies, self.errors = [], []
        self.rejected, self.check_reduction = 0, False
        self.held, self.disconnected, self.stop = (threading.Event() for _ in range(3))
        self.middle = "CANONICAL-MIDDLE-MUST-SURVIVE-" + kind
        size = 256 * 1024 if kind == "automatic" else 24 * 1024
        self.payload = "BEGIN\n" + "a" * (size // 2) + self.middle + "z" * (size // 2) + "\nEND\n"
        self.prompt = "Run the fixture effect once and report its result."
        if kind == "irreducible":
            self.prompt = "Preserve this exact user input: " + "u" * (32 * 1024)
        self.counter = fixture.workspace / "counter.txt"
        (fixture.workspace / "payload.txt").write_text(self.payload)
        fixture.provider.scenario = self
        fixture.provider.RequestHandlerClass = Handler
        fixture.config.write_text(
            f'provider = "{MODES[mode][0]}"\nmodel = "fixture-model"\n'
            'api_key_env = "FIXTURE_API_KEY"\n'
            f'base_url = "http://127.0.0.1:{fixture.provider.server_port}/v1"\n'
            f'access = "unrestricted"\nmax_tokens = {64 if mode == "anthropic" else 0}\ncontext_window = 0\n'
            'provider_retry_attempts = 1\nmax_output_bytes = 524288\n'
            'command_timeout_secs = 10\n')


def suspended(fixture, session):
    return wait_for(lambda: (info if (info := fixture.request({
        "op": "inspect", "session_id": session}))["state"] == "suspended" else None),
        "automatic suspension")


def full_snapshot(fixture, session):
    snapshot = fixture.snapshot(session)
    for index, message in enumerate(snapshot["messages"]):
        if not message.get("projection_truncated"):
            continue
        chunks, offset = [], 0
        while True:
            chunk = fixture.command(session, {"op": "message_chunk", "index": message["message_index"],
                "offset": offset, "limit": 65536, "expected_revision": snapshot["revision"]})
            assert chunk["offset"] == offset
            chunks.append(chunk["data"])
            offset = chunk["next_offset"]
            if not chunk["has_more"]:
                break
        snapshot["messages"][index] = json.loads("".join(chunks))
    return snapshot


def assert_canonical(snapshot, scenario):
    tools = [message for message in snapshot["messages"] if message["role"] == "tool"]
    count = 12 if scenario.kind == "long" else 1
    assert len(tools) == count, "canonical result missing or tool repeated"
    expected = [f"loop-{i}" for i in range(count)] if scenario.kind == "long" else [CALL]
    assert [message["tool_call_id"] for message in tools] == expected
    for message in tools:
        assert message["tool_outcome"]["execution"] == "succeeded"
        assert scenario.payload in message["content"], "canonical tool text was compacted/truncated"
    assert scenario.counter.read_text() == "effect\n" * count


def run_case(binaries, mode, kind):
    fixture = ContextFixture(binaries)
    scenario = Scenario(fixture, mode, kind)
    print(f"evidence ({mode}/{kind}): {fixture.root}", flush=True)
    try:
        fixture.start()
        session = fixture.new()
        admission = fixture.command(session, fixture.mutation(session, "submit", prompt=scenario.prompt))
        assert admission["status"] == "accepted", admission
        if kind == "cancel":
            wait_for(lambda: scenario.held.is_set() or scenario.errors, "reduced retry in flight")
            assert not scenario.errors, scenario.errors
            fixture.command(session, fixture.mutation(session, "cancel", run_id=admission["run_id"]))
            snapshot = fixture.finished(session, state="cancelled")
            wait_for(scenario.disconnected.is_set, "provider disconnect after cancellation")
        else:
            expected_state = {"partial": "incomplete", "irreducible": "failed"}.get(kind, "completed")
            snapshot = fixture.finished(session, state=expected_state)
        info = suspended(fixture, session)
        snapshot = full_snapshot(fixture, session)
        assert not scenario.errors, scenario.errors
        if kind == "irreducible":
            assert len(scenario.bodies) == 1
            assert not scenario.counter.exists()
            reason = snapshot["run"].get("failure_summary") or ""
            assert "context" in reason.lower() and "narrow" in reason.lower(), reason
            assert any(m["role"] == "user" and m["content"] == scenario.prompt for m in snapshot["messages"])
        else:
            assert_canonical(snapshot, scenario)
            if kind == "manual":
                before = full_snapshot(fixture, session)
                legacy = fixture.mutation(session, "compact", retain=1)
                refused = fixture.raw_command(session, legacy)
                assert refused.get("error") and "preserve_canonical" in refused["error"], refused
                assert full_snapshot(fixture, session)["messages"] == before["messages"]
                compact = fixture.mutation(session, "compact", retain=1, preserve_canonical=True)
                receipt = fixture.command(session, compact)
                assert receipt.get("compacted_messages", 0) > 0, receipt
                assert receipt.get("canonical_preserved") is True, receipt
                assert receipt.get("removed_messages") == 0, receipt
                compacted = full_snapshot(fixture, session)
                assert compacted["messages"] == before["messages"], "Compact changed canonical history"
                recorded = fixture.command(session, {"op": "receipt", "command_id": compact["command_id"]})
                assert fixture.command(session, compact) == receipt, "duplicate Compact receipt changed"
                assert fixture.command(session, {"op": "receipt", "command_id": compact["command_id"]}) == recorded
                assert fixture.snapshot(session)["revision"] == compacted["revision"], "duplicate mutated revision"
                old = suspended(fixture, session)["incarnation"]
                followup = "Continue without repeating the completed effect."
                fixture.command(session, fixture.mutation(session, "submit", prompt=followup))
                fixture.finished(session)
                info = suspended(fixture, session)
                snapshot = full_snapshot(fixture, session)
                assert info["incarnation"] != old, "next turn did not use a fresh voyage incarnation"
                assert followup in json.dumps(scenario.bodies[-1]), "fresh turn omitted"
                assert_canonical(snapshot, scenario)
            if kind == "rejection":
                old = info["incarnation"]
                fixture.command(session, fixture.mutation(session, "submit", prompt="Use the saved reduced context; do not repeat effects."))
                fixture.finished(session)
                info = suspended(fixture, session)
                assert info["incarnation"] != old
                snapshot = full_snapshot(fixture, session)
                assert_canonical(snapshot, scenario)
            expected = {"long": 14, "rejection": 4, "manual": 3, "cancel": 3}.get(kind, 2)
            assert len(scenario.bodies) == expected, "inference replay or missing recovery"
        # Suspension is the end-of-run barrier: no sleep-only claim of non-replay.
        assert not scenario.errors, scenario.errors
        (fixture.root / "snapshot.json").write_text(json.dumps(snapshot, indent=2))
        (fixture.root / "suspended.json").write_text(json.dumps(info, indent=2))
    finally:
        scenario.stop.set()
        try:
            fixture.close()
        finally:
            (fixture.root / "context-requests.json").write_text(json.dumps(scenario.bodies, indent=2))
            (fixture.root / "context-errors.json").write_text(json.dumps(scenario.errors, indent=2))
    print(f"PASS: {mode}/{kind}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--mode", choices=["all", *MODES], default="all")
    parser.add_argument("--case", choices=["all", "rejection", "automatic", "irreducible",
                                         "manual", "cancel", "partial", "long"], default="all")
    args = parser.parse_args()
    binaries = args.bin_dir.resolve()
    for binary in ("voyage", "vessel"):
        assert (binaries / binary).is_file(), f"missing {binary} binary"
    modes = MODES if args.mode == "all" else [args.mode]
    cases = ("rejection", "automatic", "irreducible", "manual", "cancel", "partial", "long") if args.case == "all" else [args.case]
    for mode in modes:
        for kind in cases:
            run_case(binaries, mode, kind)


if __name__ == "__main__":
    main()
