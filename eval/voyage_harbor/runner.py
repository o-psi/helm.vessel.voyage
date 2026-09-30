"""Run one real supervised Voyage in a Harbor container and retain public evidence."""

import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid

BIN = Path("/opt/voyage/bin")
ROOT = Path("/var/lib/voyage-benchmark")
LOG = Path("/logs/agent")


def cleanup_only():
    """Fence execution and observe cleanup before Harbor can inject a verifier."""
    created_path = LOG / "create.json"
    if not created_path.exists():
        raise RuntimeError("Missing trial admission identity; verification prohibited")
    session = json.loads(created_path.read_text())["session_id"]
    directory = ROOT / "vessel"
    registered = directory / "sessions" / session / "registration.json"
    env = {
        k: v for k, v in os.environ.items() if k in ("PATH", "LANG", "LC_ALL", "TERM")
    }
    env["PATH"] = str(BIN) + ":" + env.get("PATH", "/usr/bin:/bin")
    for key in (
        "HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ):
        env[key] = str(ROOT / key.lower())
    registration = json.loads(registered.read_text())
    command = [
        str(BIN / "helm"),
        "connect",
        "--directory",
        str(directory),
        "--no-start",
        "stop",
        session,
        "--incarnation",
        registration["incarnation"],
    ]
    stopped_request = subprocess.run(
        command, env=env, capture_output=True, text=True, timeout=15
    )
    (LOG / "timeout-stop-receipt.json").write_text(stopped_request.stdout)
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        stopped = registered.with_name("stopped.json")
        if stopped.exists():
            proof = json.loads(stopped.read_text())
            current = json.loads(registered.read_text())
            if (
                proof.get("cleanup_observed") is True
                and proof.get("session_id") == session
                and proof.get("incarnation") == current.get("incarnation")
            ):
                evidence = {
                    k: proof.get(k)
                    for k in (
                        "session_id",
                        "incarnation",
                        "cleanup_observed",
                        "suspended",
                    )
                }
                (LOG / "timeout-cleanup.json").write_text(
                    json.dumps(evidence, indent=2)
                )
                return 0
        time.sleep(0.1)
    raise RuntimeError("Timed-out Voyage cleanup unconfirmed; verification prohibited")


def main(input_path):
    options = json.loads(Path(input_path).read_text())
    if options["model"] != "gpt-6.1-sol" or options["effort"] not in (
        "medium",
        "high",
        "xhigh",
    ):
        raise ValueError("Unsupported benchmark model configuration")
    LOG.mkdir(parents=True, exist_ok=True)
    ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(ROOT, 0o700)
    env = {
        k: v for k, v in os.environ.items() if k in ("PATH", "LANG", "LC_ALL", "TERM")
    }
    env["PATH"] = str(BIN) + ":" + env.get("PATH", "/usr/bin:/bin")
    for key in (
        "HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
    ):
        path = ROOT / key.lower()
        path.mkdir(mode=0o700, exist_ok=True)
        env[key] = str(path)
    account, connection, session = (str(uuid.uuid4()) for _ in range(3))
    binding = {
        "account_id": account,
        "connection_id": connection,
        "identity_generation": 1,
        "connection_revision": 1,
        "transport": "chatgpt_oauth",
    }
    descriptor = {
        "id": account,
        "connection_id": connection,
        "alias": "benchmark-relay",
        "label": "Scoped benchmark relay",
        "metadata_revision": 1,
        "identity_generation": 1,
        "credential_revision": 1,
        "capability_revision": 1,
        "availability": "available",
        "state": "ready",
    }
    registry = {
        "revision": 1,
        "default_account": binding,
        "default_revision": 1,
        "connections": [
            {
                "id": connection,
                "revision": 1,
                "label": "Scoped benchmark relay",
                "endpoint": "https://chatgpt.com/backend-api/codex",
                "transports": ["chatgpt_oauth"],
            }
        ],
        "accounts": [
            {
                "descriptor": descriptor,
                "credential": {
                    "OAuth": {
                        "access_token": options["proxy_token"],
                        "refresh_token": "no-upstream-refresh-credential",
                        "account_id": "benchmark-relay",
                        "expires_at": int(time.time()) + 86400,
                    }
                },
                "refresh": None,
                "attested": True,
                "provider_identity": None,
            }
        ],
        "enrollments": [],
        "legacy": None,
    }
    registry_dir = Path(env["XDG_DATA_HOME"]) / "helm/accounts"
    registry_dir.parent.mkdir(mode=0o700, exist_ok=True)
    registry_dir.mkdir(mode=0o700)
    registry_path = registry_dir / "registry.json"
    registry_path.write_text(json.dumps(registry))
    registry_path.chmod(0o600)
    workspace = Path.cwd()
    config = ROOT / "launch.json"
    settings = {
        "provider": "chatgpt-oauth",
        "model": options["model"],
        "reasoning_effort": options["effort"],
        "access": "unrestricted",
        "allow_read": ["/"],
        "allow_write": ["/"],
        "context_window": 0,
        "command_timeout_secs": 120,
        "provider_response_timeout_ms": 120000,
        "provider_stream_idle_ms": 180000,
        "account": binding,
        "chatgpt_base_url": "https://chatgpt.com/backend-api/codex",
    }
    config.write_text(
        json.dumps(
            {
                "version": 1,
                "workspace": str(workspace),
                "config": settings,
                "explicit": {
                    "access": "unrestricted",
                    "read_roots": ["/"],
                    "write_roots": ["/"],
                },
                "selection": None,
                "confirmation": None,
            }
        )
    )
    config.chmod(0o600)
    workspace = Path.cwd()
    directory = ROOT / "vessel"
    directory.mkdir(mode=0o700, exist_ok=True)
    private_log = (ROOT / "processes.log").open("wb")
    relay = subprocess.Popen(
        [
            sys.executable,
            "/installed-agent/voyage/relay.py",
            "--socket",
            "/run/voyage-benchmark/provider.sock",
            "--session",
            session,
        ],
        stdout=private_log,
        stderr=private_log,
    )
    supervisor = subprocess.Popen(
        [
            str(BIN / "vessel"),
            "local-serve",
            "--directory",
            str(directory),
            "--voyage-binary",
            str(BIN / "voyage"),
        ],
        env=env,
        stdout=private_log,
        stderr=private_log,
    )
    prefix = [
        str(BIN / "helm"),
        "connect",
        "--directory",
        str(directory),
        "--no-start",
    ]
    summary = {
        "session_id": session,
        "model": options["model"],
        "reasoning_effort": options["effort"],
        "source": options["source"],
        "completed": False,
        "cleanup": "unconfirmed",
        "usage": {},
        "transport": "native-chatgpt-oauth-through-host-credential-relay",
    }
    started = time.monotonic()

    def run(*args, timeout=30):
        return subprocess.run(
            [*prefix, *args],
            env=env,
            cwd=workspace,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

    def retain(name, text):
        (LOG / name).write_text(text.replace(options["proxy_token"], "[REDACTED]"))

    try:
        for _ in range(150):
            if supervisor.poll() is not None or relay.poll() is not None:
                raise RuntimeError("Benchmark runtime startup failed")
            if (directory / "process-http.json").exists():
                break
            time.sleep(0.1)
        else:
            raise RuntimeError("Benchmark Vessel startup timed out")
        created = run(
            "new",
            "--id",
            session,
            "--workspace",
            str(workspace),
            "--config-path",
            str(config),
        )
        retain("create.json", created.stdout)
        if created.returncode:
            raise RuntimeError("Voyage creation rejected: " + created.stderr)
        result = run("run", session, options["instruction"], timeout=1800)
        retain("response.txt", result.stdout)
        if result.returncode:
            raise RuntimeError("Voyage run failed: " + result.stderr)
        observed = run("inspect", session)
        retain("snapshot.json", observed.stdout)
        if observed.returncode:
            raise RuntimeError("Final Voyage state unavailable")
        snapshot = json.loads(observed.stdout)
        summary["snapshot_keys"] = sorted(snapshot)
        summary["usage"] = snapshot.get("usage", {})
        exported = run("export", session, str(LOG / "conversation.md"))
        if exported.returncode:
            summary["history_export"] = "unavailable"
        else:
            summary["history_export"] = "complete-public-history"
        summary["completed"] = snapshot.get("run", {}).get("state") == "completed"
        if not summary["completed"]:
            raise RuntimeError("Canonical run did not complete")
    except (RuntimeError, subprocess.TimeoutExpired, ValueError) as error:
        summary["failure"] = str(error).replace(options["proxy_token"], "[REDACTED]")[
            :2000
        ]
        try:
            observed = run("inspect", session)
            retain("snapshot.json", observed.stdout)
        except (OSError, subprocess.TimeoutExpired):
            pass
    finally:
        summary["elapsed_seconds"] = round(time.monotonic() - started, 3)
        registered = directory / "sessions" / session / "registration.json"
        stopped = registered.with_name("stopped.json")
        try:
            registration = json.loads(registered.read_text())
            stop = run("stop", session, "--incarnation", registration["incarnation"])
            retain("stop-receipt.json", stop.stdout)
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if stopped.exists():
                    proof = json.loads(stopped.read_text())
                    current = json.loads(registered.read_text())
                    if (
                        proof.get("cleanup_observed") is True
                        and proof.get("session_id") == session
                        and proof.get("incarnation") == current.get("incarnation")
                    ):
                        summary["cleanup"] = "observed-runtime-cleanup"
                        summary["cleanup_receipt"] = {
                            k: proof.get(k)
                            for k in (
                                "session_id",
                                "incarnation",
                                "cleanup_observed",
                                "suspended",
                            )
                        }
                        break
                time.sleep(0.1)
        except (OSError, ValueError, subprocess.TimeoutExpired):
            pass
        # Native successful runs suspend only after runtime-owned cleanup. A killed
        # supervisor is not proof of Voyage cleanup; container retirement is separate.
        for child in (supervisor, relay):
            if child.poll() is None:
                child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
        summary["supervisor_reaped"] = supervisor.poll() is not None
        summary["relay_reaped"] = relay.poll() is not None
        if summary["cleanup"] != "observed-runtime-cleanup":
            summary["cleanup"] = "container-retirement-required"
        (LOG / "voyage-summary.json").write_text(json.dumps(summary, indent=2))
        private_log.close()
        if not summary["completed"]:
            retain(
                "diagnostics.txt", (ROOT / "processes.log").read_text(errors="replace")
            )
        registry_path.unlink(missing_ok=True)
        Path(input_path).unlink(missing_ok=True)
    return 0 if summary["completed"] else 1


if __name__ == "__main__":
    raise SystemExit(
        cleanup_only() if sys.argv[1] == "--cleanup" else main(sys.argv[1])
    )
