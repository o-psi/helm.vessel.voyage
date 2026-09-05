#!/usr/bin/env python3
"""Black-box clean-break regression: no legacy work, preserved data, honest status.

The former positive pairing/lease/retry tests are superseded because those features
are deliberately removed (#77). Every former route is now tested for rejection.
"""
import base64
import concurrent.futures
import json
import os
import pathlib
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
BINARY = pathlib.Path(os.environ.get("VESSEL_BIN", ROOT / "target/release/vessel")).resolve()
HELM = pathlib.Path(os.environ.get("HELM_BIN", ROOT / "target/release/helm")).resolve()
TOKEN = "fixture-operator-not-a-real-secret"
ID = "00000000-0000-0000-0000-000000000001"
RETIRED = [
    ("POST", "/v1/pairings/start"), ("POST", "/v1/pairings/claim"),
    ("GET", "/v1/pairings/TEST"), ("POST", "/v1/worker/heartbeat"),
    ("GET", "/v1/worker/tasks/next"),
    ("POST", f"/v1/worker/tasks/{ID}/result"),
    ("POST", f"/v1/worker/tasks/{ID}/failure"),
    ("GET", "/v1/helms"), ("GET", f"/v1/helms/{ID}"),
    ("DELETE", f"/v1/helms/{ID}"), ("POST", f"/v1/helms/{ID}/tasks"),
    ("GET", "/v1/tasks"), ("GET", f"/v1/tasks/{ID}"),
    ("POST", f"/v1/tasks/{ID}/cancel"), ("GET", "/v1/fleet/summary"),
    ("GET", f"/ui/helms/{ID}"), ("POST", f"/ui/helms/{ID}/tasks"),
    ("GET", f"/ui/tasks/{ID}"), ("POST", f"/ui/tasks/{ID}/cancel"),
    ("POST", f"/ui/tasks/{ID}/retry"),
]


def request(base, method, path, expected=200, authorization=None, body=None, request_id=None):
    headers = {"Content-Type": "application/json"}
    if authorization is not None:
        headers["Authorization"] = authorization
    if request_id is not None:
        headers["X-Request-ID"] = request_id
    req = urllib.request.Request(base + path, data=body, headers=headers, method=method)
    try:
        response = urllib.request.urlopen(req, timeout=3)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        status, payload, headers = response.code, response.read(), response.headers
    assert status == expected, (method, path, expected, status, payload)
    assert headers.get("X-Request-ID"), (method, path, "missing correlation")
    return payload.decode(), headers


def stop(process):
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def start(database, log, enabled=True):
    with socket.socket() as candidate:
        candidate.bind(("127.0.0.1", 0))
        port = candidate.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    env = os.environ.copy()
    env.pop("VESSEL_OPERATOR_TOKEN", None)
    if enabled:
        env["VESSEL_OPERATOR_TOKEN"] = TOKEN
    process = subprocess.Popen(
        [str(BINARY), "--bind", f"127.0.0.1:{port}", "--database", str(database)],
        env=env, stdout=log, stderr=log,
    )
    try:
        for _ in range(50):
            if process.poll() is not None:
                raise AssertionError("Vessel exited before becoming healthy")
            try:
                payload, _ = request(base, "GET", "/health")
                assert json.loads(payload)["status"] == "ok"
                return process, base
            except (OSError, urllib.error.URLError):
                time.sleep(0.1)
        raise AssertionError("Vessel failed to become healthy")
    except BaseException:
        stop(process)
        raise


def reject_cli(binary, args, env):
    result = subprocess.run([str(binary), *args], env=env, capture_output=True, timeout=5)
    assert result.returncode == 2, (binary.name, args, result.returncode)
    assert b"unexpected argument" in result.stderr or b"unrecognized subcommand" in result.stderr


