"""Offline Linux process fixture; never reads operator configuration or credentials."""

import collections
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import struct
import subprocess
import tempfile
import threading
import time
import uuid


def wait_for(observe, description, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = observe()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(f"timed out waiting for {description}")


class Provider(http.server.ThreadingHTTPServer):
    """Each prompt has an explicit response gate, so overlap is observable."""

    daemon_threads = False

    def __init__(self):
        super().__init__(("127.0.0.1", 0), ProviderHandler)
        self.requests = collections.Counter()
        self.gates = {}
        self.file_tasks = set()
        self.errors = []
        self.guard = threading.Lock()
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    def hold(self, prompt, write_file=False):
        with self.guard:
            gate = threading.Event()
            self.gates[prompt] = gate
            if write_file:
                self.file_tasks.add(prompt)
            return gate

    def count(self, prompt):
        with self.guard:
            return self.requests[prompt]

    def close(self):
        with self.guard:
            for gate in self.gates.values():
                gate.set()
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)


class ProviderHandler(http.server.BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(5)

    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            assert self.path == "/v1/chat/completions", self.path
            size = int(self.headers["Content-Length"])
            assert 0 < size <= 1024 * 1024, size
            body = json.loads(self.rfile.read(size))
            assert body["stream"] is True, "expected actual streaming execution"
            prompt = next(m["content"] for m in reversed(body["messages"])
                          if m["role"] == "user")
            with self.server.guard:
                gate = self.server.gates[prompt]
                self.server.requests[prompt] += 1
                attempt = self.server.requests[prompt]
                write_file = prompt in self.server.file_tasks
            # Do not finish either request until the test has observed both.
            assert gate.wait(45), f"provider response gate timed out: {prompt}"
            delta = {"role": "assistant", "content": f"answer:{prompt}"}
            finish = "stop"
            if write_file and attempt == 1:
                delta = {"tool_calls": [{"index": 0, "id": f"write-{prompt}",
                    "type": "function", "function": {"name": "write_file",
                    "arguments": json.dumps({"path": f"{prompt}.txt", "content": prompt})}}]}
                finish = "tool_calls"
            elif write_file:
                assert attempt == 2, "unexpected child inference/replay"
                result = body["messages"][-1]
                assert result["role"] == "tool", result
                assert result["tool_call_id"] == f"write-{prompt}", result
            chunk = {"choices": [{"index": 0, "delta": delta,
                "finish_reason": finish}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1}}
            payload = ("data: " + json.dumps(chunk) + "\n\ndata: [DONE]\n\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when the test cancels a held request.
        except Exception as error:
            with self.server.guard:
                self.server.errors.append(str(error))
            self.send_error(500)


class Fixture:
    def __init__(self, bin_dir):
        # Short paths also leave room for the per-session Unix socket name.
        self.root = Path(tempfile.mkdtemp(prefix="vct-", dir="/tmp"))
        self.workspace = self.root / "workspace"
        self.directory = self.root / "v"
        self.binary = bin_dir / "voyage"
        self.binaries = {self.binary}
        self.sessions = {}
        self.pids = {}
        self.supervisor = None
        self.provider = Provider()
        self.log = (self.root / "vessel.log").open("wb")
        self.env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
                    "HOME": str(self.root / "home"),
                    "XDG_CONFIG_HOME": str(self.root / "config"),
                    "XDG_DATA_HOME": str(self.root / "data"),
                    "XDG_STATE_HOME": str(self.root / "state"),
                    "XDG_CACHE_HOME": str(self.root / "cache"),
                    "FIXTURE_API_KEY": "synthetic-fixture-key"}
        for path in [self.workspace, *(Path(self.env[key]) for key in
                     ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
                      "XDG_STATE_HOME", "XDG_CACHE_HOME"))]:
            path.mkdir(mode=0o700)
        self.config = self.root / "config.toml"
        self.config.write_text(
            'provider = "openai-chat"\nmodel = "fixture-model"\n'
            'api_key_env = "FIXTURE_API_KEY"\n'
            f'base_url = "http://127.0.0.1:{self.provider.server_port}/v1"\n'
            'access = "unrestricted"\nmax_tokens = 64\ncontext_window = 0\n'
            'subagent_max_concurrency = 1\nprovider_retry_attempts = 1\n'
            'command_timeout_secs = 10\n')
        self.config.chmod(0o600)
        self.bin_dir = bin_dir

    def start(self):
        self.binaries.add(self.binary)
        self.supervisor = subprocess.Popen(
            [str(self.bin_dir / "vessel"), "local-serve", "--directory",
             str(self.directory), "--voyage-binary", str(self.binary)],
            env=self.env, cwd=self.workspace, stdin=subprocess.DEVNULL,
            stdout=self.log, stderr=self.log, start_new_session=True)
        def ready():
            assert self.supervisor.poll() is None, "Vessel exited during startup"
            try:
                self.request({"op": "capabilities"})
                return True
            except (FileNotFoundError, ConnectionRefusedError):
                return False
        wait_for(ready, "Vessel readiness")

    def restart_supervisor(self, binary):
        self.supervisor.terminate()
        self.supervisor.wait(timeout=10)
        self.binary = binary
        self.start()

    def request(self, command, *, envelope=False):
        payload = json.dumps({"protocol": 1, "command": command}).encode()
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(30)
            client.connect(str(self.directory / "vessel.sock"))
            client.sendall(struct.pack("!I", len(payload)) + payload)
            with client.makefile("rb") as incoming:
                header = incoming.read(4)
                assert len(header) == 4, "truncated Vessel frame header"
                size, = struct.unpack("!I", header)
                assert 0 < size <= 4 * 1024 * 1024, "invalid Vessel frame length"
                payload = incoming.read(size)
                assert len(payload) == size, "truncated Vessel response"
        response = json.loads(payload)
        assert response["protocol"] == 1, response
        if envelope:
            return response
        assert response.get("error") is None, response
        return response["result"]

    def new(self, configured=True):
        session = str(uuid.uuid4())
        # Retain the target before asking the supervisor to create a process.
        self.sessions[session] = None
        command = {"op": "start_configured" if configured else "start",
                   "session_id": session, "command_id": str(uuid.uuid4()),
                   "workspace": str(self.workspace)}
        if configured:
            command["config_path"] = str(self.config)
        info = self.request(command)
        assert info["state"] in ("live", "suspended"), info
        self.sessions[session] = info["incarnation"]
        health = self.command(session, {"op": "health"})
        assert health["session_id"] == session, health
        self.pids[session] = health["pid"]
        return session

    def raw_command(self, session, command):
        # Refresh the currently registered process identity before admission. Tests
        # that exercise stale identity rejection use request() directly.
        info = self.request({"op": "inspect", "session_id": session})
        incarnation = info["incarnation"]
        self.sessions[session] = incarnation
        response = self.request({"op": "forward", "session_id": session,
                                 "incarnation": incarnation, "command": command})
        assert response.get("resumed_from") in (None, incarnation), response
        self.sessions[session] = response["incarnation"]
        return response

    def command(self, session, command):
        response = self.raw_command(session, command)
        assert response.get("error") is None, response
        return response["result"]

    def snapshot(self, session):
        return self.command(session, {"op": "snapshot"})

    def tool(self, session, tool_name, **arguments):
        run = self.snapshot(session)["run"]["run_id"]
        command = self.mutation(session, "execute_tool", run_id=run,
                                name=tool_name, arguments=arguments)
        self.command(session, command)
        def observe():
            receipt = self.command(session, {"op": "receipt", "command_id": command["command_id"]})
            if receipt is None or receipt.get("outcome") == "pending_or_unknown":
                return None
            outcome = receipt["outcome"]
            assert outcome["status"] == "completed", receipt
            return outcome
        receipt = wait_for(observe, f"{tool_name} tool completion")
        return json.loads(receipt["output"])

    def mutation(self, session, op, **fields):
        return {"op": op, "command_id": str(uuid.uuid4()),
                "expected_revision": self.snapshot(session)["revision"],
                "expires_at_ms": int(time.time() * 1000) + 120_000, **fields}

    def submit(self, session, prompt):
        gate = self.provider.hold(prompt)
        command = self.mutation(session, "submit", prompt=prompt)
        receipt = self.command(session, command)
        assert receipt["status"] == "accepted", receipt
        return gate, command, receipt["run_id"]

    def reached_provider(self, session, prompt):
        def observe():
            snapshot = self.snapshot(session)
            run = snapshot["run"]
            assert run["state"] in ("accepted", "running"), (
                f"voyage failed before concurrent provider execution: {run}")
            assert not self.provider.errors, self.provider.errors
            return self.provider.count(prompt) == 1 and run["state"] == "running"
        wait_for(observe, f"provider request for {prompt}")
        self.pids[session] = self.command(session, {"op": "health"})["pid"]

    def finished(self, session, state="completed"):
        def observe():
            snapshot = self.snapshot(session)
            run = snapshot["run"]
            if run["state"] in ("accepted", "running", "cancel_requested"):
                return None
            assert run["state"] == state, run
            return snapshot if snapshot["pending_cleanup_run"] is None else None
        return wait_for(observe, f"{state} run and observed cleanup")

    def active_reservations(self):
        path = self.root / "data/helm/host-resources/reservations.sqlite3"
        with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as db:
            return dict(db.execute(
                "SELECT owner, units FROM reservations WHERE kind='executors' AND observed=0"))

    def owned_pids(self):
        # Covers a start that failed before a health response, without trusting
        # a stale PID or reading another runtime's registration/credentials.
        matches = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                argv = (entry / "cmdline").read_bytes().split(b"\0")
                if (len(argv) > 4 and argv[0] in {os.fsencode(path) for path in self.binaries}
                        and argv[1:3] == [b"serve", b"--directory"]):
                    if Path(os.fsdecode(argv[3])).parent == self.directory / "sessions":
                        matches.append(int(entry.name))
            except (FileNotFoundError, ProcessLookupError):
                pass
        return matches

    def close(self):
        self.provider.close()
        # Runtime owners start independent process sessions; stopping Vessel
        # alone would leak them. Signal only the exact fixture-owned processes.
        forced = []
        for sig in (signal.SIGTERM, signal.SIGKILL):
            for pid in self.owned_pids():
                try:
                    # Bind the signal to this process lifetime, not a reusable PID.
                    descriptor = os.pidfd_open(pid)
                    try:
                        if pid in self.owned_pids():
                            if sig == signal.SIGKILL:
                                forced.append(pid)
                            signal.pidfd_send_signal(descriptor, sig)
                    finally:
                        os.close(descriptor)
                except ProcessLookupError:
                    pass
            deadline = time.monotonic() + 5
            while self.owned_pids() and time.monotonic() < deadline:
                time.sleep(0.02)
        remaining = self.owned_pids()
        if self.supervisor is not None:
            self.supervisor.terminate()
            try:
                self.supervisor.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.supervisor.kill()
                self.supervisor.wait(timeout=5)
        self.log.close()
        assert not remaining, f"fixture runtime cleanup unconfirmed: {remaining}"
        assert not forced, f"fixture runtime needed forced termination: {forced}"
        for session in self.pids:
            stopped = self.directory / "sessions" / session / "stopped.json"
            assert stopped.exists(), f"missing runtime cleanup evidence for {session}"
            assert json.loads(stopped.read_text())["cleanup_observed"] is True
