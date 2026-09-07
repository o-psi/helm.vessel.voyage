"""Offline failure regression through Vessel + voyage; no real credentials/model calls."""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
import time
import urllib.request
import uuid


def wait_for(observe, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("timed out waiting for runtime state")


class Quota(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        assert self.path == "/responses"
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert body["stream"] is True
        self.server.requests += 1
        payload = json.dumps({"error": {"type": "usage_limit_reached",
            "message": "private-provider-diagnostic", "plan_type": "private-plan",
            "resets_at": 1789319074}}).encode()
        self.send_response(429)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    binaries = args.bin_dir.resolve()
    root = Path(tempfile.mkdtemp(prefix="vql-"))
    print(f"evidence: {root}", flush=True)
    directory = root / "vessel"
    workspace = root / "workspace"
    workspace.mkdir()
    env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
    for key in ("HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
        path = root / key.lower()
        path.mkdir(mode=0o700)
        env[key] = str(path)
    tokens = Path(env["XDG_DATA_HOME"]) / "helm/chatgpt-oauth.json"
    tokens.parent.mkdir(mode=0o700)
    tokens.write_text(json.dumps({"access_token": "synthetic-access", "refresh_token": "synthetic-refresh",
        "account_id": "synthetic-account", "expires_at": int(time.time()) + 3600}))
    tokens.chmod(0o600)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Quota)
    server.requests = 0
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / "config.toml"
    config.write_text('provider = "chatgpt-oauth"\nmodel = "fixture-model"\n'
        f'chatgpt_base_url = "http://127.0.0.1:{server.server_port}"\n'
        'provider_retry_attempts = 4\nprovider_retry_initial_ms = 10\nprovider_retry_max_ms = 20\n'
        'access = "read-only"\ncontext_window = 0\n')
    config.chmod(0o600)
    log = (root / "vessel.log").open("wb")
    supervisor = None
    session = str(uuid.uuid4())

    def request(command):
        credential = json.loads((directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v3/process/command",
            data=json.dumps({"protocol": 1, "command": command}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=20) as incoming:
            value = json.load(incoming)
        assert value.get("error") is None, value
        return value["result"]

    def command(value):
        info = request({"op": "inspect", "session_id": session})
        response = request({"op": "forward", "session_id": session,
            "incarnation": info["incarnation"], "command": value})
        assert response.get("error") is None, response
        return response["result"]

    def owned_pids():
        pids = []
        for path in Path("/proc").iterdir():
            if not path.name.isdigit():
                continue
            try:
                argv = (path / "cmdline").read_bytes().split(b"\0")
                if (argv[:3] == [os.fsencode(binaries / "voyage"), b"serve", b"--directory"]
                        and len(argv) > 3 and Path(os.fsdecode(argv[3])).parent == directory / "sessions"):
                    pids.append(int(path.name))
            except (FileNotFoundError, ProcessLookupError):
                pass
        return pids

    try:
        supervisor = subprocess.Popen([str(binaries / "vessel"), "local-serve", "--directory",
            str(directory), "--voyage-binary", str(binaries / "voyage")], env=env,
            cwd=workspace, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / "process-http.json").exists())
        request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
            "workspace": str(workspace), "config_path": str(config)})
        # Two explicitly submitted turns: no automatic retry or stuck cleanup,
        # and the first failure remains visible after a new runtime incarnation.
        for turn in range(2):
            snapshot = command({"op": "snapshot"})
            command({"op": "submit", "command_id": str(uuid.uuid4()),
                "expected_revision": snapshot["revision"], "expires_at_ms": int(time.time()*1000) + 60000,
                "prompt": f"hello {turn}"})

            def finished():
                saved = command({"op": "snapshot"})
                return saved if saved["run"]["state"] == "failed" and saved["pending_cleanup_run"] is None else None

            snapshot = wait_for(finished)
            (root / f"turn-{turn+1}.json").write_text(json.dumps(snapshot, indent=2))
            assert server.requests == turn + 1, f"account exhaustion retried: {server.requests} requests"
            assert "account usage limit reached" in snapshot["run"]["failure_summary"]
            assert len(snapshot["turns"]) == turn + 1
            for summary in snapshot["turns"]:
                assert summary["phase"] == "failed", summary
                assert "account usage limit reached" in summary["failure_summary"], summary
            assert all(m["role"] == "user" for m in snapshot["messages"])
            assert "private-provider-diagnostic" not in json.dumps(snapshot)
            assert "private-plan" not in json.dumps(snapshot)
            wait_for(lambda: request({"op": "inspect", "session_id": session})["state"] == "suspended")
        print("PASS: one request per explicit turn; failed status, safe quota explanation, retained history and cleanup")
    finally:
        for pid in owned_pids():
            try:
                descriptor = os.pidfd_open(pid)
                try:
                    if pid in owned_pids():
                        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
                finally:
                    os.close(descriptor)
            except ProcessLookupError:
                pass
        wait_for(lambda: not owned_pids())
        if supervisor is not None:
            supervisor.terminate()
            supervisor.wait(timeout=10)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        log.close()


if __name__ == "__main__":
    main()
