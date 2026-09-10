"""Offline Linux happy path: a useful native shell command and its saved result.

Uses real Vessel-supervised voyages and a scripted loopback provider, not an account.
"""
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


PROMPT = ("Summarize sales.csv locally: aggregate quantity times unit_cents by item, "
          "write summary.json including total_cents, and report the result.")
ORIGINAL = "item,quantity,unit_cents\napples,3,125\noats,2,240\napples,1,125\n"
EXPECTED = '{"apples": 500, "oats": 480, "total_cents": 980}\n'
STDOUT = "Processed 3 rows; total_cents=980; wrote summary.json\n"
TOOL_RESULT = "exit: 0\nstdout:\n" + STDOUT + "\nstderr:\n"
REPLY = "Processed 3 sales rows: apples 500 cents, oats 480 cents, total 980 cents. Saved summary.json."
# Bounded foreground work: no background jobs, credentials, or remote I/O.
COMMAND = """timeout 5s python3 - <<'PYCOMMAND'
import csv
import json
import os
from pathlib import Path
pid = os.getpid()
identity = {"pid": pid, "start": Path(f"/proc/{pid}/stat").read_text().split(") ", 1)[1].split()[19]}
Path("command-process.json").write_text(json.dumps(identity))
totals = {}
count = 0
with Path("sales.csv").open(newline="") as source:
    for row in csv.DictReader(source):
        totals[row["item"]] = totals.get(row["item"], 0) + int(row["quantity"]) * int(row["unit_cents"])
        count += 1
totals["total_cents"] = sum(totals.values())
Path("summary.json").write_text(json.dumps(totals, sort_keys=True) + "\\n")
print(f"Processed {count} rows; total_cents={totals['total_cents']}; wrote summary.json")
PYCOMMAND"""


