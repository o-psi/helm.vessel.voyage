#!/usr/bin/env python3
"""Actual offline Helm CLI orphan recovery and full attended confirmation.

A cfg(test) seeder creates synthetic records through Store. Every observation and
maintenance command afterward runs the production Helm binary without credentials,
provider configuration, GitHub transport overrides, or live API calls.
"""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

CAP = 2 * 1024 * 1024
HEADER = re.compile(rb"Exact GitHub preview \[(\d+)/(\d+)\]")


def attended(command, env, root, confirm=None, success=True):
    master, slave = pty.openpty()
    initial = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 14, 80, 0, 0))
    def setup():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    with tempfile.TemporaryFile() as output:
        process = subprocess.Popen(command, cwd=root, env=env, stdin=slave,
                                   stdout=output, stderr=slave, preexec_fn=setup)
        terminal = bytearray()
        deadline = time.monotonic() + 20
        frames = 0
        decided = False
        try:
            while process.poll() is None:
                assert time.monotonic() < deadline, bytes(terminal[-3000:])
                if select.select([master], [], [], .05)[0]:
                    try:
                        terminal.extend(os.read(master, 65536))
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                assert len(terminal) <= CAP
                assert os.fstat(output.fileno()).st_size <= CAP
                matches = HEADER.findall(terminal)
                frame = bytes(terminal[terminal.rfind(b"Exact GitHub preview"):])
                if confirm is not None and not decided and len(matches) > frames:
                    if b"[y] confirm" in frame:
                        os.write(master, b"y" if confirm else b"n")
                        decided = True
                    elif b"[n/Esc] deny" in frame:
                        os.write(master, b" ")
                    else:
                        continue
                    frames = len(matches)
            while select.select([master], [], [], .05)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not data:
                    break
                terminal.extend(data)
                assert len(terminal) <= CAP
            assert (process.returncode == 0) == success, bytes(terminal[-4000:])
            assert termios.tcgetattr(slave) == initial, "CLI did not restore terminal flags"
            if confirm is not None:
                assert decided, "mutation never displayed complete exact confirmation"
                for sequence in (b"\x1b[?1049h", b"\x1b[?2004h", b"\x1b[?2004l", b"\x1b[?1049l"):
                    assert sequence in terminal, (sequence, bytes(terminal[-3000:]))
            output.seek(0)
            text = output.read().decode()
            return json.loads(text) if success else text
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
            os.close(master)
            os.close(slave)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-binary")
    parser.add_argument("--helm-bin", default=os.environ.get("HELM_BIN", "target/release/helm"))
    args = parser.parse_args()
    if args.test_binary is None:
        # Select Cargo's actual library test artifact, never a stale glob match.
        build = subprocess.run(
            ["cargo", "test", "-p", "helm", "--lib", "--all-features", "--no-run", "--message-format=json"],
            text=True, capture_output=True, timeout=600, check=True,
        )
        artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
        binaries = [item["executable"] for item in artifacts
                    if item.get("reason") == "compiler-artifact"
                    and item.get("target", {}).get("name") == "helm"
                    and "lib" in item.get("target", {}).get("kind", [])
                    and item.get("profile", {}).get("test") and item.get("executable")]
        assert len(binaries) == 1, "expected exactly one current Helm library test artifact"
        args.test_binary = binaries[0]
    test_binary, helm = str(Path(args.test_binary).resolve()), str(Path(args.helm_bin).resolve())
    with tempfile.TemporaryDirectory(prefix="helm-github-journal-") as directory:
        root = Path(directory)
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "TERM": "xterm-256color",
               "HOME": directory, "XDG_CONFIG_HOME": str(root / "config"),
               "XDG_DATA_HOME": str(root / "data"), "RUST_BACKTRACE": "0"}
        # The seeder bypasses normal Helm startup, which creates this parent.
        (root / "data" / "helm").mkdir(parents=True, mode=0o700)
        seed = subprocess.run([test_binary, "--exact", "github::cli_journal_fixture::seed_orphan_journal", "--nocapture"],
                              env=dict(env, HELM_GITHUB_JOURNAL_SEED=directory), cwd=root,
                              text=True, capture_output=True, timeout=10)
        assert seed.returncode == 0, (seed.stdout + seed.stderr)[-6000:]
        records = json.loads(next(line.removeprefix("SEED_JSON=") for line in seed.stdout.splitlines() if line.startswith("SEED_JSON=")))
        assert not (root / "deleted-workspace").exists()
        config = root / "config.toml"
        config.write_text('github_enabled = false\naccess = "unrestricted"\n')
        base = [helm, "--config", str(config), "--workspace", directory]
        def run(*words, confirm=None, success=True, access=None):
            command = base + (["--access", access] if access else []) + ["github", *words]
            return attended(command, env, root, confirm, success)
        def inspect(record):
            return run("admin", "inspect", record["id"])
        assert len(run("admin", "list")) == 3
        first, second, uncertain = records
        assert inspect(first)["state"] == "prepared"
        assert inspect(uncertain)["state"] == "sending"
        assert run("list") == [], "ordinary scope adopted deleted workspace records"
        run("inspect", first["id"], success=False)
        piped = subprocess.run(base + ["github", "admin", "cancel", first["id"], first["digest"]],
                               env=env, cwd=root, input=b"y\n", capture_output=True, timeout=10)
        assert piped.returncode != 0 and b"attended" in piped.stderr
        assert inspect(first)["state"] == "prepared"
        run("admin", "cancel", first["id"], first["digest"], access="read-only", success=False)
        assert inspect(first)["state"] == "prepared"
        run("admin", "cancel", first["id"], first["digest"], confirm=False, success=False)
        assert inspect(first)["state"] == "prepared"
        run("admin", "cancel", first["id"], first["digest"], confirm=True)
        assert inspect(first)["state"] == "cancelled"
        run("admin", "dispose", uncertain["id"], uncertain["digest"], "Remote outcome remains unknown after inspection", confirm=True)
        assert inspect(uncertain)["state"] == "disposed"
        run("admin", "forget", first["id"], first["digest"], confirm=True)
        exported = run("admin", "audit")
        assert len(exported["entries"]) == 1
        assert exported["entries"][0]["id"] == first["id"]
        run("admin", "forget", uncertain["id"], uncertain["digest"], confirm=True)
        run("admin", "clear-audit", exported["digest"], success=False)
        current = run("admin", "audit")
        assert len(current["entries"]) == 2
        run("admin", "clear-audit", current["digest"], confirm=True)
        assert run("admin", "audit")["entries"] == []
        run("admin", "inspect", first["id"], success=False)
        assert inspect(second)["state"] == "prepared"
        assert len(run("admin", "list")) == 1
    print("GitHub actual offline CLI orphan recovery, denial, approval, audit and restart: PASS")


if __name__ == "__main__":
    main()