def main():
    assert BINARY.is_file() and HELM.is_file(), "build release binaries first"
    with tempfile.TemporaryDirectory() as temporary:
        root = pathlib.Path(temporary)
        # Isolate local Helm data and preserve even malformed old enrollment bytes.
        config = root / "config"
        data = root / "data"
        (config / "helm").mkdir(parents=True)
        (data / "helm/sessions").mkdir(parents=True)
        enrollment = config / "helm/voyage.json"
        enrollment.write_bytes(b"obsolete enrollment: must not be loaded or changed")
        saved = data / "helm/sessions/preserved.json"
        saved.write_bytes(b"canonical local history sentinel")
        env = {**os.environ, "XDG_CONFIG_HOME": str(config), "XDG_DATA_HOME": str(data), "HOME": str(root)}
        before = {path: path.read_bytes() for path in [enrollment, saved]}
        for args in [["--voyage"], ["--voyage=http://127.0.0.1:1"], ["chat", "--voyage"],
                     ["--name", "old-worker"], ["attach", "https://example.invalid", "fixture-key"]]:
            reject_cli(HELM, args, env)
        for args in [["pair", "voyage:v1:TEST"], ["fleet"], ["--lease-secs", "1"],
                     ["--stale-after-secs", "1"], ["--pairing-ttl-secs", "1"]]:
            reject_cli(BINARY, args, env)
        for binary in [HELM, BINARY]:
            for args in [["--help"], ["completions", "bash"], ["manpage"]]:
                result = subprocess.run([str(binary), *args], env=env, capture_output=True, timeout=5, check=True)
                assert b"--voyage" not in result.stdout
                assert b"pairing-ttl-secs" not in result.stdout
        assert all(path.read_bytes() == content for path, content in before.items())

        database = root / "vessel.db"
        # Seed a realistic legacy snapshot containing credentials, queued/running work,
        # and private transcript data. It must never enter any response or log.
        private = "PRIVATE-LEGACY-SENTINEL"
        snapshot = json.dumps({"pairings": {"OLD": private}, "worker_tokens": {private: ID},
                               "helms": {ID: {"status": "online"}},
                               "queues": {ID: [{"prompt": private}]},
                               "tasks": {ID: {"state": "running", "result": private}}})
        with sqlite3.connect(database) as connection:
            connection.executescript("CREATE TABLE control_plane(id INTEGER PRIMARY KEY, state TEXT);"
                                     "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY);"
                                     "INSERT INTO schema_migrations VALUES(1);")
            connection.execute("INSERT INTO control_plane VALUES(1, ?)", [snapshot])
        database_before = database.read_bytes()
        log_path = root / "server.log"
        with log_path.open("wb") as log:
            for enabled in [True, True, False]:  # restart must not recover/retry legacy work
                server, base = start(database, log, enabled)
                try:
                    payload, _ = request(base, "GET", "/ready")
                    assert json.loads(payload)["status"] == "ready"
                    payload, _ = request(base, "GET", "/metrics")
                    assert "voyage_connectivity_enabled 0" in payload
                    assert "voyage_tasks" not in payload and "voyage_helms" not in payload
                    for path in ["/ui", "/v1/diagnostics"]:
                        request(base, "GET", path, 401 if enabled else 503)
                        request(base, "GET", path, 401 if enabled else 503, "Bearer wrong")
                        if enabled:
                            for auth in ["Bearer " + TOKEN, "Basic " + base64.b64encode(("operator:" + TOKEN).encode()).decode()]:
                                payload, _ = request(base, "GET", path, authorization=auth)
                                assert "unavailable" in payload and private not in payload
                            if path == "/v1/diagnostics":
                                assert json.loads(payload)["legacy_state"] == "not_loaded"
                    # Authentication cannot restore old routes; malformed bodies don't
                    # reach retired parsers. Parallel requests cannot enqueue anything.
                    def reject(route):
                        method, path = route
                        for auth in [None, "Bearer " + private, "Bearer " + TOKEN]:
                            payload, _ = request(base, method, path, 404, auth, b"{malformed")
                            assert private not in payload
                    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
                        list(pool.map(reject, RETIRED))
                    _, headers = request(base, "GET", "/health", request_id="safe-request_1")
                    assert headers["X-Request-ID"] == "safe-request_1"
                    for unsafe in ["bad id", "x" * 129]:
                        _, headers = request(base, "GET", "/health", request_id=unsafe)
                        assert headers["X-Request-ID"] != unsafe
                    request(base, "POST", "/ui", 405, "Bearer " + TOKEN, b"{}")
                    assert database.read_bytes() == database_before
                finally:
                    stop(server)
                assert database.read_bytes() == database_before
            # Fresh installations remain usable as a management-plane shell.
            fresh_server, base = start(root / "fresh.db", log)
            try:
                request(base, "GET", "/ready")
            finally:
                stop(fresh_server)
        logs = log_path.read_text()
        assert TOKEN not in logs and private not in logs
        assert all(path.read_bytes() == content for path, content in before.items())
    print("Vessel clean-break lifecycle passed: retired CLI/routes, auth, restart, data preservation")


if __name__ == "__main__":
    main()