def wait_for(observe, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = observe()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError("timed out waiting for runtime state")


def command_alive(identity):
    try:
        stat = Path(f"/proc/{identity['pid']}/stat").read_text()
        return stat.split(") ", 1)[1].split()[19] == identity["start"]
    except FileNotFoundError:
        return False


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        self.connection.settimeout(10)
        try:
            assert self.path == "/v1/chat/completions", self.path
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert body["stream"] is True
            self.server.requests.append(body)
            step = len(self.server.requests)
            messages = body["messages"]
            assert [m["content"] for m in messages if m["role"] == "user"] == [PROMPT]
            if step == 1:
                assert "shell" in {t["function"]["name"] for t in body["tools"]}
                delta = {"tool_calls": [{"index": 0, "id": "summarize-sales", "type": "function",
                         "function": {"name": "shell", "arguments": json.dumps({"command": COMMAND})}}]}
            elif step == 2:
                result = messages[-1]
                assert result["role"] == "tool" and result["tool_call_id"] == "summarize-sales", result
                assert result["content"] == TOOL_RESULT, result
                # Independent filesystem observation, not a model/tool assertion.
                assert self.server.output.read_bytes() == EXPECTED.encode()
                delta = {"content": REPLY}
            else:
                raise AssertionError(f"unexpected provider request {step}")
            reason = "tool_calls" if "tool_calls" in delta else "stop"
            events = [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]},
                      {"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]}]
            payload = ("".join("data: " + json.dumps(event) + "\n\n" for event in events)
                       + "data: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
        except Exception as error:
            self.server.errors.append(f"request {len(self.server.requests)}: {error!r}")
            payload = json.dumps({"error": {"message": "fixture assertion failed"}}).encode()
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    binaries = args.bin_dir.resolve()
    for name in ("vessel", "voyage"):
        assert (binaries / name).is_file(), f"missing binary: {binaries / name}"
    root = Path(tempfile.mkdtemp(prefix="voyage-command-workflow-"))
    print(f"evidence: {root}", flush=True)
    directory = root / "vessel"
    workspace = root / "workspace"
    workspace.mkdir()
    source = workspace / "sales.csv"
    source.write_text(ORIGINAL)
    env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
    for key in ("HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"):
        path = root / key.lower()
        path.mkdir(mode=0o700)
        env[key] = str(path)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    server.requests, server.errors = [], []
    server.output = workspace / "summary.json"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / "config.toml"
    config.write_text('provider = "openai-chat"\nmodel = "fixture-model"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\ncommand_timeout_secs = 10\n'
                      'access = "unrestricted"\ncontext_window = 0\n')
    config.chmod(0o600)
    log = (root / "vessel.log").open("wb")
    supervisor = None
    session = str(uuid.uuid4())

    def request(value):
        credential = json.loads((directory / "process-http.json").read_text())
        req = urllib.request.Request(credential["endpoint"] + "/v1/vessel/command",
            data=json.dumps({"protocol": 1, "command": value}).encode(),
            headers={"Authorization": "Bearer " + credential["token"], "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=10) as incoming:
            reply = json.load(incoming)
        assert reply.get("error") is None, reply
        return reply["result"]

    def command(value):
        reply = request({**value, "session_id": session})
        assert reply.get("error") is None, reply
        return reply["result"]

    def owned_pids():
        pids = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                argv = (entry / "cmdline").read_bytes().split(b"\0")
                if (argv[:3] == [os.fsencode(binaries / "voyage"), b"serve", b"--directory"]
                        and len(argv) > 3 and Path(os.fsdecode(argv[3])).parent == directory / "sessions"):
                    pids.append(int(entry.name))
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
        snapshot = command({"op": "snapshot"})
        command({"op": "submit", "command_id": str(uuid.uuid4()),
                 "expected_revision": snapshot["revision"],
                 "expires_at_ms": int(time.time() * 1000) + 60000, "prompt": PROMPT})

        def finished():
            assert not server.errors, server.errors
            saved = command({"op": "snapshot"})
            state = saved.get("run", {}).get("state")
            assert state not in ("failed", "cancelled"), saved.get("run")
            return saved if state == "completed" and saved.get("pending_cleanup_run") is None else None

        wait_for(finished)
        wait_for(lambda: request({"op": "inspect", "session_id": session})["state"] == "suspended")
        # Read retained state after the independent executor has suspended.
        snapshot = command({"op": "snapshot"})
        (root / "snapshot.json").write_text(json.dumps(snapshot, indent=2))
        messages = snapshot["messages"]
        assert [m["content"] for m in messages if m["role"] == "user"] == [PROMPT]
        assert [m["content"] for m in messages if m["role"] == "assistant" and not m.get("tool_calls")] == [REPLY]
        tools = [m for m in messages if m["role"] == "tool"]
        assert len(tools) == 1 and tools[0]["tool_call_id"] == "summarize-sales", tools
        assert tools[0]["content"] == TOOL_RESULT, tools
        outcome = tools[0]["tool_outcome"]
        assert outcome["execution"] == "succeeded", outcome
        assert outcome["command"] == {"status": "exited", "code": 0}, outcome
        assert outcome.get("incomplete") is None, outcome
        assert len(snapshot["turns"]) == 1 and snapshot["turns"][0]["phase"] == "completed"
        assert server.output.read_bytes() == EXPECTED.encode()
        assert source.read_text() == ORIGINAL, "input changed"
        assert len(server.requests) == 2 and not server.errors, server.errors
        identity = json.loads((workspace / "command-process.json").read_text())
        wait_for(lambda: not command_alive(identity))
    finally:
        try:
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
        finally:
            try:
                if supervisor is not None:
                    supervisor.terminate()
                    try:
                        supervisor.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        supervisor.kill()
                        supervisor.wait(timeout=10)
                        raise AssertionError("supervisor required forced cleanup")
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                log.close()
                (root / "provider-requests.json").write_text(json.dumps(server.requests, indent=2))
                (root / "provider-errors.json").write_text(json.dumps(server.errors, indent=2))
                assert not thread.is_alive(), "provider thread did not stop"
                identity_path = workspace / "command-process.json"
                if identity_path.exists():
                    identity = json.loads(identity_path.read_text())
                    wait_for(lambda: not command_alive(identity))
    assert supervisor.poll() is not None and not owned_pids()
    cleanup = {"supervisor_reaped": True, "voyage_processes_remaining": owned_pids(),
               "command_process_remaining": command_alive(identity), "provider_stopped": not thread.is_alive()}
    assert cleanup["command_process_remaining"] is False, cleanup
    (root / "cleanup.json").write_text(json.dumps(cleanup, indent=2))
    print("PASS: native shell, actual summary file, stdout/exit 0, provider tool result, saved reply and cleanup")


if __name__ == "__main__":
    main()
