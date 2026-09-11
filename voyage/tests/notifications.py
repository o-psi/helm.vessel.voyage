"""Focused #72 synthetic public-gateway notification/decision separation journey.

Uses actual independently supervised Voyage and two independent scoped principals.
No live providers, external messages or human acceptance gate. Run only against the
coordinator-admitted exact binary set; source drafting does not establish a pass.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import fcntl
import os
import signal
import struct
import termios
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request

from approval_semantics import ApprovalFixture, Gateway, uid
from delivery_recovery import wait_for
import ui_journeys as ui
import images_composer as pty_helpers


class NotificationFixture(ApprovalFixture):
    def session(self, approval=False):
        session = uid()
        self.sessions.append(session)
        config = self.root / (session + ".toml")
        config.write_text('provider = "openai-responses"\nmodel = "fixture-model"\n'
            f'base_url = "http://127.0.0.1:{self.provider.server_port}/v1"\n'
            'api_key_required = false\nprovider_retry_attempts = 1\n'
            'context_window = 0\ncommand_timeout_secs = 30\n'
            f'access = "{"approval" if approval else "read-only"}"\n')
        config.chmod(0o600)
        self.request({"op": "start_configured", "session_id": session, "command_id": uid(),
            "workspace": str(self.workspace), "config_path": str(config)})
        return session


def call(fixture, gateway, credential, operation):
    command = {"op": "notifications", "operation": operation}
    if credential is None:
        return fixture.request(command, allow_error=True)
    request = urllib.request.Request(gateway.origin + "/v1/vessel/command",
        data=json.dumps({"protocol": 1, "command": command}).encode(),
        headers={"Content-Type": "application/json",
            "Authorization": "Bearer " + credential["token"],
            "X-Voyage-Grant": credential["grant_id"]})
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        return {"error": f"HTTP {error.code}"}


def success(value):
    assert not value.get("error"), value
    return value["result"]


def destination(fixture, session, credential, vessel):
    grant = json.loads((fixture.directory / "access/grants" /
        (credential["grant_id"] + ".json")).read_text())
    return {"id": uid(), "recipient_grant_id": credential["grant_id"],
        "recipient_principal_id": grant["principal_id"], "recipient_grant_revision": grant["revision"],
        "source_vessel_id": vessel, "source_session_id": session,
        "event_kinds": ["test", "completed", "failed", "cancelled", "interrupted", "incomplete", "attention", "budget"],
        "expires_at_ms": int(time.time() * 1000) + 120000,
        "notification_ttl_ms": 60000, "quiet_hours_utc": None}


def client_checks(fixture, credential_file, destination_id, event_id):
    argv = [str(fixture.binaries / "helm"), "connect", "--no-start", "--access-file", str(credential_file)]
    cli = subprocess.run([*argv, "inbox", "list", destination_id, "--limit", "100"],
        cwd=fixture.workspace, env=fixture.env, capture_output=True, text=True, timeout=15)
    assert cli.returncode == 0, cli.stderr
    page = json.loads(cli.stdout)["page"]
    assert any(e["notification"]["event_id"] == event_id for e in page["entries"])
    pty = ui.launch(argv, fixture.env | {"TERM": "xterm-256color"}, fixture.workspace,
        fixture.root / "notification-inbox.pty", 120, 32)
    before = len(fixture.provider.bodies)
    def expect(text):
        def observed():
            assert pty["process"].poll() is None, "Helm exited"
            return text in ui.screen(pty)
        return wait_for(observed, timeout=15)
    try:
        expect("F2")
        ui.send(pty, ui.F2)
        time.sleep(.2)
        ui.send(pty, ui.ENTER)
        # The only scoped source is selected explicitly, never by a notification.
        command = f"/inbox list {destination_id}"
        ui.paste(pty, command)
        ui.send(pty, ui.ENTER)
        expect("Notification inbox")
        expect("Execution cleanup")
        fcntl.ioctl(pty["fd"], termios.TIOCSWINSZ, struct.pack("HHHH", 18, 40, 0, 0))
        pty["rows"], pty["columns"] = 18, 40
        os.kill(pty["pid"], signal.SIGWINCH)
        expect("Notification inbox")
        ui.send(pty, ui.ESC)
        fcntl.ioctl(pty["fd"], termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        pty["rows"], pty["columns"] = 32, 120
        os.kill(pty["pid"], signal.SIGWINCH)
        expect(command)
        # Normal Unicode editing resumes after the passive panel; never model input.
        ui.send(pty, " 日本語 e\u0301")
    finally:
        pty_helpers.stop_pty(pty, wait_for)
    assert len(fixture.provider.bodies) == before, "inbox panel admitted inference"
    drafts = [json.loads(p.read_text()) for p in fixture.root.rglob("helm-views/*.json")]
    assert any(d["text"].startswith(command) and "日本語" in d["text"] for d in drafts), "composer not retained"
    assert b"\x1b[?1049l" in pty["output"], "alternate terminal screen not restored"
    fixture.record("notification-independent-helm-cli-tui", {"event": event_id, "inference_calls": before,
        "wide": [120, 32], "narrow": [40, 18], "draft_preserved": True})


def journey(fixture, gateway):
    session = fixture.session()
    recipient = gateway.grant(session, ["observe", "history", "decide"], lifetime=180000)
    stranger = gateway.grant(session, ["observe", "history", "decide"], lifetime=180000)
    vessel = fixture.request({"op": "capabilities"})["vessel_id"]
    dest = destination(fixture, session, recipient, vessel)
    op = {"operation": "configure", "command_id": uid(), "destination": dest}
    assert call(fixture, gateway, recipient, op).get("error"), "remote configured source delegation"
    configured = success(call(fixture, gateway, None, op))
    assert success(call(fixture, gateway, None, op)) == configured
    assert call(fixture, gateway, None, {**op, "destination": {**dest, "recipient_principal_id": uid()}}).get("error")
    listing = success(call(fixture, gateway, stranger, {"operation": "destinations"}))
    assert not listing["destinations"], "foreign destination metadata disclosed"
    accept = {"operation": "accept", "command_id": uid(), "destination_id": dest["id"]}
    assert call(fixture, gateway, stranger, accept).get("error")
    test = {"operation": "test", "command_id": uid(), "destination_id": dest["id"]}
    assert call(fixture, gateway, None, test).get("error"), "unsubscribed recipient received data"
    success(call(fixture, gateway, recipient, accept))
    receipt = success(call(fixture, gateway, None, test))
    assert success(call(fixture, gateway, None, test)) == receipt, "lost-ack retry changed acceptance"
    inbox = {"operation": "inbox", "destination_id": dest["id"], "after": 0, "limit": 100}
    assert call(fixture, gateway, stranger, inbox).get("error")
    page = success(call(fixture, gateway, recipient, inbox))["page"]
    assert len(page["entries"]) == 1 and page["entries"][0]["notification"]["kind"] == "test"
    event = receipt["event_id"]
    opened = success(call(fixture, gateway, recipient,
        {"operation": "open", "destination_id": dest["id"], "event_id": event}))
    assert opened["actionable"] is False and "decision" not in opened
    with ThreadPoolExecutor(max_workers=2) as pool:
        receipts = list(pool.map(lambda state: success(call(fixture, gateway, recipient,
            {"operation": "receipt", "destination_id": dest["id"], "event_id": event, "state": state})),
            ["dismissed", "seen"]))
    assert any(r["state"] == "dismissed" for r in receipts)
    assert success(call(fixture, gateway, recipient, inbox))["page"]["entries"][0]["receipt"]["state"] == "dismissed"
    assert not fixture.provider.bodies, "receipt/open/test started inference"
    fixture.record("notification-synthetic-authority-receipts", {"destination": dest["id"], "event": event})

    submit = fixture.submit(session, "notification real terminal fixture")
    fixture.command(session, submit)
    terminal = fixture.finished(session)
    def delivered():
        value = success(call(fixture, gateway, recipient, inbox))
        return next((e for e in value["page"]["entries"] if e["notification"]["kind"] == "completed"), None)
    complete = wait_for(delivered)
    assert complete["notification"]["run_id"] == terminal["run"]["run_id"]
    # A fresh client credential-file reader proves persistence, not a shared local view cursor.
    credential_file = fixture.root / "recipient-access.json"
    credential_file.write_text(json.dumps(recipient))
    credential_file.chmod(0o600)
    client_checks(fixture, credential_file, dest["id"], complete["notification"]["event_id"])
    fixture.record("notification-real-terminal", complete)
    fixture.suspended(session)
    before = len(fixture.provider.bodies)
    fixture.supervisor.terminate()
    fixture.supervisor.wait(timeout=10)
    fixture.start()
    reconnected = success(call(fixture, gateway, recipient, inbox))["page"]
    assert any(e["notification"]["event_id"] == complete["notification"]["event_id"] for e in reconnected["entries"])
    assert len(fixture.provider.bodies) == before, "notification reconnect replayed inference"
    success(call(fixture, gateway, None, {"operation": "revoke", "command_id": uid(), "destination_id": dest["id"]}))
    assert not success(call(fixture, gateway, recipient, inbox))["page"]["entries"]
    assert success(call(fixture, gateway, recipient,
        {"operation": "open", "destination_id": dest["id"], "event_id": event}))["actionable"] is False
    assert call(fixture, gateway, None, {"operation": "configure", "command_id": uid(), "destination": dest}).get("error"), "revoked ID restored"
    fixture.request({"op": "revoke_grant", "command_id": uid(), "grant_id": recipient["grant_id"], "expected_revision": 1})
    assert call(fixture, gateway, recipient, inbox).get("error")
    fixture.record("notification-restart-revoke", {"inference_calls": before, "event": complete["notification"]["event_id"]})


def approval_handoff(fixture, gateway):
    session = fixture.session(approval=True)
    approver = gateway.grant(session, ["observe", "history", "decide"], lifetime=180000)
    viewer = gateway.grant(session, ["observe", "history"], lifetime=180000)
    vessel = fixture.request({"op": "capabilities"})["vessel_id"]
    destinations = []
    for credential in [approver, viewer]:
        dest = destination(fixture, session, credential, vessel)
        dest["event_kinds"] = ["attention"]
        success(call(fixture, gateway, None, {"operation": "configure", "command_id": uid(), "destination": dest}))
        success(call(fixture, gateway, credential, {"operation": "accept", "command_id": uid(), "destination_id": dest["id"]}))
        destinations.append(dest)
    fixture.command(session, fixture.submit(session, "approval-fixture notification separation"))
    def attention(credential, dest):
        value = success(call(fixture, gateway, credential,
            {"operation": "inbox", "destination_id": dest["id"], "after": 0, "limit": 10}))
        return next((row["notification"] for row in value["page"]["entries"]
            if row["notification"]["kind"] == "attention"), None)
    event = wait_for(lambda: attention(approver, destinations[0]))
    other = wait_for(lambda: attention(viewer, destinations[1]))
    assert event["source_event_id"] == other["source_event_id"]
    viewed = success(call(fixture, gateway, viewer,
        {"operation": "open", "destination_id": destinations[1]["id"], "event_id": other["event_id"]}))
    assert viewed["status"] == "decision_authority_unavailable" and "decision" not in viewed
    request = {"operation": "open", "destination_id": destinations[0]["id"], "event_id": event["event_id"]}
    opened = success(call(fixture, gateway, approver, request))
    assert opened["status"] == "current" and opened["approval_granted"] is False
    assert opened["actionable"] is False
    decision = opened["decision"]
    assert decision["decision_id"] == event["decision_id"]
    assert success(call(fixture, gateway, approver, request))["decision"] == decision
    success(call(fixture, gateway, approver, {"operation": "receipt", "destination_id": destinations[0]["id"],
        "event_id": event["event_id"], "state": "dismissed"}))
    pending = gateway.call(approver, session, {"op": "decisions"})
    assert any(row["decision_id"] == decision["decision_id"] for row in pending["result"]), "dismiss decided request"
    assert not (fixture.workspace / "approval-output.txt").exists(), "notification operation dispatched a tool"
    snapshot = fixture.command(session, {"op": "snapshot"})
    response = {"op": "respond", "command_id": uid(), "incarnation": decision["incarnation"],
        "run_id": decision["run_id"], "decision_id": decision["decision_id"], "response": "denied",
        "expected_revision": snapshot["revision"], "expires_at_ms": int(time.time() * 1000) + 60000}
    decision_receipt = gateway.call(approver, session, response)
    assert not decision_receipt.get("error"), decision_receipt
    assert gateway.call(approver, session, response) == decision_receipt
    fixture.finished(session)
    assert not (fixture.workspace / "approval-output.txt").exists()
    after = success(call(fixture, gateway, approver, request))
    assert after["status"] in ["resolved_or_expired", "unavailable", "expired_or_revoked", "stale"]
    assert not after["actionable"] and "decision" not in after
    fixture.record("notification-explicit-owner-handoff", {"event": event["event_id"], "decision": decision["decision_id"],
        "notification_side_effects": 0, "owner_response": "denied"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    fixture = NotificationFixture(args.bin_dir.resolve())
    gateway = None
    try:
        fixture.start()
        gateway = Gateway(fixture)
        journey(fixture, gateway)
        approval_handoff(fixture, gateway)
    finally:
        if gateway:
            gateway.close()
        fixture.close()


if __name__ == "__main__":
    main()
