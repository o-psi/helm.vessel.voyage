#!/usr/bin/env python3
"""Offline launcher contract: piped shell, terminal input, verified downloads, cleanup."""
from __future__ import annotations

import errno
import gzip
import hashlib
import os
from pathlib import Path
import pty
import select
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
ASSET = "voyage-installer-x86_64-unknown-linux-gnu.gz"
STUB = b'''#!/bin/sh
printf 'fixture-ready\\n'
IFS= read -r answer
[ "$answer" = terminal-answer ] || exit 42
printf '%s' "$answer" > "$FIXTURE_MARKER"
exit "${FIXTURE_EXIT:-0}"
'''


def executable(path: Path, text: str | bytes) -> None:
    path.write_bytes(text.encode() if isinstance(text, str) else text)
    path.chmod(0o755)


def run_terminal(env: dict[str, str], expected: int) -> bytes:
    pid, master = pty.fork()
    if pid == 0:
        os.execve("/bin/sh", ["sh", "-c", 'cat "$FIXTURE_SCRIPT" | sh'], env)
    output = bytearray()
    sent = False
    reaped = False
    deadline = time.monotonic() + 10
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
            if b"fixture-ready" in output and not sent:
                os.write(master, b"terminal-answer\n")
                sent = True
            done, status = os.waitpid(pid, os.WNOHANG)
            if done:
                reaped = True
                assert os.waitstatus_to_exitcode(status) == expected, bytes(output)
                return bytes(output)
        raise AssertionError(f"launcher hung: {bytes(output)!r}")
    finally:
        if not reaped:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)


def main() -> None:
    for case in ("success", "pinned", "local", "child_failure", "download_failure", "checksum_failure",
                 "mismatch", "malformed", "traversal", "multiline", "corrupt_gzip", "unsupported", "bad_version", "bad_local", "terminated"):
        with tempfile.TemporaryDirectory(prefix="voyage bootstrap ") as raw:
            root = Path(raw)
            bindir = root / "bin"
            bindir.mkdir()
            tmp = root / "temporary files"
            tmp.mkdir()
            payload = gzip.compress(STUB)
            if case == "corrupt_gzip":
                payload = b"not gzip"
            (root / "payload").write_bytes(payload)
            digest = hashlib.sha256(payload).hexdigest()
            manifest = f"{digest}  {ASSET}\n"
            if case == "mismatch":
                manifest = f"{'0' * 64}  {ASSET}\n"
            elif case == "malformed":
                manifest = f"oops  {ASSET}\n"
            elif case == "traversal":
                manifest = f"{digest}  ../../victim\n"
            elif case == "multiline":
                manifest += f"{digest}  other-file\n"
            (root / "manifest").write_text(manifest)
            executable(bindir / "uname", '#!/bin/sh\nif [ "$FIXTURE_CASE" = unsupported ]; then echo Other; elif [ "$1" = -s ]; then echo Linux; else echo x86_64; fi\n')
            executable(bindir / "curl", f'''#!{sys.executable}
import os, pathlib, signal, sys
args = sys.argv[1:]
assert args[args.index('--proto') + 1] == '=https'
assert args[args.index('--proto-redir') + 1] == '=https'
assert '--connect-timeout' in args and '--max-time' in args
root = pathlib.Path(os.environ['FIXTURE_ROOT'])
url = args[-1]
assert url.startswith('https://github.com/o-psi/voyage/releases/')
if os.environ['FIXTURE_CASE'] == 'pinned':
    assert '/download/v0.1.0/' in url
(root / 'requested').write_text(url)
if os.environ['FIXTURE_CASE'] == 'terminated':
    os.kill(os.getppid(), signal.SIGTERM)
    sys.exit(0)
checksum = url.endswith('.sha256')
if os.environ['FIXTURE_CASE'] == ('checksum_failure' if checksum else 'download_failure'):
    sys.exit(22)
pathlib.Path(args[args.index('--output') + 1]).write_bytes((root / ('manifest' if checksum else 'payload')).read_bytes())
''')
            env = os.environ.copy()
            env.pop("VOYAGE_INSTALLER_BIN", None)
            env.pop("VOYAGE_INSTALLER_VERSION", None)
            env.update(PATH=f"{bindir}:/usr/bin:/bin", TMPDIR=str(tmp), FIXTURE_ROOT=str(root),
                       FIXTURE_CASE=case, FIXTURE_MARKER=str(root / "executed"),
                       FIXTURE_SCRIPT=str(ROOT / "install.sh"))
            if case == "local":
                executable(root / "local installer", STUB)
                env["VOYAGE_INSTALLER_BIN"] = os.path.relpath(root / "local installer")
            if case == "bad_local":
                env["VOYAGE_INSTALLER_BIN"] = str(root / "missing")
            if case == "pinned":
                env["VOYAGE_INSTALLER_VERSION"] = "v0.1.0"
            if case == "bad_version":
                env["VOYAGE_INSTALLER_VERSION"] = "../../malicious"
            if case == "child_failure":
                env["FIXTURE_EXIT"] = "23"
            success = case in ("success", "pinned", "local", "child_failure")
            output = run_terminal(env, 143 if case == "terminated" else 23 if case == "child_failure" else 0 if success else 1)
            assert (root / "executed").exists() == success, (case, output)
            assert list(tmp.iterdir()) == [], (case, list(tmp.iterdir()))
            if case in ("local", "bad_local", "unsupported", "bad_version"):
                assert not (root / "requested").exists(), case
            print(f"installer bootstrap {case}: passed")
    with tempfile.TemporaryDirectory(prefix="voyage-package-installer-") as raw:
        root = Path(raw)
        binary = root / "target/release/voyage-installer"
        binary.parent.mkdir(parents=True)
        executable(binary, STUB)
        env = os.environ.copy()
        env.pop("TARGET", None)
        command = [str(ROOT / "scripts/package-installer"), "fixture"]
        packaged = subprocess.run(command, cwd=root, env=env, capture_output=True, timeout=5)
        assert packaged.returncode == 0, packaged.stderr
        assets = list((root / "dist/installer-fixture").glob("*.gz"))
        assert len(assets) == 1
        assert gzip.decompress(assets[0].read_bytes()) == STUB
        expected = hashlib.sha256(assets[0].read_bytes()).hexdigest()
        assert assets[0].with_suffix(".gz.sha256").read_text() == f"{expected}  {assets[0].name}\n"
        repeated = subprocess.run(command, cwd=root, env=env, capture_output=True, timeout=5)
        assert repeated.returncode != 0
        assert hashlib.sha256(assets[0].read_bytes()).hexdigest() == expected
        print("installer packaging integrity and overwrite refusal: passed")
    result = subprocess.run(["sh", str(ROOT / "install.sh")], stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=5, start_new_session=True)
    assert result.returncode == 1 and b"interactive terminal" in result.stderr, result
    print("installer bootstrap no terminal: passed")


if __name__ == "__main__":
    main()
