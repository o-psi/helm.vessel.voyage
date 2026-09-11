"""Offline #21 decision authority checks through the public scoped HTTP gateway.

Synthetic principals/provider only. No live account, external TLS, or UI claim.
Run without -O after building vessel and voyage. Evidence and cleanup use the
existing delivery-recovery fixture rather than a replacement test framework.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request
import uuid

from delivery_recovery import Fixture, wait_for


def uid():
    return str(uuid.uuid4())


class Gateway:
    def __init__(self, fixture):
        self.fixture = fixture
        with socket.socket() as reserved:
            reserved.bind(("127.0.0.1", 0))
            port = reserved.getsockname()[1]
        self.origin = f"http://127.0.0.1:{port}"
        self.process = subprocess.Popen([
            str(fixture.binaries / "vessel"), "--bind", f"127.0.0.1:{port}",
            "--database", str(fixture.root / "gateway.sqlite3"),
            "--process-directory", str(fixture.directory),
            "--public-origin", self.origin, "--allow-insecure-loopback"],
            env=fixture.env | {"VESSEL_OPERATOR_TOKEN": "x" * 32},
            cwd=fixture.workspace, stdin=subprocess.DEVNULL,
            stdout=fixture.log, stderr=fixture.log)
        try:
            wait_for(self.ready)
        except BaseException:
            self.close()
            raise

    def ready(self):
        assert self.process.poll() is None, "gateway exited"
        try:
            with urllib.request.urlopen(self.origin + "/health", timeout=1):
                return True
        except urllib.error.HTTPError:
            return True
        except OSError:
            return False

    def grant(self, session, rights, lifetime=60000):
        return self.fixture.request({"op": "grant", "command_id": uid(),
            "grant_id": uid(), "principal_id": uid(), "session_id": session,
            "workspace": str(self.fixture.workspace), "rights": rights,
            "expires_at_ms": int(time.time() * 1000) + lifetime,
            "endpoint": self.origin})

    def workspace_grant(self):
        principal = uid()
        invitation_path = self.fixture.root / (uid() + ".json")
        issued = subprocess.run([str(self.fixture.binaries / "vessel"), "pair-invite",
            "--directory", str(self.fixture.directory), "--endpoint", self.origin,
            "--principal", principal, "--workspace", str(self.fixture.workspace),
            "--output", str(invitation_path), "--rights", "decide"],
            env=self.fixture.env, cwd=self.fixture.workspace, capture_output=True,
            text=True, timeout=10)
        assert issued.returncode == 0, issued.stderr
        invitation = json.loads(invitation_path.read_text())
        request = urllib.request.Request(self.origin + "/v1/vessel/pair",
            data=json.dumps({"protocol": 1, "command_id": uid(), "principal_id": principal,
                "invitation_id": invitation["invitation_id"], "code": invitation["code"]}).encode(),
            headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=10) as incoming:
            result = json.load(incoming)
        assert not result.get("error"), result.get("error")
        return result["result"]

    def call(self, credential, session, command):
        request = urllib.request.Request(self.origin + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": {**command,
                "session_id": session}}).encode(),
            headers={"Content-Type": "application/json",
                "Authorization": "Bearer " + credential["token"],
                "X-Voyage-Grant": credential["grant_id"],
                **({"X-Voyage-Vessel": credential["vessel_id"]} if "vessel_id" in credential else {})})
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                value = json.load(response)
        except urllib.error.HTTPError as error:
            return {"error": f"HTTP {error.code}"}
        if value.get("error"):
            return value
        return {"result": value["result"]["result"], "error": None}

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
            raise AssertionError("gateway did not stop gracefully")


def matrix(fixture, gateway, mode, workspace=False):
    session = fixture.session(approval=True)
    requester = gateway.grant(session, ["execute"])
    approver = gateway.workspace_grant() if workspace else gateway.grant(session, ["decide"])
    observer = gateway.grant(session, ["observe", "history", "execute", "steer", "cancel"])
    started = gateway.call(requester, session, fixture.submit(session, "approval-fixture " + mode))
    assert not started.get("error"), started
    def observe_decisions():
        value = gateway.call(approver, session, {"op": "decisions"})
        assert not value.get("error"), value
        return value["result"]
    pending = wait_for(observe_decisions)
    decision = pending[0]
    snapshot = fixture.command(session, {"op": "snapshot"})
    response = {"op": "respond", "command_id": uid(),
        "incarnation": decision["incarnation"], "run_id": decision["run_id"],
        "decision_id": decision["decision_id"], "response": "approved",
        "expected_revision": snapshot["revision"], "expires_at_ms": int(time.time()*1000)+60000}
    assert gateway.call(observer, session, response).get("error"), "non-decider approved"
    assert gateway.call(requester, session, {"op": "decisions"}).get("error"), "execute disclosed decisions"
    assert gateway.call(approver, session, {"op": "snapshot"}).get("error"), "decide disclosed history"
    assert gateway.call(approver, session, {**response, "command_id": uid(), "incarnation": uid()}).get("error")
    assert gateway.call(approver, session, {**response, "command_id": uid(), "run_id": uid()}).get("error")
    assert gateway.call(approver, uid(), response).get("error"), "wrong session accepted"
    results = []
    if mode == "grant-expired":
        approver = gateway.grant(session, ["decide"], lifetime=150)
        time.sleep(0.2)
        assert gateway.call(approver, session, response).get("error")
    elif mode == "cancelled":
        fixture.command(session, {"op": "cancel", "command_id": uid(),
            "run_id": decision["run_id"], "expected_revision": snapshot["revision"],
            "expires_at_ms": int(time.time()*1000)+60000})
        assert gateway.call(approver, session, response).get("error")
    elif mode == "revoked":
        fixture.request({"op": "revoke_grant", "command_id": uid(),
            "grant_id": approver["grant_id"], "expected_revision": 1})
        assert gateway.call(approver, session, response).get("error")
    elif mode == "expired":
        time.sleep(2.1)
        assert gateway.call(approver, session, response).get("error")
    elif mode == "denied":
        response["response"] = "denied"
        results.append(gateway.call(approver, session, response))
        assert not results[0].get("error"), results
    elif mode == "race":
        competitor = gateway.grant(session, ["decide"])
        other = {**response, "command_id": uid(), "response": "denied"}
        with ThreadPoolExecutor(max_workers=2) as pool:
            a = pool.submit(gateway.call, approver, session, response)
            b = pool.submit(gateway.call, competitor, session, other)
            results = [a.result(), b.result()]
        assert sum(not r.get("error") for r in results) == 1, results
    else:
        results.append(gateway.call(approver, session, response))
        assert not results[0].get("error"), results
        # Same principal + exact payload recovers; changed identity/payload cannot.
        assert gateway.call(approver, session, response) == results[0]
        assert gateway.call(approver, session, {"op": "receipt",
            "command_id": response["command_id"]}).get("error")
        original = {k: v for k, v in response.items() if k != "incarnation"}
        recovered = gateway.call(approver, session, {"op": "resolve",
            "command_id": response["command_id"], "original": original})
        assert recovered == results[0], recovered
        assert gateway.call(approver, session, {**response, "response": "denied"}).get("error")
        competitor = gateway.grant(session, ["decide"])
        assert gateway.call(competitor, session, response).get("error")
    snapshot = fixture.finished(session)
    output = fixture.workspace / "approval-output.txt"
    approved = mode == "approved" or (mode == "race" and not results[0].get("error"))
    assert output.exists() == approved, (mode, results, snapshot)
    if approved:
        assert output.read_text() == "unauthorized"
        output.unlink()
    history = fixture.command(session, {"op": "history", "offset": 0, "limit": 128})
    assert history["has_more"] is False
    effects = [message for message in history["messages"]
               if message["role"] == "tool" and message["tool_call_id"] == "fixture-write"
               and message["tool_success"] is True]
    assert len(effects) == int(approved), (mode, effects)
    with sqlite3.connect(fixture.directory / "sessions" / session / "journal/journal.sqlite3") as db:
        requester_id = json.loads(db.execute("SELECT record FROM runs WHERE id=?",
            (decision["run_id"],)).fetchone()[0])["principal_id"]
        responder_ids = [db.execute("SELECT principal FROM process_command_bindings WHERE id=?",
            (r["result"]["command_id"],)).fetchone()[0] for r in results if not r.get("error")]
    assert all(responder != requester_id for responder in responder_ids)
    assert len(responder_ids) <= 1
    fixture.record("scoped-approval-" + ("workspace-" if workspace else "session-") + mode,
        {"decision": decision, "responses": results, "requester_principal": requester_id,
         "responder_principals": responder_ids, "file_effect": approved,
         "successful_tool_occurrences": len(effects),
         "snapshot": snapshot, "history": history})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    gateway = None
    try:
        fixture.start()
        gateway = Gateway(fixture)
        for mode in ("approved", "denied", "race", "revoked", "expired", "grant-expired", "cancelled"):
            matrix(fixture, gateway, mode)
        for mode in ("approved", "denied", "race"):
            matrix(fixture, gateway, mode, workspace=True)
    finally:
        if gateway is not None:
            gateway.close()
        fixture.close()


if __name__ == "__main__":
    main()
