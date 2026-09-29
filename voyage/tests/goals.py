"""Offline Linux Goal journeys through real Vessel-supervised Voyage processes.

Run: python3 voyage/tests/goals.py --bin-dir target/debug
Uses explicit synthetic accounts and loopback inference only. Evidence is private
under the existing fixture directory. No production runtime is contacted.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import urllib.request
import threading
import tempfile
import time
import uuid

from approval_semantics import Gateway
from delivery_recovery import wait_for
from live_events_integration import Fixture as AccountFixture, collect
from ui_journeys import launch, screen, send, paste, pty_helpers


class Fixture(AccountFixture):
    def start(self):
        self.key_root = tempfile.TemporaryDirectory(prefix="goal-connection-", dir="/dev/shm")
        key = Path(self.key_root.name) / "connection-key"
        with key.open("xb") as handle:
            handle.write(os.urandom(32))
        key.chmod(0o600)
        self.env["VOYAGE_CREDENTIAL_KEY_FILE"] = str(key)
        super().start()

    def close(self):
        try:
            super().close()
        finally:
            if hasattr(self, "key_root"):
                self.key_root.cleanup()

    def request(self, command, allow_error=False):
        if command["op"] != "start_settings":
            return super().request(command, allow_error)
        # Retain launch identity before effects, even if readiness is uncertain.
        sid = command["session_id"]
        self.binding = json.loads(Path(command["config_path"]).read_text())["config"]["account"]
        if getattr(self, "next_access", None):
            config_path = Path(command["config_path"])
            config = json.loads(config_path.read_text())
            config["config"]["access"] = self.next_access
            config["explicit"]["access"] = self.next_access
            config_path.write_text(json.dumps(config))
            self.next_access = None
        if getattr(self, "next_vessel", None):
            config_path = Path(command["config_path"])
            config = json.loads(config_path.read_text())
            config["config"]["vessel"] = self.next_vessel
            config_path.write_text(json.dumps(config))
            self.next_vessel = None
        self.sessions.append(sid)
        response = super().request(command, allow_error=True)
        if response.get("error") is None:
            return response if allow_error else response["result"]
        assert response.get("outcome_unknown") is True, response
        def observe():
            info = super(Fixture, self).request({"op": "inspect", "session_id": sid})
            if info["state"] not in ("live", "suspended"):
                return None
            # Private fixture metadata is checked without publishing its credential.
            registration = json.loads((self.directory / "sessions" / sid / "registration.json").read_text())
            assert registration["command_id"] == command["command_id"]
            return info
        info = wait_for(observe, timeout=15)
        self.record("goal-creation-observed-after-uncertain-readiness", {"session_id": sid,
            "command_id": command["command_id"], "incarnation": info["incarnation"], "state": info["state"]})
        return {"protocol": 1, "result": info, "error": None, "outcome_unknown": False} if allow_error else info

    def session(self):
        sid = super().session()
        self.sessions = list(dict.fromkeys(self.sessions))
        return sid


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            length = int(self.headers["Content-Length"])
            assert 0 < length <= 4 * 1024 * 1024
            body = json.loads(self.rfile.read(length))
            self.server.bodies.append(body)
            text = json.dumps(next(item for item in body.get("input", []) if item.get("role") == "user"))
            scenario = next(name for name in ("completion", "budget", "unknown", "paused", "restart", "false-report", "tokens", "time", "approval", "cancel", "input", "blocked", "failure", "nested", "child-one", "child-two")
                            if "goal-case-" + name in text)
            self.server.counts[scenario] = self.server.counts.get(scenario, 0) + 1
            attempt = self.server.counts[scenario]
            if scenario in ("nested", "child-one", "child-two"):
                assert (scenario == "nested") == any(tool.get("name") == "goal" for tool in body["tools"]), "child acquired parent Goal authority"
            if scenario == "failure":
                self.send_error(503, "Synthetic provider unavailable")
                return
            if scenario in self.server.gates:
                assert self.server.gates[scenario].wait(25), "fixture response gate timed out"
            output = [{"type": "message", "role": "assistant", "content": [
                {"type": "output_text", "text": "One bounded step finished."}]}]
            if (scenario == "completion" and attempt == 2) or (scenario == "blocked" and attempt == 1):
                output = [{"type": "function_call", "call_id": "proof-call", "name": "read_file",
                           "arguments": json.dumps({"path": "proof.txt"})}]
            elif (scenario == "completion" and attempt == 3) or (scenario == "false-report" and attempt == 1) or (scenario == "blocked" and attempt == 2):
                assert any(tool.get("name") == "goal" for tool in body["tools"]), "root Goal tool missing"
                evidence = "invented-call" if scenario == "false-report" else "proof-call"
                output = [{"type": "function_call", "call_id": "report-call", "name": "goal",
                           "arguments": json.dumps({"action": "report", "report": {"outcome": "blocked" if scenario == "blocked" else "complete",
                            "summary": "The expected output was observed", "evidence": [
                                {"call_id": evidence, "conclusion": "proof.txt contains verified output"}]}})}]
            elif scenario in ("nested", "child-one") and attempt == 1:
                child = "child-one" if scenario == "nested" else "child-two"
                child_id, command_id = self.server.children[child]
                action = {"action": "submit", "session_id": child_id, "command_id": command_id,
                          "expected_revision": 0, "prompt": "goal-case-" + child}
                if self.server.nested_create:
                    self.server.fixture.sessions.append(child_id)  # Record ownership before launch effects.
                    action = {"action": "create", "session_id": child_id, "command_id": command_id,
                              "workspace": str(self.server.fixture.workspace), "task": "goal-case-" + child}
                if scenario == "nested" and getattr(self.server, "remote_target", False):
                    action["target"] = "paired"
                output = [{"type": "function_call", "call_id": "delegate-" + child, "name": "vessel",
                           "arguments": json.dumps(action)}]
            elif scenario == "child-two" and attempt == 1:
                output = [{"type": "function_call", "call_id": "leaf-proof", "name": "read_file",
                           "arguments": json.dumps({"path": "proof.txt"})}]
            elif scenario == "approval" and attempt == 1:
                output = [{"type": "function_call", "call_id": "fixture-write", "name": "write_file",
                           "arguments": json.dumps({"path": "approval-output.txt", "content": "unauthorized"})}]
            response = {"id": f"fixture-{scenario}-{attempt}", "status": "completed", "output": output}
            if scenario != "unknown":
                response["usage"] = {"input_tokens": 20, "output_tokens": 4}
            stream = getattr(self.server, "web_stream", None)
            if stream is not None:
                first_text, last_text = "Shared live prefix. ", "Both clients recovered."
                response["output"][0]["content"][0]["text"] = first_text + last_text
                first = ("data: " + json.dumps({"type":"response.output_text.delta", "delta":first_text}) + "\n\n").encode()
                rest = ("data: " + json.dumps({"type":"response.output_text.delta", "delta":last_text}) + "\n\n" +
                        "data: " + json.dumps({"type":"response.completed", "response":response}) + "\n\n").encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(first) + len(rest)))
                self.end_headers()
                self.wfile.write(first)
                self.wfile.flush()
                assert stream.wait(45), "cross-client disconnect gate timed out"
                self.wfile.write(rest)
                return
            payload = ("data: " + json.dumps({"type": "response.completed", "response": response}) + "\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected after cancellation/owner death, never evidence of completion.
        except Exception as error:
            self.server.errors.append(str(error))
            self.send_error(500)


def mutation(fixture, sid, action, **fields):
    return {"op": "goal_update", "command_id": str(uuid.uuid4()),
            "expected_revision": fixture.command(sid, {"op": "snapshot"})["revision"],
            "expires_at_ms": int(time.time() * 1000) + 60000,
            "action": {"action": action, **fields}}


def set_goal(fixture, scenario, automatic=True, **limits):
    sid = fixture.session()
    command = mutation(fixture, sid, "set", objective="goal-case-" + scenario,
        limits={"runs": 4, "tokens": 10000, "elapsed_ms": 30000, "no_progress_runs": 3, **limits},
        replace_goal_id=None, continue_automatically=automatic)
    receipt = fixture.command(sid, command)
    assert receipt["status"] == "applied" and receipt["goal_id"] == command["command_id"], receipt
    return sid, command


def stopped(fixture, sid):
    def observe():
        assert not fixture.provider.errors, fixture.provider.errors
        snapshot = fixture.command(sid, {"op": "snapshot"})
        goal = snapshot["goal"]["goal"]
        return snapshot if goal["status"] != "active" and snapshot.get("pending_cleanup_run") is None and (
            not snapshot.get("run") or snapshot["run"]["state"] not in ("accepted", "running", "cancel_requested")) else None
    return wait_for(observe)


def tui_review(fixture, sid):
    """A real Helm PTY races another authenticated client, without inference."""
    pty = launch([str(fixture.binaries / "helm"), "connect", "--directory", str(fixture.directory), "--no-start"],
                 fixture.env, fixture.workspace, fixture.root / "goal-tui.pty", 120, 36)
    def expect(text):
        def observed():
            assert pty["process"].poll() is None, "Helm exited before Goal review"
            return text in screen(pty)
        try:
            wait_for(observed)
        except AssertionError:
            (fixture.root / "goal-tui-failed-screen.txt").write_text(screen(pty))
            raise
    try:
        expect("/goal")
        paste(pty, "/goal resume")
        send(pty, "\r")
        expect("AUTHORIZE automatic continuation")
        expect("Owner access observed")
        prior = fixture.command(sid, {"op": "goal_read"})["goal"]
        fixture.command(sid, mutation(fixture, sid, "edit", goal_id=prior["id"],
            objective="goal-case-budget changed by the other client", limits=prior["limits"]))
        expect("Goal changed since this review")
        send(pty, "\r")
        expect("Goal changed since review")
        assert not fixture.provider.bodies, "stale TUI review started inference"
        send(pty, "\x1b")
        wait_for(lambda: "Voyage Goal" not in screen(pty))
        send(pty, "\x7f" * len("/goal resume"))
        paste(pty, "/goal limits 2 9000 30 3")
        send(pty, "\r")
        expect("Usage is retained")
        expect("Owner access observed")
        send(pty, "\r")
        wait_for(lambda: fixture.command(sid, {"op": "goal_read"})["goal"]["limits"]["tokens"] == 9000)
        state = fixture.command(sid, {"op": "goal_read"})
        assert state["goal"]["status"] == "paused" and state["goal"]["usage"]["runs"] == 0
        fixture.record("goal-tui-and-other-client-revision-review", state)
    finally:
        pty_helpers.stop_pty(pty, wait_for)
    assert not fixture.provider.bodies, "Helm detach must not authorize continuation"


def run(fixture):
    (fixture.workspace / "proof.txt").write_text("verified output\n")
    sid, command = set_goal(fixture, "budget", automatic=False, runs=2)
    original = fixture.command(sid, {"op": "goal_read"})
    assert original["goal"]["status"] == "paused"
    assert fixture.command(sid, command)["goal_id"] == command["command_id"]
    assert not fixture.provider.bodies
    tui_review(fixture, sid)
    goal_id = command["command_id"]
    # Two clients review the same canonical revision. Exactly one edit can win.
    left = mutation(fixture, sid, "edit", goal_id=goal_id, objective="goal-case-budget left", limits=original["goal"]["limits"])
    right = {**left, "command_id": str(uuid.uuid4()), "action": {**left["action"], "objective": "goal-case-budget right"}}
    with ThreadPoolExecutor(max_workers=2) as pool:
        replies = list(pool.map(lambda c: fixture.command(sid, c, allow_error=True), (left, right)))
    assert sum(reply.get("error") is None for reply in replies) == 1, replies
    winner = left if replies[0].get("error") is None else right
    assert fixture.command(sid, winner)["goal_id"] == goal_id
    fixture.command(sid, mutation(fixture, sid, "resume", goal_id=goal_id))
    snapshot = stopped(fixture, sid)
    goal = snapshot["goal"]["goal"]
    assert goal["status"] == "limited" and goal["stop_reason"] == "run_limit", goal
    assert goal["usage"]["runs"] == 2 and goal["usage"]["input_tokens"] == 40 and goal["usage"]["output_tokens"] == 8, goal
    assert fixture.provider.counts["budget"] == 2
    fixture.record("goal-concurrent-controls-and-run-budget", snapshot)

    sid, command = set_goal(fixture, "completion")
    snapshot = stopped(fixture, sid)
    goal = snapshot["goal"]["goal"]
    assert goal["status"] == "complete" and goal["usage"]["runs"] == 2, goal
    assert (goal["usage"]["input_tokens"], goal["usage"]["output_tokens"]) == (80, 16), goal
    assert goal["assessment"]["report"]["evidence"][0]["call_id"] == "proof-call", goal
    assert fixture.provider.counts["completion"] == 4
    events, cursor = collect(fixture, sid)
    goal_events = [event for event in events if event["kind"] == "goal"]
    assert goal_events and "goal-case-completion" not in json.dumps(goal_events), goal_events
    assert all("objective" not in event["payload"] and "assessment" not in event["payload"] for event in goal_events)
    time.sleep(.2)
    assert fixture.provider.counts["completion"] == 4
    fixture.record("goal-continuation-evidence-and-public-events", {"snapshot": snapshot, "goal_events": goal_events, "cursor": cursor})

    for scenario, reason in (("unknown", "usage_unknown"), ("false-report", "no_progress")):
        sid, _ = set_goal(fixture, scenario, no_progress_runs=1)
        snapshot = stopped(fixture, sid)
        goal = snapshot["goal"]["goal"]
        assert goal["status"] != "complete" and goal["stop_reason"] == reason, goal
        assert not goal.get("assessment"), goal
        if scenario == "unknown":
            assert goal["usage"]["unmeasured_runs"] == 1, goal
            reply = fixture.command(sid, mutation(fixture, sid, "resume", goal_id=goal["id"]), allow_error=True)
            assert reply.get("error"), reply
        fixture.record("goal-" + scenario + "-stops-truthfully", snapshot)

    safety(fixture)
    lifecycle(fixture)
    nested(fixture)
    nested(fixture, create=True)
    nested(fixture, interrupt=True)
    nested_remote(fixture)
    nested_remote(fixture, create=True)
    authority(fixture)


def safety(fixture):
    for scenario, limits, reason in (("tokens", {"tokens": 24}, "token_limit"),
                                      ("failure", {}, ("usage_unknown", "provider_failure")),
                                      ("approval", {}, "approval_required")):
        if scenario == "approval":
            fixture.next_access = "approval"
        sid, _ = set_goal(fixture, scenario, **limits)
        snapshot = stopped(fixture, sid)
        goal = snapshot["goal"]["goal"]
        assert goal["stop_reason"] in ((reason,) if isinstance(reason, str) else reason) and goal["status"] != "complete", goal
        assert goal["usage"]["runs"] == 1, goal
        assert not (fixture.workspace / "approval-output.txt").exists()
        fixture.record("goal-" + scenario + "-bounded-stop", snapshot)

    sid, _ = set_goal(fixture, "blocked")
    snapshot = stopped(fixture, sid)
    assert snapshot["goal"]["goal"]["status"] == "blocked", snapshot["goal"]
    assert snapshot["goal"]["goal"]["assessment"]["report"]["outcome"] == "blocked"
    assert fixture.provider.counts["blocked"] == 3
    fixture.record("goal-evidence-backed-impasse", snapshot)

    for scenario in ("time", "cancel", "input"):
        gate = fixture.provider.gates[scenario] = threading.Event()
        sid, _ = set_goal(fixture, scenario, **({"elapsed_ms": 1000} if scenario == "time" else {}))
        wait_for(lambda: fixture.provider.counts.get(scenario) == 1)
        before = fixture.command(sid, {"op": "snapshot"})
        if scenario != "time":
            action = {"op": "cancel" if scenario == "cancel" else "steer",
                      "command_id": str(uuid.uuid4()), "run_id": before["run"]["run_id"],
                      "expected_revision": before["revision"], "expires_at_ms": int(time.time()*1000)+60000}
            if scenario == "input":
                action["prompt"] = "Human input takes precedence; finish this one step."
            fixture.command(sid, action)
            if scenario == "input":
                gate.set()
        snapshot = stopped(fixture, sid)
        gate.set()
        goal = snapshot["goal"]["goal"]
        reason = {"time": "time_limit", "cancel": "usage_unknown", "input": "user_input"}[scenario]
        assert goal["stop_reason"] == reason and not goal["continuation_authorized"], goal
        assert goal["usage"]["runs"] == 1 and goal["status"] != "complete", goal
        fixture.record("goal-" + scenario + "-stops-continuation", snapshot)

    # Reading a suspended owner must preserve the Goal and not restore authority.
    sid, creation = set_goal(fixture, "budget", automatic=False)
    before = fixture.command(sid, {"op": "goal_read"})
    incarnation = fixture.request({"op": "inspect", "session_id": sid})["incarnation"]
    fixture.suspended(sid)
    assert fixture.command(sid, {"op": "goal_read"}) == before
    assert fixture.request({"op": "inspect", "session_id": sid})["state"] == "suspended"
    def saved_snapshot():
        # Read-only observer admission can briefly be unavailable during owner
        # guard retirement. Poll the observation; never repeat the mutation.
        response = fixture.command(sid, {"op": "snapshot"}, allow_error=True)
        if response.get("error") == "suspended observation unavailable":
            return None
        assert response.get("error") is None, response
        return response["result"]
    saved = wait_for(saved_snapshot, timeout=5)
    change = {"op": "goal_update", "command_id": str(uuid.uuid4()),
              "expected_revision": saved["revision"], "expires_at_ms": int(time.time()*1000)+60000,
              "action": {"action": "edit", "goal_id": creation["command_id"],
                         "objective": "goal-case-budget revised after suspension", "limits": before["goal"]["limits"]}}
    receipt = fixture.command(sid, change)
    assert receipt["status"] == "applied"
    assert fixture.request({"op": "inspect", "session_id": sid})["incarnation"] != incarnation
    assert fixture.command(sid, change) == receipt
    snapshot = fixture.command(sid, {"op": "snapshot"})
    assert snapshot["goal"]["goal"]["status"] == "paused" and snapshot["goal"]["goal"]["usage"]["runs"] == 0
    fixture.record("goal-suspended-read-and-fresh-owner-edit", snapshot)



def lifecycle(fixture):
    for scenario in ("paused", "restart"):
        gate = fixture.provider.gates[scenario] = threading.Event()
        sid, creation = set_goal(fixture, scenario)
        wait_for(lambda: fixture.provider.counts.get(scenario) == 1)
        goal = fixture.command(sid, {"op": "goal_read"})["goal"]
        if scenario == "paused":
            fixture.command(sid, mutation(fixture, sid, "pause", goal_id=goal["id"]))
            gate.set()
            snapshot = stopped(fixture, sid)
            assert snapshot["goal"]["goal"]["status"] == "paused", snapshot["goal"]
            assert snapshot["goal"]["goal"]["usage"]["input_tokens"] == 20
        else:
            incarnation = fixture.request({"op": "inspect", "session_id": sid})["incarnation"]
            owned = [(pid, argv) for pid, argv in fixture.owned_processes().items()
                     if Path(os.fsdecode(argv[3])) == fixture.directory / "sessions" / sid]
            assert len(owned) == 1, owned
            pid, argv = owned[0]
            descriptor = os.pidfd_open(pid)
            try:
                assert fixture.owned_processes().get(pid) == argv
                signal.pidfd_send_signal(descriptor, signal.SIGKILL)
            finally:
                os.close(descriptor)
            wait_for(lambda: pid not in fixture.owned_processes())
            gate.set()
            recovery = fixture.request({"op": "recover", "command_id": str(uuid.uuid4()), "session_id": sid,
                "incarnation": incarnation, "acknowledge_cleanup": None, "reconcile_tools": None,
                "expected_revision": None, "acknowledge_resources": []})
            assert recovery["restart_permitted"] is True, recovery
            restarted = fixture.request({"op": "restart", "command_id": str(uuid.uuid4()), "session_id": sid, "incarnation": incarnation})
            assert restarted["incarnation"] != incarnation
            snapshot = stopped(fixture, sid)
            goal = snapshot["goal"]["goal"]
            assert goal["id"] == creation["command_id"] and goal["status"] != "active" and goal["usage"]["unmeasured_runs"] == 1, goal
            replay = fixture.command(sid, creation)
            assert replay["goal_id"] == creation["command_id"] and replay["status"] == "applied", replay
            assert fixture.command(sid, {"op": "goal_read"})["goal"] == goal
        time.sleep(.2)
        assert fixture.provider.counts[scenario] == 1, "unexpected automatic replay after " + scenario
        fixture.record("goal-" + scenario + "-no-replay", snapshot)


def nested(fixture, interrupt=False, create=False, remote_credential=None):
    fixture.provider.remote_target = remote_credential is not None
    fixture.provider.nested_create = create
    for name in ("nested", "child-one", "child-two"):
        fixture.provider.counts[name] = 0
    gate = threading.Event()
    if interrupt:
        fixture.provider.gates["child-two"] = gate
    # Existing idle destinations isolate nested accounting from process creation.
    # Every child is a real independent owner, explicitly provisioned by this fixture.
    fixture.provider.children = {}
    for name in ("child-one", "child-two"):
        fixture.next_access = "unrestricted"
        fixture.provider.children[name] = (str(uuid.uuid4()) if create else fixture.session(), str(uuid.uuid4()))
    fixture.next_access = "unrestricted"
    if remote_credential is not None:
        # Explicit host configuration retains the paired identity pin. Neither
        # the credential nor its path is supplied by the model.
        credential_path = fixture.root / "nested-remote.json"
        credential_path.write_text(json.dumps(remote_credential))
        credential_path.chmod(0o600)
        fixture.next_vessel = {"enabled": True, "remotes": {"paired": str(credential_path)}}
    sid, _ = set_goal(fixture, "nested", runs=1, elapsed_ms=60000)
    if interrupt:
        wait_for(lambda: fixture.provider.counts.get("child-two") == 1)
        def terminal_parent():
            value = fixture.command(sid, {"op": "snapshot"})
            return value if value.get("run", {}).get("state") == "completed" else None
        before = wait_for(terminal_parent)
        fixture.command(sid, {"op": "cancel", "command_id": str(uuid.uuid4()),
            "run_id": before["run"]["run_id"], "expected_revision": before["revision"],
            "expires_at_ms": int(time.time()*1000)+60000})
    snapshot = stopped(fixture, sid)
    if interrupt:
        initial_goal = snapshot["goal"]["goal"]
        assert initial_goal["usage"]["unmeasured_runs"] == 1 and not initial_goal["continuation_authorized"], initial_goal
        gate.set()
    goal = snapshot["goal"]["goal"]
    children = {}
    for name, (child, command_id) in fixture.provider.children.items():
        value = fixture.command(child, {"op": "snapshot"})
        assert value["goal"]["goal"] is None, value["goal"]
        def settled_receipt():
            result = fixture.command(child, {"op": "receipt", "command_id": command_id})
            return result if result.get("execution_usage") else None
        receipt = wait_for(settled_receipt)
        (fixture.root / ("nested-" + name + ".json")).write_text(json.dumps({"root": snapshot, "child": value, "receipt": receipt}, indent=2))
        usage = receipt["execution_usage"]
        assert usage["complete"] and usage["cleanup_observed"], usage
        assert usage["budget"]["session_id"] == child and usage["budget"]["command_id"] == command_id
        children[name] = usage
    assert children["child-one"]["budget"]["parent_session_id"] == sid
    assert children["child-two"]["budget"]["parent_session_id"] == fixture.provider.children["child-one"][0]
    assert children["child-two"]["budget"]["tokens"] < children["child-one"]["budget"]["tokens"] < goal["limits"]["tokens"]
    assert (children["child-two"]["input_tokens"], children["child-two"]["output_tokens"]) == (40, 8)
    assert (children["child-one"]["input_tokens"], children["child-one"]["output_tokens"]) == (80, 16)
    if interrupt:
        def reconciled():
            # Child owner retirement can temporarily delay its receipt read.
            # Reconciliation with fencing disabled observes/account-checkpoints
            # only; it never resubmits the original child operation.
            receipt = fixture.command(sid, {"op": "goal_reconcile", "offset": 0, "limit": 128, "fence_children": False})
            value = fixture.command(sid, {"op": "snapshot"})
            current = value["goal"]["goal"]
            assert current["status"] == initial_goal["status"] and not current["continuation_authorized"], current
            assert current["usage"]["unmeasured_runs"] == 1, current
            (fixture.root / "late-reconciliation.json").write_text(json.dumps({"receipt": receipt, "goal": current}))
            return value if (current["usage"]["input_tokens"], current["usage"]["output_tokens"]) == (120, 24) else None
        snapshot = wait_for(reconciled)
        goal = snapshot["goal"]["goal"]
        fixture.command(sid, {"op": "goal_reconcile", "offset": 0, "limit": 128, "fence_children": False})
        assert fixture.command(sid, {"op": "goal_read"})["goal"] == goal, "late usage changed after complete accounting"
    else:
        assert goal["usage"]["unmeasured_runs"] == 0, goal
        assert goal["stop_reason"] == "run_limit" and goal["status"] == "limited", goal
    assert (goal["usage"]["input_tokens"], goal["usage"]["output_tokens"]) == (120, 24), goal
    assert all(fixture.provider.counts[name] == 2 for name in ("nested", "child-one", "child-two")), fixture.provider.counts
    fixture.record("goal-three-process-nested-" + ("paired-remote-" if remote_credential else "") + ("cancel-and-late-accounting" if interrupt else "creation-and-exact-accounting" if create else "exact-accounting"), {"snapshot": snapshot, "children": children})


def nested_remote(fixture, create=False):
    gateway = Gateway(fixture)
    try:
        credential = owner_credential(fixture, gateway)
        def capabilities(identity):
            request = urllib.request.Request(gateway.origin + "/v1/vessel/command",
                data=json.dumps({"protocol": 1, "command": {"op": "capabilities"}}).encode(),
                headers={"Content-Type": "application/json", "Authorization": "Bearer " + credential["token"],
                    "X-Voyage-Grant": credential["grant_id"], "X-Voyage-Vessel": identity})
            with urllib.request.urlopen(request, timeout=10) as response:
                return json.load(response)
        caps = capabilities(credential["vessel_id"])
        assert caps.get("error") is None and caps["result"]["scope"] == "owner"
        assert {"goals", "execution_budget", "start_settings"} <= set(caps["result"]["features"])
        assert capabilities(str(uuid.uuid4())).get("error"), "wrong Vessel identity accepted"
        nested(fixture, create=create, remote_credential=credential)
    finally:
        gateway.close()


def authority(fixture):
    fixture.provider.counts["paused"] = 0
    sid = fixture.session()
    gateway = Gateway(fixture)
    try:
        credential = owner_credential(fixture, gateway)
        scoped = gateway.grant(sid, ["history", "observe", "execute"])
        assert not gateway.call(scoped, sid, {"op": "goal_read"}).get("error")
        forbidden = mutation(fixture, sid, "set", objective="goal-case-budget scoped denial",
            limits={"runs": 2, "tokens": 1000, "elapsed_ms": 10000, "no_progress_runs": 2},
            replace_goal_id=None, continue_automatically=True)
        assert gateway.call(scoped, sid, forbidden).get("error"), "ordinary Execute acquired Goal control"
        assert fixture.command(sid, {"op": "goal_read"})["goal"] is None
        secret = {**forbidden, "command_id": str(uuid.uuid4()), "action": {**forbidden["action"],
                  "objective": "Do not disclose " + fixture.env["LIVE_EVENTS_FIXTURE_KEY"]}}
        assert gateway.call(credential, sid, secret).get("error"), "configured secret became public Goal data"
        assert fixture.command(sid, {"op": "goal_read"})["goal"] is None
        gate = fixture.provider.gates["paused"] = threading.Event()
        allowed = {**forbidden, "command_id": str(uuid.uuid4()), "action": {**forbidden["action"], "objective": "goal-case-paused"}}
        reply = gateway.call(credential, sid, allowed)
        assert reply.get("error") is None and reply["result"]["status"] == "applied", reply
        wait_for(lambda: fixture.provider.counts.get("paused") == 1)
        result = subprocess.run([str(fixture.binaries / "vessel"), "revoke-connection", "--directory", str(fixture.directory),
            "--grant", credential["grant_id"], "--expected-revision", "1", "--command-id", str(uuid.uuid4())],
            env=fixture.env, cwd=fixture.workspace, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stderr
        gate.set()
        snapshot = stopped(fixture, sid)
        goal = snapshot["goal"]["goal"]
        assert goal["stop_reason"] == "authority_revoked" and not goal["continuation_authorized"], goal
        assert goal["usage"]["runs"] == 1 and fixture.provider.counts["paused"] == 1
        assert gateway.call(credential, sid, mutation(fixture, sid, "resume", goal_id=goal["id"])).get("error")
        fixture.record("goal-authenticated-scope-secret-and-revocation", snapshot)
    finally:
        gateway.close()


def owner_credential(fixture, gateway):
    principal = str(uuid.uuid4())
    invitation_path = fixture.root / ("invitation-" + str(uuid.uuid4()) + ".json")
    result = subprocess.run([str(fixture.binaries / "vessel"), "pair-invite",
        "--directory", str(fixture.directory), "--endpoint", gateway.origin,
        "--principal", principal, "--full-access", "--output", str(invitation_path)],
        env=fixture.env, cwd=fixture.workspace, capture_output=True, text=True, timeout=15)
    assert result.returncode == 0, result.stderr
    invitation = json.loads(invitation_path.read_text())
    request = urllib.request.Request(gateway.origin + "/v1/vessel/pair",
        data=json.dumps({"protocol": 1, "command_id": str(uuid.uuid4()), "principal_id": principal,
            "invitation_id": invitation["invitation_id"], "code": invitation["code"]}).encode(),
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=10) as response:
        paired = json.load(response)
    assert not paired.get("error"), paired.get("error")
    return paired["result"]


def web(fixture):
    sid = fixture.session()
    gateway = Gateway(fixture)
    client = None
    pty = None
    try:
        credential = owner_credential(fixture, gateway)
        key, cert = fixture.root / "tls.key", fixture.root / "tls.pem"
        result = subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
            "-subj", "/CN=127.0.0.1", "-keyout", str(key), "-out", str(cert)], capture_output=True, timeout=15)
        assert result.returncode == 0, "fixture TLS generation failed"
        config = fixture.root / "web-config.json"
        config.write_text(json.dumps({"webRoot": str(fixture.web_root), "gateway": gateway.origin,
            "credential": credential, "key": str(key), "cert": str(cert), "evidence": str(fixture.root)}))
        with (fixture.root / "web.log").open("w") as log:
            client = subprocess.Popen(["node", str(Path(__file__).with_name("goals_web.mjs")), str(config)],
                                      stdout=log, stderr=log)
            def ready():
                assert client.poll() is None, "Web journey exited; inspect private fixture web.log"
                return (fixture.root / "goal-web-review-ready").exists()
            wait_for(ready, timeout=40)
            prior = fixture.command(sid, {"op": "goal_read"})["goal"]
            assert prior["status"] == "paused" and prior["usage"]["runs"] == 0
            pty = launch([str(fixture.binaries / "helm"), "connect", "--directory", str(fixture.directory), "--no-start"],
                         fixture.env, fixture.workspace, fixture.root / "goal-web-tui.pty", 120, 36)
            wait_for(lambda: "/goal" in screen(pty))
            paste(pty, "/goal")
            send(pty, "\r")
            wait_for(lambda: "Web and TUI shared objective" in screen(pty))
            send(pty, "\x1b")
            wait_for(lambda: "Voyage Goal" not in screen(pty))
            send(pty, "\x7f" * len("/goal"))
            paste(pty, "/goal limits 2 9000 30 3")
            send(pty, "\r")
            wait_for(lambda: "Owner access observed" in screen(pty))
            send(pty, "\r")
            wait_for(lambda: fixture.command(sid, {"op": "goal_read"})["goal"]["limits"]["tokens"] == 9000)
            pty_helpers.stop_pty(pty, wait_for)
            pty = None
            (fixture.root / "goal-web-other-client-done").write_text("done")
            wait_for(lambda: (fixture.root / "goal-web-stream-ready").exists(), timeout=40)
            assert fixture.command(sid, {"op": "goal_read"})["goal"] is None
            assert not fixture.provider.bodies, "paused client controls dispatched inference"
            pty = launch([str(fixture.binaries / "helm"), "connect", "--directory", str(fixture.directory), "--no-start"],
                         fixture.env, fixture.workspace, fixture.root / "stream-web-tui.pty", 120, 36)
            wait_for(lambda: "fixture-model" in screen(pty))
            fixture.provider.web_stream = threading.Event()
            command = fixture.submit(sid, "goal-case-budget shared live event qualification")
            fixture.command(sid, command)
            wait_for(lambda: "Shared live prefix." in screen(pty))
            wait_for(lambda: (fixture.root / "goal-web-stream-disconnected").exists(), timeout=40)
            running = fixture.command(sid, {"op":"snapshot"})
            assert running["run"]["state"] == "running", running["run"]
            fixture.provider.web_stream.set()
            finished = fixture.finished(sid)
            assert finished["run"]["state"] == "completed", finished["run"]
            wait_for(lambda: "Both clients recovered." in screen(pty))
            (fixture.root / "goal-web-stream-done").write_text(json.dumps({"revision":finished["revision"],
                "run_id":finished["run"]["run_id"], "observation_cursor":finished["observation_cursor"]}))
            assert client.wait(timeout=45) == 0, "Web journey failed; inspect private fixture web.log"
            assert len(fixture.provider.bodies) == 1, "observation replayed inference"
            pty_helpers.stop_pty(pty, wait_for)
            pty = None
        assert fixture.command(sid, {"op": "goal_read"})["goal"] is None
        fixture.record("goal-real-web-tui-canonical-controls", json.loads((fixture.root / "goal-web-result.json").read_text()))
    finally:
        if getattr(fixture.provider, "web_stream", None):
            fixture.provider.web_stream.set()
        if pty:
            pty_helpers.stop_pty(pty, wait_for)
        try:
            if client and client.poll() is None:
                client.terminate()
                try:
                    client.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    client.kill()
                    client.wait(timeout=5)
        finally:
            gateway.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--only", choices=["safety", "lifecycle", "web", "nested", "authority", "nested-interrupt", "nested-create", "nested-remote", "nested-remote-create"])
    parser.add_argument("--web-root", type=Path)
    args = parser.parse_args()
    assert args.only != "web" or args.web_root, "--web-root required for Web journey"
    assert __debug__, "Assertions must be enabled"
    os.umask(0o077)
    fixture = Fixture(args.bin_dir.resolve())
    fixture.web_root = args.web_root.resolve() if args.web_root else None
    fixture.provider.RequestHandlerClass = Provider
    fixture.provider.fixture = fixture
    fixture.provider.errors = []
    fixture.provider.counts = {}
    fixture.provider.gates = {}
    try:
        fixture.start()
        (fixture.workspace / "proof.txt").write_text("verified output\n")
        {"safety": safety, "lifecycle": lifecycle, "web": web, "nested": nested, "authority": authority, "nested-interrupt": lambda f: nested(f, interrupt=True), "nested-create": lambda f: nested(f, create=True), "nested-remote": nested_remote, "nested-remote-create": lambda f: nested_remote(f, create=True)}.get(args.only, run)(fixture)
    finally:
        for gate in fixture.provider.gates.values():
            gate.set()
        fixture.close()


if __name__ == "__main__":
    main()
