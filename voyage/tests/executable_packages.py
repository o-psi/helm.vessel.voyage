"""Focused offline Linux executable-package workflow, using real supervised owners.

Requires explicitly built Helm/Vessel/Voyage and static Go conformance/read binaries.
Never builds, installs a sandbox, spends provider budget or changes host services.
Run only inside the coordinator-admitted verification slot. Failure is not a skip.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import uuid

from delivery_recovery import Fixture, wait_for


class Extensions(Fixture):
    def session(self, required=True, access="unrestricted"):
        session = str(uuid.uuid4())
        self.sessions.append(session)
        config = self.root / (session + ".toml")
        config.write_text('provider = "chatgpt-oauth"\nmodel = "fixture-model"\n'
            f'chatgpt_base_url = "http://127.0.0.1:{self.provider.server_port}"\n'
            'provider_retry_attempts = 1\ncontext_window = 0\ncommand_timeout_secs = 20\n'
            f'access = {json.dumps(access)}\n[sandbox]\nmode = "{"required" if required else "off"}"\n')
        config.chmod(0o600)
        self.request({"op": "start_configured", "session_id": session, "command_id": str(uuid.uuid4()),
            "workspace": str(self.workspace), "config_path": str(config)})
        return session

    def cli(self, *arguments, success=True, scope="user"):
        result = subprocess.run([str(self.binaries / "helm"), "--workspace", str(self.workspace),
            "extension", "--scope", scope, *map(str, arguments)], cwd=self.workspace, env=self.env,
            stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15)
        assert (result.returncode == 0) == success, (arguments, result.returncode, result.stdout, result.stderr)
        return result.stdout

    def inspect_package(self, name="conformance", scope="user"):
        return json.loads(self.cli("inspect", name, scope=scope))

    def status(self, binding):
        return json.loads(self.cli("execution-status", binding))

    def start_tool(self, session, name, arguments):
        return self.command(session, {"op": "execute_tool", "command_id": str(uuid.uuid4()),
            "expected_revision": self.command(session, {"op": "snapshot"})["revision"],
            "expires_at_ms": int(time.time() * 1000) + 60000, "name": name, "arguments": arguments})

    def tool(self, session, name, arguments, success=True):
        admitted = self.start_tool(session, name, arguments)
        snapshot = self.finished(session)
        assert snapshot["run"]["state"] == ("completed" if success else "failed"), snapshot["run"]
        self.suspended(session)
        return admitted, self.command(session, {"op": "snapshot"})


def archive(fixture, binary, definitions, name="conformance", version="1.0.0"):
    package = fixture.root / (name + "-" + version)
    package.mkdir()
    shutil.copyfile(binary, package / "tool")
    capabilities = ["execute"] + (["host.file.read"] if name == "reader" else [])
    manifest = {"format": 2, "id": name, "version": version, "voyage": "0.1", "protocol": 1,
        "platform": "linux-x86_64", "runtime": "static-elf", "entrypoint": "tool",
        "contents": [{"path": "tool", "sha256": hashlib.sha256((package / "tool").read_bytes()).hexdigest()}],
        "capabilities": capabilities, "definitions": json.loads(definitions.read_text())}
    (package / "manifest.json").write_text(json.dumps(manifest))
    output = fixture.root / (name + "-" + version + ".helmpkg")
    sha = fixture.cli("pack", package, output).strip()
    assert sha == hashlib.sha256(output.read_bytes()).hexdigest()
    return output, sha, manifest


def last_output(snapshot):
    return [message["content"] for message in snapshot["messages"] if message["role"] == "assistant"][-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--conformance-bin", type=Path, required=True)
    parser.add_argument("--read-bin", type=Path, required=True)
    args = parser.parse_args()
    assert sys.platform == "linux", "native Linux evidence required; no fallback"
    source = Path(__file__).resolve().parents[2]
    fixture = Extensions(args.bin_dir.resolve())
    fixture.env["VOYAGE_EXTENSION_CANARY"] = "synthetic-not-for-extension-" + uuid.uuid4().hex
    try:
        fixture.start()
        first, sha, manifest = archive(fixture, args.conformance_bin.resolve(),
            source / "sdk/extension-v1/cmd/conformance/definitions.json")
        fixture.cli("install", first)
        state = fixture.inspect_package()
        assert state["execution_reviewed"] is False and state["active"] is False, state
        fixture.cli("enable", "conformance", "--expected", sha, success=False)
        fixture.cli("review-executable", "conformance", "--expected", sha,
            "--capability", "model_context", success=False)
        session = fixture.session()
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "inactive"}, success=False)
        fixture.cli("review-executable", "conformance", "--expected", sha, "--capability", "execute")
        binding = fixture.inspect_package()["binding"]
        _, snapshot = fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "structured-output-marker"})
        assert "structured-output-marker" in last_output(snapshot)
        events = fixture.status(binding)["latest"]
        assert {event["action"] for event in events} >= {"lifecycle.run_start", "tool.probe", "lifecycle.run_finish"}, events
        assert all(event["cleanup"] == "observed" for event in events), events
        fixture.record("real-pack-install-review-invoke-lifecycle", {"snapshot": snapshot, "events": events})
        _, snapshot = fixture.tool(session, "extcmd_conformance_echo_command", {"text": "operator-command-output"})
        assert "operator-command-output" in last_output(snapshot)
        _, snapshot = fixture.tool(session, "ext_conformance_probe", {"mode": "isolation"})
        text = last_output(snapshot)
        for key in ("host_file_denied", "network_denied", "session_escape_denied", "environment_empty", "private_tmp_writable"):
            assert f'"{key}":true' in text.replace(" ", ""), (key, text)
        assert fixture.env["VOYAGE_EXTENSION_CANARY"] not in json.dumps(snapshot)
        fixture.record("required-isolation-and-command", snapshot)

        # Merely reviewing a package cannot turn sandbox-off into executable authority.
        off = fixture.session(required=False)
        fixture.tool(off, "ext_conformance_probe", {"mode": "echo"}, success=False)
        readonly = fixture.session(access="read-only")
        fixture.tool(readonly, "ext_conformance_probe", {"mode": "echo"}, success=False)
        fixture.record("no-fallback-off-and-readonly", {"off": off, "readonly": readonly})

        replacement, next_sha, _ = archive(fixture, args.conformance_bin.resolve(),
            source / "sdk/extension-v1/cmd/conformance/definitions.json", version="1.0.1")
        fixture.cli("update", "conformance", replacement, "--expected", "0" * 64, success=False)
        assert fixture.inspect_package()["execution_reviewed"] is True
        # Revocation happens before drain; code is not replaced under the active call.
        admitted = fixture.start_tool(session, "ext_conformance_probe", {"mode": "block"})
        wait_for(lambda: any(event["action"] == "tool.probe" and event["cleanup"] == "pending"
            for event in fixture.status(binding)["latest"]))
        fixture.cli("update", "conformance", replacement, "--expected", sha, success=False)
        fixture.cli("disable", "conformance", "--expected", sha, success=False)
        state = fixture.inspect_package()
        assert state["digest"] == sha and not state["execution_reviewed"] and state["pending_execution"] > 0, state
        before = fixture.command(session, {"op": "snapshot"})
        fixture.command(session, {"op": "cancel", "command_id": str(uuid.uuid4()),
            "expected_revision": before["revision"], "expires_at_ms": int(time.time()*1000)+60000,
            "run_id": admitted["run_id"]})
        cancelled = fixture.finished(session)
        assert cancelled["run"]["state"] == "cancelled", cancelled["run"]
        fixture.suspended(session)
        wait_for(lambda: fixture.inspect_package()["pending_execution"] == 0)
        fixture.cli("update", "conformance", replacement, "--expected", sha)
        state = fixture.inspect_package()
        assert state["digest"] == next_sha and not state["execution_reviewed"], state
        fixture.cli("review-executable", "conformance", "--expected", sha, "--capability", "execute", success=False)
        fixture.cli("review-executable", "conformance", "--expected", next_sha, "--capability", "execute")
        fixture.record("revoke-before-drain-cancel-update-no-inherited-review", {"state": state, "cancelled": cancelled})

        for mode in ("crash", "flood", "held_child"):
            fixture.tool(session, "ext_conformance_probe", {"mode": mode}, success=False)
            wait_for(lambda: fixture.inspect_package()["pending_execution"] == 0)
            # A subsequent explicit turn is new work, not replay of the failed call.
            fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "fresh-after-" + mode})
            fixture.record(mode + "-cleanup-and-fresh-run", fixture.status(binding))

        # Corrupted bytes/capabilities are rejected during install, not at first effect.
        malformed = json.loads(first.read_text())
        malformed["files"]["tool"] = "AAAA"
        invalid = fixture.root / "invalid.helmpkg"
        invalid.write_text(json.dumps(malformed))
        fixture.cli("update", "conformance", invalid, "--expected", next_sha, success=False)
        assert fixture.inspect_package()["digest"] == next_sha
        fixture.cli("install", first, scope="project")
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo"}, success=False)
        fixture.cli("remove", "conformance", "--expected", sha, scope="project")
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "user-after-shadow-removal"})
        fixture.record("integrity-and-inactive-project-shadow", fixture.inspect_package())

        reader, reader_sha, _ = archive(fixture, args.read_bin.resolve(),
            source / "sdk/extension-v1/cmd/read/definitions.json", name="reader")
        fixture.cli("install", reader)
        fixture.cli("review-executable", "reader", "--expected", reader_sha,
            "--capability", "execute", "--capability", "host.file.read")
        (fixture.workspace / "allowed.txt").write_text("brokered-file-marker")
        _, snapshot = fixture.tool(session, "ext_reader_read_text", {"path": "allowed.txt"})
        assert "brokered-file-marker" in last_output(snapshot)
        fixture.tool(session, "ext_reader_read_text", {"path": "/etc/passwd"}, success=False)
        fixture.record("separately-authorized-host-read-and-refusal", snapshot)
        fixture.cli("disable", "conformance", "--expected", next_sha)
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo"}, success=False)
        fixture.cli("remove", "conformance", "--expected", next_sha)
        fixture.record("disable-remove", json.loads(fixture.cli("list")))
    finally:
        fixture.close()
    print("PASS: executable packaging/activation, supervised invocation, isolation and bounded failure workflow")


if __name__ == "__main__":
    main()
