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
import signal
import subprocess
import sys
import time
import uuid

from delivery_recovery import Fixture, wait_for


class Extensions(Fixture):
    def session(self, required=True, access="unrestricted", config_in_workspace=False):
        session = str(uuid.uuid4())
        self.sessions.append(session)
        config = (self.workspace if config_in_workspace else self.root) / (session + ".toml")
        config.write_text('provider = "openai-responses"\nmodel = "fixture-model"\napi_key_required = false\n'
            f'base_url = "http://127.0.0.1:{self.provider.server_port}"\n'
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
        self.last_tool_command = {"op": "operator_tool", "command_id": str(uuid.uuid4()),
            "expected_revision": self.command(session, {"op": "snapshot"})["revision"],
            "expires_at_ms": int(time.time() * 1000) + 60000, "name": name, "arguments": arguments}
        return self.command(session, self.last_tool_command)

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
    parser.add_argument("--transform-bin", type=Path, required=True)
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
        first_admission, snapshot = fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "structured-output-marker"})
        assert "structured-output-marker" in last_output(snapshot)
        events = fixture.status(binding)["latest"]
        assert {event["action"] for event in events} >= {"lifecycle.run_start", "tool.probe", "lifecycle.run_finish"}, events
        assert all(event["cleanup"] == "observed" for event in events), events
        original_ids = {event["invocation"] for event in events}
        replay = fixture.command(session, fixture.last_tool_command)
        assert replay["run_id"] == first_admission["run_id"], replay
        assert {event["invocation"] for event in fixture.status(binding)["latest"]} == original_ids
        assert len([event for event in events if event["action"] == "lifecycle.run_start"]) == 1
        assert len([event for event in events if event["action"] == "lifecycle.run_finish"]) == 1
        fixture.record("real-pack-install-review-invoke-lifecycle", {"snapshot": snapshot, "events": events})
        _, snapshot = fixture.tool(session, "extcmd_conformance_echo_command", {"text": "operator-command-output"})
        assert "operator-command-output" in last_output(snapshot)
        _, snapshot = fixture.tool(session, "ext_conformance_probe", {"mode": "isolation"})
        text = last_output(snapshot)
        for key in ("host_file_denied", "network_denied", "session_escape_denied", "environment_empty", "private_tmp_writable", "descriptors_private", "system_runtime_absent"):
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
        control_id = str(uuid.uuid4())
        control = {"op": "execute_tool", "command_id": control_id,
            "expected_revision": fixture.command(session, {"op": "snapshot"})["revision"],
            "expires_at_ms": int(time.time()*1000)+60000, "run_id": admitted["run_id"],
            "name": "list_directory", "arguments": {"path": "."}}
        fixture.command(session, control)
        def control_done():
            receipt = fixture.command(session, {"op": "receipt", "command_id": control_id})
            return receipt if receipt.get("outcome", {}).get("status") == "completed" else None
        concurrent = wait_for(control_done, timeout=5)
        active_events = [event for event in fixture.status(binding)["latest"] if event["run"] == admitted["run_id"]]
        assert len([event for event in active_events if event["action"] == "lifecycle.run_start"]) == 1, active_events
        fixture.record("concurrent-control-no-lifecycle-replay", concurrent)
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

        for mode in ("crash", "flood", "wrong_id", "duplicate", "host_capability", "held_child"):
            _, adverse = fixture.tool(session, "ext_conformance_probe", {"mode": mode}, success=(mode == "held_child"))
            if mode == "held_child":
                assert '"child_started":true' in last_output(adverse).replace(" ", ""), adverse
            wait_for(lambda: fixture.inspect_package()["pending_execution"] == 0)
            # A subsequent explicit turn is new work, not replay of the failed call.
            fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "fresh-after-" + mode})
            fixture.record(mode + "-cleanup-and-fresh-run", fixture.status(binding))

        # Kill only this fixture-owned runtime, not its independent guardian.
        # Recovery must observe old resources and retain unknown effects, not replay.
        interrupted = fixture.start_tool(session, "ext_conformance_probe", {"mode": "block"})
        wait_for(lambda: any(event["run"] == interrupted["run_id"] and event["action"] == "tool.probe"
            and event["cleanup"] == "pending" for event in fixture.status(binding)["latest"]))
        matches = [(pid, argv) for pid, argv in fixture.owned_processes().items()
            if Path(os.fsdecode(argv[3])).name == session]
        assert len(matches) == 1, matches
        pid, argv = matches[0]
        descriptor = os.pidfd_open(pid)
        try:
            assert fixture.owned_processes().get(pid) == argv
            signal.pidfd_send_signal(descriptor, signal.SIGKILL)
        finally:
            os.close(descriptor)
        def recovered():
            try:
                snapshot = fixture.command(session, {"op": "snapshot"})
                return snapshot if snapshot.get("run", {}).get("state") == "interrupted" else None
            except (OSError, AssertionError):
                return None
        recovered_state = wait_for(recovered)
        wait_for(lambda: fixture.inspect_package()["pending_execution"] == 0)
        old_events = [event for event in fixture.status(binding)["latest"] if event["run"] == interrupted["run_id"]]
        assert len([event for event in old_events if event["action"] == "tool.probe"]) == 1, old_events
        assert next(event for event in old_events if event["action"] == "tool.probe")["outcome"] == "unknown", old_events
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo", "text": "fresh-after-runtime-crash"})
        fixture.record("runtime-crash-guardian-recovery-no-replay", {"events": old_events, "snapshot": recovered_state})

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

        transform, transform_sha, _ = archive(fixture, args.transform_bin.resolve(),
            source / "sdk/extension-v1/cmd/transform/definitions.json", name="transform")
        fixture.cli("install", transform)
        fixture.cli("review-executable", "transform", "--expected", transform_sha, "--capability", "execute")
        _, transformed = fixture.tool(session, "ext_transform_uppercase", {"text": "real standalone transformation"})
        assert "REAL STANDALONE TRANSFORMATION" in last_output(transformed)
        fixture.record("standalone-transformation-example", transformed)

        reader, reader_sha, _ = archive(fixture, args.read_bin.resolve(),
            source / "sdk/extension-v1/cmd/read/definitions.json", name="reader")
        fixture.cli("install", reader)
        fixture.cli("review-executable", "reader", "--expected", reader_sha,
            "--capability", "execute", "--capability", "host.file.read")
        (fixture.workspace / "allowed.txt").write_text("brokered-file-marker")
        _, snapshot = fixture.tool(session, "ext_reader_read_text", {"path": "allowed.txt"})
        assert "brokered-file-marker" in last_output(snapshot)
        fixture.tool(session, "ext_reader_read_text", {"path": "/etc/passwd"}, success=False)
        private_session = fixture.session(config_in_workspace=True)
        fixture.tool(private_session, "ext_reader_read_text", {"path": private_session + ".toml"}, success=False)
        os.link(fixture.workspace / "allowed.txt", fixture.workspace / "hardlink.txt")
        fixture.tool(session, "ext_reader_read_text", {"path": "hardlink.txt"}, success=False)
        fixture.record("separately-authorized-host-read-and-refusal", snapshot)
        # A colliding manifest is excluded as a whole before initialization.
        # Its executable need not cooperate for namespace collision refusal.
        collision_definitions = json.loads((source / "sdk/extension-v1/cmd/read/definitions.json").read_text())
        collision_definitions["tools"][0]["name"] = "text"
        unique = dict(collision_definitions["tools"][0]); unique["name"] = "unique"
        collision_definitions["tools"].append(unique)
        collision_path = fixture.root / "collision-definitions.json"
        collision_path.write_text(json.dumps(collision_definitions))
        collision, collision_sha, _ = archive(fixture, args.read_bin.resolve(), collision_path, name="reader-read")
        fixture.cli("install", collision)
        fixture.cli("review-executable", "reader-read", "--expected", collision_sha, "--capability", "execute")
        collision_binding = fixture.inspect_package("reader-read")["binding"]
        fixture.tool(session, "ext_reader_read_unique", {"path": "allowed.txt"}, success=False)
        assert fixture.status(collision_binding)["latest"] == []
        fixture.record("collision-excludes-whole-package-before-effects", fixture.inspect_package("reader-read"))

        fixture.cli("disable", "conformance", "--expected", next_sha)
        fixture.tool(session, "ext_conformance_probe", {"mode": "echo"}, success=False)
        fixture.cli("remove", "conformance", "--expected", next_sha)
        fixture.record("disable-remove", json.loads(fixture.cli("list")))
    finally:
        fixture.close()
    print("PASS: executable packaging/activation, supervised invocation, isolation and bounded failure workflow")


if __name__ == "__main__":
    main()
