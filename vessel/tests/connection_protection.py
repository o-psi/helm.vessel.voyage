"""Offline Linux #10 CLI/pairing process checks; synthetic secrets, no provider calls."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.request
import uuid


def wait_for(probe):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            result = probe()
            if result:
                return result
        except (OSError, ValueError):
            pass
        time.sleep(.05)
    raise AssertionError("fixture readiness timeout")


def main(binaries):
    root = Path(tempfile.mkdtemp(prefix="voyage-connection-protection-"))
    print(f"evidence: {root}", flush=True)
    env = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"}
    for name in ["HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"]:
        path = root / name.lower()
        path.mkdir(mode=0o700)
        env[name] = str(path)
    processes = []
    results = []
    with tempfile.TemporaryDirectory(prefix="voyage-connection-key-", dir="/dev/shm") as keydir:
        key = Path(keydir) / "key"
        key.write_bytes(os.urandom(32))
        key.chmod(0o600)
        env["VOYAGE_CREDENTIAL_KEY_FILE"] = str(key)
        state = root / "vessel"
        workspace = root / "workspace"
        workspace.mkdir(mode=0o700)
        log = (root / "processes.log").open("wb")

        def cli(program, *args, unlocked=True, success=True):
            use_env = dict(env)
            if not unlocked:
                use_env.pop("VOYAGE_CREDENTIAL_KEY_FILE")
            result = subprocess.run([str(binaries / program), *map(str, args)], env=use_env,
                cwd=workspace, capture_output=True, timeout=15)
            assert (result.returncode == 0) == success, "unexpected CLI status (private diagnostics retained)"
            with (root / "cli.log").open("ab") as output:
                output.write(result.stdout + result.stderr)
            return result

        def start(args):
            process = subprocess.Popen([str(binaries / "vessel"), *map(str, args)], env=env,
                cwd=workspace, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
            processes.append(process)
            return process

        try:
            start(["local-serve", "--directory", state, "--voyage-binary", binaries / "voyage"])
            wait_for(lambda: (state / "process-http.json").exists())
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            origin = f"http://127.0.0.1:{port}"
            gateway_args = ["--bind", f"127.0.0.1:{port}", "--database", root / "gateway.db",
                "--process-directory", state, "--public-origin", origin, "--allow-insecure-loopback"]
            gateway = start(gateway_args)
            capabilities = wait_for(lambda: json.load(urllib.request.urlopen(origin + "/v1/vessel/pair/capabilities", timeout=2)))
            principal = str(uuid.uuid4())
            invitation_path = root / "invitation.json"
            invite_args = ["pair-invite", "--directory", state, "--endpoint", origin,
                "--principal", principal, "--workspace", workspace, "--output", invitation_path]
            cli("vessel", *invite_args, unlocked=False, success=False)
            assert not invitation_path.exists()
            cli("vessel", *invite_args)
            invitation = json.loads(invitation_path.read_text())
            request = {"protocol": 1, "command_id": str(uuid.uuid4()), "principal_id": principal,
                "invitation_id": invitation["invitation_id"], "code": invitation["code"]}

            def pair():
                req = urllib.request.Request(origin + "/v1/vessel/pair", data=json.dumps(request).encode(),
                    headers={"Content-Type": "application/json", "x-voyage-vessel": capabilities["vessel_id"]})
                with urllib.request.urlopen(req, timeout=5) as response:
                    return json.load(response)

            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                attempts = list(pool.map(lambda _: pair(), range(2)))
            assert any(result.get("error") is None for result in attempts)
            result = pair()
            assert result.get("error") is None
            credential = result["result"]
            journal = state / "access/pairing/state.json"
            sealed = journal.read_bytes()
            assert sealed.startswith(b"VOYAGE-CREDENTIAL-1\0")
            assert credential["token"].encode() not in sealed
            assert invitation["code"].encode() not in sealed
            assert pair()["result"] == credential
            assert journal.read_bytes() == sealed
            results.append("mandatory-key, real pairing, concurrent exact redemption and encrypted restart material")

            def audit(limit=64, cursor=None, unlocked=True, success=True):
                args = ["connection-audit", "--directory", state, "--limit", limit]
                if cursor is not None:
                    args += ["--cursor", cursor]
                output = cli("vessel", *args, unlocked=unlocked, success=success)
                return json.loads(output.stdout) if success else None

            first = audit(2)
            assert first["horizon_sequence"] == 4
            gateway.terminate()
            gateway.wait(timeout=10)
            # Administration and journal reads do not require a running gateway.
            command = str(uuid.uuid4())
            revocation_args = ["revoke-connection", "--directory", state, "--grant", credential["grant_id"],
                "--expected-revision", 1, "--command-id", command]
            revocation = json.loads(cli("vessel", *revocation_args).stdout)
            assert revocation["cleanup"] == "not_observed"
            after = journal.read_bytes()
            assert json.loads(cli("vessel", *revocation_args).stdout) == revocation
            assert journal.read_bytes() == after
            second = audit(2, first["next_cursor"])
            assert [event["sequence"] for event in second["events"]] == [3, 4]
            assert second["next_cursor"] is None
            assert len(audit()["events"]) == 6
            audit(unlocked=False, success=False)
            inventory = json.loads(cli("vessel", "list-connections", "--directory", state, unlocked=False).stdout)
            assert inventory["connections"][0]["revoked"] is True
            assert journal.read_bytes() == after
            gateway = start(gateway_args)
            wait_for(lambda: json.load(urllib.request.urlopen(origin + "/v1/vessel/pair/capabilities", timeout=2)))
            assert pair().get("error") is not None
            results.append("fixed audit horizon across append/restart, revoked retry, locked audit and unlocked inventory")

            connections = root / "helm-connections"
            connections.mkdir(mode=0o700)
            originals = {}
            for suffix, content in [("credential", credential), ("redemption", request), ("tmp", request)]:
                path = connections / (str(uuid.uuid4()) + "." + suffix)
                path.write_text(json.dumps(content))
                path.chmod(0o600)
                originals[path] = path.read_bytes()
            cli("helm", "protect-connections", "--directory", connections, unlocked=False, success=False)
            assert all(path.read_bytes() == data for path, data in originals.items())
            cli("helm", "protect-connections", "--directory", connections)
            assert all(path.read_bytes().startswith(b"VOYAGE-CREDENTIAL-1\0") for path in originals)
            before = {path: path.read_bytes() for path in originals}
            saved_key = key.read_bytes()
            key.write_bytes(os.urandom(32))
            cli("helm", "protect-connections", "--directory", connections, success=False)
            assert all(path.read_bytes() == data for path, data in before.items())
            key.write_bytes(saved_key)
            # Unsafe custody is refused even with the correct bytes.
            persistent_key = root / "synthetic-key-refused"
            persistent_key.write_bytes(saved_key)
            persistent_key.chmod(0o600)
            env["VOYAGE_CREDENTIAL_KEY_FILE"] = str(persistent_key)
            cli("helm", "protect-connections", "--directory", connections, success=False)
            persistent_key.unlink()
            link = Path(keydir) / "linked-key"
            link.symlink_to(key)
            env["VOYAGE_CREDENTIAL_KEY_FILE"] = str(link)
            cli("helm", "protect-connections", "--directory", connections, success=False)
            link.unlink()
            env["VOYAGE_CREDENTIAL_KEY_FILE"] = str(key)
            os.link(key, link)
            cli("helm", "protect-connections", "--directory", connections, success=False)
            link.unlink()
            key.write_bytes(saved_key + b"x")
            cli("helm", "protect-connections", "--directory", connections, success=False)
            key.write_bytes(saved_key)
            cli("helm", "protect-connections", "--directory", connections)
            cli("vessel", "protect-connections", "--directory", state)
            results.append("explicit Helm/Vessel migration, missing/wrong-key preservation and exact-key recovery")
            for log_path in [root / "cli.log", root / "processes.log"]:
                data = log_path.read_bytes()
                for secret in [credential["token"], invitation["code"]]:
                    assert secret.encode() not in data
            assert not list((state / "sessions").iterdir()), "fixture must never create voyages"
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            log.close()
            assert all(process.poll() is not None for process in processes)
    results.append("all fixture processes exited; no inference or native-platform claim")
    (root / "results.json").write_text(json.dumps({"passed": results}, indent=2))
    print(json.dumps({"passed": len(results), "evidence": str(root)}), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, required=True)
    args = parser.parse_args()
    main(args.bin_dir.resolve())
