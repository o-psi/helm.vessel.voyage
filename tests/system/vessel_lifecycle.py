#!/usr/bin/env python3
"""Black-box Vessel pairing, authorization, queue, and result lifecycle test."""
from __future__ import annotations

import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
BINARY = ROOT / "target" / "release" / "vessel"


def request(base: str, method: str, path: str, body=None, token=None, expected=200):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    try:
        with urllib.request.urlopen(urllib.request.Request(base + path, data=data, headers=headers, method=method), timeout=2) as response:
            status, payload = response.status, response.read()
    except urllib.error.HTTPError as error:
        status, payload = error.code, error.read()
    assert status == expected, f"{method} {path}: expected {expected}, got {status}: {payload!r}"
    return json.loads(payload) if payload else None


def main() -> int:
    assert BINARY.is_file(), f"missing {BINARY}; run cargo build --release"
    with socket.socket() as candidate:
        candidate.bind(("127.0.0.1", 0))
        port = candidate.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    descriptor, database_name = tempfile.mkstemp(prefix="vessel-system-", suffix=".db")
    os.close(descriptor)
    pathlib.Path(database_name).unlink()
    server = subprocess.Popen([str(BINARY), "--bind", f"127.0.0.1:{port}", "--database", database_name], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        for _ in range(50):
            try:
                if request(base, "GET", "/health")["status"] == "ok":
                    break
            except Exception:
                time.sleep(0.1)
        else:
            raise AssertionError("Vessel failed to become healthy")
        assert request(base, "GET", "/ready")["status"] == "ready"
        diagnostics = request(base, "GET", "/v1/diagnostics")
        assert diagnostics["protocol_version"] >= 1
        with urllib.request.urlopen(base + "/metrics", timeout=2) as response:
            assert response.headers.get("X-Request-ID")
            assert "voyage_tasks" in response.read().decode()

        helm_id = str(uuid.uuid4())
        pairing = request(base, "POST", "/v1/pairings/start", {
            "helm": {"id": helm_id, "name": "system-test", "version": "test", "model": "mock", "capabilities": ["shell"]},
            "protocol_version": 1
        })
        token = pairing["worker_token"]
        request(base, "GET", f"/v1/pairings/{pairing['code']}", token="wrong", expected=401)
        request(base, "POST", "/v1/pairings/claim", {"connection_string": f"voyage:v1:{pairing['code']}"})
        request(base, "GET", f"/v1/pairings/{pairing['code']}", token=token)
        request(base, "POST", "/v1/worker/heartbeat", {"status": "online", "protocol_version": 1}, token=token)
        task = request(base, "POST", f"/v1/helms/{helm_id}/tasks", {"prompt": "test task", "session_id": None})
        envelope = request(base, "GET", "/v1/worker/tasks/next", token=token)
        assert envelope["id"] == task["id"]
        completed = request(base, "POST", f"/v1/worker/tasks/{task['id']}/result", {
            "lease_id": envelope["lease_id"],
            "result": {"task_id": task["id"], "session_id": str(uuid.uuid4()), "answer": "done", "input_tokens": 1, "output_tokens": 1}
        }, token=token)
        assert completed["state"] == "completed" and completed["result"]["answer"] == "done"
        print("Vessel lifecycle passed: pairing, auth rejection, heartbeat, dispatch, completion")
        return 0
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
        for suffix in ("", "-shm", "-wal"):
            pathlib.Path(database_name + suffix).unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main())
