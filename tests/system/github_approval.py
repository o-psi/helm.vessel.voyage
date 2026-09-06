#!/usr/bin/env python3
"""POSIX PTY checks of the actual cfg(test) standalone GitHub approver.

No provider or GitHub transport is constructed. The test executable is explicitly
supplied; production Helm has no fixture command or alternate GitHub origin.
"""
import argparse
import errno
import fcntl
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

DRIVER = "github::approval_fixture::terminal_driver"
HEADER = re.compile(rb"Exact GitHub preview \[(\d+)/(\d+)\]")
CAP = 2 * 1024 * 1024


class Terminal:
    def __init__(self, binary, env, columns=80, rows=12, mode="attended"):
        self.master, self.slave = pty.openpty()
        self.initial = termios.tcgetattr(self.slave)
        self.resize(columns, rows)
        self.output = bytearray()
        self.deadline = time.monotonic() + 15
        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)
        self.process = subprocess.Popen(
            [binary, "--exact", DRIVER, "--nocapture"],
            stdin=self.slave, stdout=self.slave, stderr=self.slave,
            env=dict(env, HELM_GITHUB_APPROVAL_DRIVER=mode),
            preexec_fn=controlling_terminal,
        )

    def resize(self, columns, rows):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, columns, 0, 0))

    def pump(self, seconds=.05):
        assert time.monotonic() < self.deadline, bytes(self.output[-4000:])
        if select.select([self.master], [], [], seconds)[0]:
            try:
                data = os.read(self.master, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                data = b""
            self.output.extend(data)
            assert len(self.output) <= CAP, "PTY output exceeded fixture bound"

    def wait(self, predicate):
        while not predicate(bytes(self.output)):
            self.pump()
            assert self.process.poll() is None or predicate(bytes(self.output)), bytes(self.output[-4000:])

    def send(self, data):
        os.write(self.master, data)

    def page(self):
        count = len(HEADER.findall(self.output))
        self.send(b" ")
        self.wait(lambda data: len(HEADER.findall(data)) > count)
        self.wait(lambda data: b"[n/Esc] deny" in data[data.rfind(b"Exact GitHub preview"):])

    def final_page(self):
        for _ in range(100):
            frame = bytes(self.output[self.output.rfind(b"Exact GitHub preview"):])
            if b"[y] confirm" in frame:
                assert b"FINAL_EXACT_BODY" in frame
                return
            self.page()
        raise AssertionError("preview did not reach its final page")

    def finish(self, expected, entered=True):
        self.wait(lambda data: f"DRIVER_OUTCOME={expected}".encode() in data)
        while self.process.poll() is None:
            self.pump()
        assert self.process.returncode == 0, bytes(self.output[-4000:])
        assert termios.tcgetattr(self.slave) == self.initial, "terminal flags not restored"
        if entered:
            for sequence in (b"\x1b[?1049h", b"\x1b[?2004h", b"\x1b[?2004l", b"\x1b[?1049l"):
                assert sequence in self.output, (sequence, bytes(self.output[-4000:]))
        return bytes(self.output)

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)


def run_case(binary, env, case):
    tiny = case in ("narrow", "short")
    ui = Terminal(binary, env, columns=59 if case == "narrow" else 80,
                  rows=7 if case == "short" else 12,
                  mode="unattended" if case == "unattended" else "attended")
    try:
        if tiny or case == "unattended":
            ui.finish("Unavailable", entered=not case == "unattended")
            assert b"Exact GitHub preview" not in ui.output
            return
        ui.wait(lambda data: b"[n/Esc] deny" in data)
        if case == "confirm":
            ui.send(b"y")
            # A subsequent explicit page proves premature confirmation was ignored.
            ui.page()
            ui.final_page()
            for index in range(50):
                assert f"BODY_{index:03}:".encode() in ui.output
            assert "🧭".encode() in ui.output and b"\\u001b\\u202e" in ui.output
            ui.send(b"y")
            ui.finish("Approved")
        elif case in ("paste", "split-paste"):
            ui.final_page()
            ui.send(b"\x1b[200~y\x1b[201~" if case == "paste" else b"\x1b")
            ui.finish("Denied")
        elif case == "resize":
            ui.final_page()
            count = len(HEADER.findall(ui.output))
            ui.resize(60, 8)
            ui.wait(lambda data: len(HEADER.findall(data)) > count)
            ui.wait(lambda data: b"[n/Esc] deny" in data[data.rfind(b"Exact GitHub preview"):])
            assert HEADER.findall(ui.output)[-1][0] == b"1", "resize did not restart exact preview"
            ui.send(b"y")
            ui.page()
            ui.final_page()
            ui.send(b"y")
            ui.finish("Approved")
        elif case == "resize-unusable":
            ui.resize(10, 3)
            ui.finish("Unavailable")
        elif case == "cancel":
            os.kill(ui.process.pid, signal.SIGUSR1)
            ui.finish("Cancelled")
        else:
            ui.send({"deny": b"n", "escape": b"\x1b", "ctrl-c": b"\x03"}[case])
            ui.finish("Denied")
    finally:
        ui.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--test-binary", required=True)
    args = parser.parse_args()
    binary = str(Path(args.test_binary).resolve())
    with tempfile.TemporaryDirectory(prefix="helm-github-approval-") as directory:
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "TERM": "xterm-256color",
               "HOME": directory, "XDG_CONFIG_HOME": directory + "/config",
               "XDG_DATA_HOME": directory + "/data", "RUST_BACKTRACE": "0"}
        for case in ("confirm", "deny", "escape", "ctrl-c", "paste", "split-paste",
                     "resize", "resize-unusable", "cancel", "narrow", "short", "unattended"):
            run_case(binary, env, case)
            print(f"PASS GitHub approval PTY {case}", flush=True)
        result = subprocess.run([binary, "--exact", DRIVER, "--nocapture"],
                                env=dict(env, HELM_GITHUB_APPROVAL_DRIVER="attended"),
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=10)
        assert result.returncode == 0 and b"DRIVER_OUTCOME=Unavailable" in result.stdout
        assert len(result.stdout) < CAP and b"Exact GitHub preview" not in result.stdout
        print("PASS GitHub approval non-TTY unavailable", flush=True)


if __name__ == "__main__":
    main()
