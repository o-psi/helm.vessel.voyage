"""Offline #72 adverse HTTP journey; source draft is NOT execution evidence.

Run with the parent's admitted exact --bin-dir (vessel/voyage); no build here.
Synthetic session grants and loopback provider only. No clock manipulation: the
store's fake-clock unit checks own rollback coverage. Pending-outbox restart is
not claimed: start/stop alone cannot deterministically park delivery between an
owner event and its acknowledgement. The existing notifications journey covers
restart of an already delivered event; this file does not relabel that as pending.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import sqlite3
import threading
import time
import traceback
import urllib.error

from notifications import NotificationFixture, call, success, destination
from approval_semantics import Gateway, uid
from delivery_recovery import wait_for


MARKER = "SYNTHETIC-NOT-A-CREDENTIAL-secret-transcript-" + "x" * 32
NOTIFICATION_FIELDS = {
    "event_id", "session_id", "run_id", "incarnation", "kind",
    "source_event_id", "created_at_ms", "expires_at_ms", "decision_id", "budget",
}


def invoke(fixture, gateway, credential, operation):
    # notifications.call handles scoped HTTP errors, but local fixture.request
    # raises them. Only explicit client refusals count, never transport/5xx errors.
    try:
        value = call(fixture, gateway, credential, operation)
    except urllib.error.HTTPError as error:
        if error.code not in (400, 401, 403, 404, 422):
            raise
        return {"error": f"HTTP {error.code}"}
    assert value.get("error") not in ("HTTP 500", "HTTP 502", "HTTP 503", "HTTP 504"), value
    return value


def operation(fixture, gateway, credential, kind, dest, **extra):
    return invoke(fixture, gateway, credential,
        {"operation": kind, "destination_id": dest["id"], **extra})


def configure(fixture, gateway, dest):
    return invoke(fixture, gateway, None,
        {"operation": "configure", "command_id": uid(), "destination": dest})


def accept(fixture, gateway, credential, dest):
    return success(operation(fixture, gateway, credential, "accept", dest, command_id=uid()))


def inbox(fixture, gateway, credential, dest):
    return operation(fixture, gateway, credential, "inbox", dest, after=0, limit=100)


def grant_record(fixture, credential):
    return json.loads((fixture.directory / "access/grants" /
        (credential["grant_id"] + ".json")).read_text())


def metadata(notification):
    assert set(notification) == NOTIFICATION_FIELDS, notification
    assert MARKER not in json.dumps(notification), "source text entered notification metadata"
    assert notification["budget"] is None, notification


def no_payload(value):
    if value.get("error"):
        assert "result" not in value or value["result"] is None, value
        return
    result = value["result"]
    assert "notification" not in result and "decision" not in result, result
    if "page" in result:
        assert result["page"]["entries"] == [], result
    if "actionable" in result:
        assert result["actionable"] is False, result


def authority_matrix(fixture, gateway):
    session, foreign_session = fixture.session(), fixture.session()
    vessel = fixture.request({"op": "capabilities"})["vessel_id"]
    recipient = gateway.grant(session, ["observe", "history"], lifetime=180000)
    foreign = gateway.grant(foreign_session, ["observe", "history"], lifetime=180000)
    base = destination(fixture, session, recipient, vessel)
    cases = {
        "foreign-principal": {"recipient_principal_id": grant_record(fixture, foreign)["principal_id"]},
        "foreign-session": {"source_session_id": foreign_session},
        "missing-source": {"source_session_id": uid()},
        "foreign-vessel": {"source_vessel_id": uid()},
        "wrong-generation": {"recipient_grant_revision": base["recipient_grant_revision"] + 1},
        "foreign-grant": {"recipient_grant_id": foreign["grant_id"]},
        "past-grant-bound": {"expires_at_ms": grant_record(fixture, recipient)["expires_at_ms"] + 1},
    }
    for name, changes in cases.items():
        assert configure(fixture, gateway, {**base, "id": uid(), **changes}).get("error"), name
    observer = gateway.grant(session, ["observe"], lifetime=180000)
    observe_dest = destination(fixture, session, observer, vessel)
    assert configure(fixture, gateway, observe_dest).get("error"), "Observe admitted Budget without History"
    observe_dest["event_kinds"] = ["test", "completed"]
    success(configure(fixture, gateway, observe_dest))
    accept(fixture, gateway, observer, observe_dest)
    # Unknown fields must be rejected rather than quietly retained as metadata.
    for field in ("text", "secret", "title", "url", "tool_arguments"):
        assert configure(fixture, gateway, {**base, "id": uid(), field: MARKER}).get("error"), field
    success(configure(fixture, gateway, base))
    assert operation(fixture, gateway, None, "accept", base, command_id=uid()).get("error")
    assert operation(fixture, gateway, foreign, "accept", base, command_id=uid()).get("error")
    assert operation(fixture, gateway, None, "test", base, command_id=uid()).get("error")
    accept(fixture, gateway, recipient, base)
    assert operation(fixture, gateway, None, "test", base, command_id=uid(), text=MARKER).get("error")
    receipt = success(operation(fixture, gateway, None, "test", base, command_id=uid()))
    page = success(inbox(fixture, gateway, recipient, base))["page"]
    assert len(page["entries"]) == 1, page
    metadata(page["entries"][0]["notification"])
    no_payload(inbox(fixture, gateway, foreign, base))
    assert not fixture.provider.bodies, "notification-only operations started inference"
    # Use real owner-produced metadata, not an injected notification row.
    fixture.command(session, fixture.submit(session, MARKER))
    terminal = fixture.finished(session)
    def completed():
        rows = success(inbox(fixture, gateway, observer, observe_dest))["page"]["entries"]
        return next((r["notification"] for r in rows if r["notification"]["kind"] == "completed"), None)
    event = wait_for(completed, timeout=10)
    metadata(event)
    assert event["run_id"] == terminal["run"]["run_id"]
    fixture.record("notification-adverse-authority-metadata", {
        "rejected_cases": list(cases), "injected_fields_rejected": 5,
        "completed_event": event["event_id"], "test_event": receipt["event_id"]})
    return session, recipient, vessel


def revoked_and_expired(fixture, gateway, session, recipient, vessel):
    for mode in ("destination-revoked", "destination-expired", "grant-revoked", "grant-expired"):
        credential = gateway.grant(session, ["observe", "history"], lifetime=180000)
        dest = destination(fixture, session, credential, vessel)
        if mode == "grant-expired":
            credential = gateway.grant(session, ["observe", "history"], lifetime=1800)
            dest = destination(fixture, session, credential, vessel)
            dest["expires_at_ms"] = grant_record(fixture, credential)["expires_at_ms"]
        elif mode == "destination-expired":
            dest["expires_at_ms"] = int(time.time() * 1000) + 1800
        success(configure(fixture, gateway, dest))
        accept(fixture, gateway, credential, dest)
        test = {"operation": "test", "command_id": uid(), "destination_id": dest["id"]}
        receipt = success(invoke(fixture, gateway, None, test))
        assert len(success(inbox(fixture, gateway, credential, dest))["page"]["entries"]) == 1
        if mode == "destination-revoked":
            success(operation(fixture, gateway, None, "revoke", dest, command_id=uid()))
        elif mode == "grant-revoked":
            fixture.request({"op": "revoke_grant", "command_id": uid(),
                "grant_id": credential["grant_id"], "expected_revision": 1})
        else:
            wait_for(lambda: int(time.time() * 1000) > dest["expires_at_ms"], timeout=2)
        no_payload(inbox(fixture, gateway, credential, dest))
        no_payload(operation(fixture, gateway, credential, "open", dest, event_id=receipt["event_id"]))
        retry = invoke(fixture, gateway, None, test)
        no_payload(retry)
        if not retry.get("error"):
            assert retry["result"] == receipt, "retry changed durable receipt"
        assert operation(fixture, gateway, None, "test", dest, command_id=uid()).get("error"), mode
        fixture.record("notification-adverse-" + mode, {"event": receipt["event_id"], "payload_hidden": True})


def begin_writer(connection):
    # Arrange the barrier before starting the experiment; a concurrent courier
    # may own SQLite briefly. This does not extend any runtime busy/deadline limit.
    def acquire():
        try:
            connection.execute("BEGIN IMMEDIATE")
            return True
        except sqlite3.OperationalError as error:
            if "locked" not in str(error):
                raise
            return False
    wait_for(acquire, timeout=1)


def lock_crossing_expiry(fixture, gateway, session, recipient, vessel):
    dest = destination(fixture, session, recipient, vessel)
    dest["event_kinds"] = ["test"]
    dest["expires_at_ms"] = int(time.time() * 1000) + 1400
    success(configure(fixture, gateway, dest))
    accept(fixture, gateway, recipient, dest)
    success(operation(fixture, gateway, None, "test", dest, command_id=uid()))
    assert len(success(inbox(fixture, gateway, recipient, dest))["page"]["entries"]) == 1
    database = fixture.directory / "notifications/notifications.sqlite3"
    entered = threading.Event()
    def read_during_lock():
        entered.set()
        return inbox(fixture, gateway, recipient, dest)
    # URI mode=rw forbids silently creating a wrong-path replacement database.
    with sqlite3.connect(database.as_uri() + "?mode=rw", uri=True, timeout=0) as locked:
        begin_writer(locked)
        acquired = time.monotonic()
        try:
            assert dest["expires_at_ms"] - int(time.time() * 1000) > 700, "setup consumed lock window"
            with ThreadPoolExecutor(max_workers=1) as pool:
                future = pool.submit(read_during_lock)
                try:
                    assert entered.wait(.2), "reader never entered HTTP call"
                    time.sleep(.08)
                    assert not future.done(), "read failed or escaped held SQLite writer lock"
                    wait_for(lambda: int(time.time() * 1000) > dest["expires_at_ms"], timeout=1.5)
                    assert not future.done(), "request returned before lock release"
                finally:
                    # Always release before executor joins, including assertion errors.
                    locked.rollback()
                held = time.monotonic() - acquired
                assert held < 1.8, "invalid schedule: too close to existing 2s SQLite busy timeout"
                value = future.result(timeout=3)
                # An error/timeout is NOT a pass for stale-payload protection.
                assert success(value)["page"]["entries"] == [], value
        finally:
            locked.rollback()
    fixture.record("notification-adverse-lock-expiry", {"lock_held_seconds": held,
        "destination": dest["id"], "empty_successful_read": True,
        "barrier_limit": "client call entry observed; no server-side lock-acquisition instrumentation"})


def revoke_during_lock(fixture, gateway, session, vessel):
    credential = gateway.grant(session, ["observe", "history"], lifetime=180000)
    dest = destination(fixture, session, credential, vessel)
    dest["event_kinds"] = ["test"]
    success(configure(fixture, gateway, dest))
    accept(fixture, gateway, credential, dest)
    success(operation(fixture, gateway, None, "test", dest, command_id=uid()))
    assert len(success(inbox(fixture, gateway, credential, dest))["page"]["entries"]) == 1
    entered = threading.Event()
    def reader():
        entered.set()
        return inbox(fixture, gateway, credential, dest)
    database = fixture.directory / "notifications/notifications.sqlite3"
    grant_path = fixture.directory / "access/grants" / (credential["grant_id"] + ".json")
    with sqlite3.connect(database.as_uri() + "?mode=rw", uri=True, timeout=0) as locked:
        begin_writer(locked)
        acquired = time.monotonic()
        try:
            with ThreadPoolExecutor(max_workers=1) as pool:
                future = pool.submit(reader)
                try:
                    assert entered.wait(.2)
                    time.sleep(.1)
                    assert not future.done(), "reader did not remain pending under SQLite lock"
                    # HOST-RECORD MUTATION, not a public revoke call: that call
                    # serializes behind the same registration mutex. This only
                    # touches this fixture's fresh synthetic private grant. Keep
                    # the exact binding/revision so only current revocation can
                    # reject the previously valid identity; never restore it.
                    record = grant_record(fixture, credential)
                    assert record["revoked"] is False
                    record["revoked"] = True
                    replacement = grant_path.with_name(uid() + ".tmp")
                    try:
                        fd = os.open(replacement, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                        with os.fdopen(fd, "w") as output:
                            json.dump(record, output)
                            output.flush()
                            os.fsync(output.fileno())
                        os.replace(replacement, grant_path)
                        directory_fd = os.open(grant_path.parent, os.O_RDONLY | os.O_DIRECTORY)
                        try:
                            os.fsync(directory_fd)
                        finally:
                            os.close(directory_fd)
                    finally:
                        replacement.unlink(missing_ok=True)
                    assert not future.done(), "reader completed before writer lock release"
                finally:
                    locked.rollback()
                held = time.monotonic() - acquired
                assert held < 1.8, "invalid schedule: near 2s SQLite busy timeout"
                result = future.result(timeout=3)
                no_payload(result)
                assert result.get("error"), "revoked grant remained authorized"
        finally:
            locked.rollback()
    no_payload(inbox(fixture, gateway, credential, dest))
    fixture.record("notification-adverse-host-revoke-under-lock", {
        "host_record_mutation": True, "grant": credential["grant_id"],
        "lock_held_seconds": held, "payload_hidden": True,
        "barrier_limit": "pending HTTP call proven; cannot distinguish store-open wait from post-authority BEGIN wait"})


def accept_is_not_approval(fixture, gateway):
    session = fixture.session(approval=True)
    viewer = gateway.grant(session, ["observe", "history"], lifetime=180000)
    vessel = fixture.request({"op": "capabilities"})["vessel_id"]
    dest = destination(fixture, session, viewer, vessel)
    dest["event_kinds"] = ["attention"]
    success(configure(fixture, gateway, dest))
    fixture.command(session, fixture.submit(session, "approval-fixture adverse accept " + MARKER))
    decision = wait_for(lambda: fixture.command(session, {"op": "decisions"}), timeout=10)[0]
    accept(fixture, gateway, viewer, dest)
    pending = fixture.command(session, {"op": "decisions"})
    assert any(d["decision_id"] == decision["decision_id"] for d in pending), "Accept resolved approval"
    snapshot = fixture.command(session, {"op": "snapshot"})
    response = {"op": "respond", "command_id": uid(), "incarnation": decision["incarnation"],
        "run_id": decision["run_id"], "decision_id": decision["decision_id"],
        "response": "approved", "expected_revision": snapshot["revision"],
        "expires_at_ms": int(time.time() * 1000) + 60000}
    assert gateway.call(viewer, session, response).get("error"), "Accept conferred Decide"
    assert any(d["decision_id"] == decision["decision_id"]
        for d in fixture.command(session, {"op": "decisions"}))
    assert not (fixture.workspace / "approval-output.txt").exists()
    # Explicit local denial is the separate owner action that settles this fixture.
    response.update(command_id=uid(), response="denied",
        expected_revision=fixture.command(session, {"op": "snapshot"})["revision"])
    fixture.command(session, response)
    fixture.finished(session)
    assert not (fixture.workspace / "approval-output.txt").exists()
    fixture.record("notification-adverse-accept-not-approve", {"decision": decision["decision_id"],
        "viewer_respond_refused": True, "owner_denied": True})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        raise RuntimeError("assertions required; do not run Python with -O")
    fixture = NotificationFixture(args.bin_dir.resolve())
    gateway = None
    primary = None
    cleanup_errors = []
    try:
        fixture.start()
        gateway = Gateway(fixture)
        session, recipient, vessel = authority_matrix(fixture, gateway)
        lock_crossing_expiry(fixture, gateway, session, recipient, vessel)
        revoke_during_lock(fixture, gateway, session, vessel)
        revoked_and_expired(fixture, gateway, session, recipient, vessel)
        accept_is_not_approval(fixture, gateway)
    except BaseException as error:
        primary = error
        (fixture.root / "adverse-primary-error.txt").write_text(traceback.format_exc())
    finally:
        for name, close in (("gateway", gateway.close if gateway else None), ("fixture", fixture.close)):
            if close is None:
                continue
            try:
                close()
            except BaseException as error:
                cleanup_errors.append(error)
                (fixture.root / ("adverse-cleanup-" + name + ".txt")).write_text(traceback.format_exc())
        # Keep all original traceback objects and durable evidence, not just the
        # last finally exception. Python 3.11+ BaseExceptionGroup also keeps Ctrl-C.
        errors = ([primary] if primary is not None else []) + cleanup_errors
        if len(errors) > 1:
            raise BaseExceptionGroup("adverse journey and cleanup failures (see evidence directory)", errors)
        if errors:
            raise errors[0].with_traceback(errors[0].__traceback__)


if __name__ == "__main__":
    main()
