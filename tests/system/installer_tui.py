#!/usr/bin/env python3
"""Real Ratatui setup through a PTY, including the piped shell entry point."""
from __future__ import annotations

import fcntl
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time

from policy_ceiling import rendered_screen

ROOT = Path(__file__).resolve().parents[2]
BIN = Path(os.environ.get("INSTALLER_BIN", ROOT / "target/release/voyage-installer")).resolve()


class Terminal:
    def __init__(self, root, piped=False):
        self.master, self.slave = pty.openpty()
        self.rows, self.columns = 30, 90
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", self.rows, self.columns, 0, 0))
        self.before = termios.tcgetattr(self.slave)
        self.output = bytearray()
        def session():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        env = dict(os.environ, TERM="xterm-256color", HOME=str(root), XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"), VOYAGE_INSTALLER_BIN=str(BIN))
        command = ["/bin/sh", "-c", "cat install.sh | sh"] if piped else [str(BIN)]
        self.process = subprocess.Popen(command, cwd=ROOT, env=env, stdin=self.slave, stdout=self.slave, stderr=self.slave, preexec_fn=session)

    def pump(self):
        if select.select([self.master], [], [], .04)[0]:
            self.output.extend(os.read(self.master, 65536))

    def screen(self):
        return rendered_screen(bytes(self.output), self.rows, self.columns)

    def wait(self, expected):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            self.pump()
            if expected in self.screen():
                return
        raise AssertionError(f"Missing {expected!r}:\n{self.screen()}")

    def send(self, keys):
        os.write(self.master, keys.encode())

    def choose(self, index, next_title):
        self.send("\x1b[B" * index + "\r")
        self.wait(next_title)

    def resize(self, rows, columns):
        self.rows, self.columns = rows, columns
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        os.killpg(self.process.pid, signal.SIGWINCH)

    def finish(self, expected=0):
        deadline = time.monotonic() + 5
        while self.process.poll() is None and time.monotonic() < deadline:
            self.pump()
        assert self.process.poll() == expected, (self.process.poll(), self.screen())
        self.pump()
        assert termios.tcgetattr(self.slave) == self.before, "terminal attributes not restored"
        assert b"\x1b[?1049l" in self.output, "alternate screen not restored"
        assert b"\x1b[?2004l" in self.output, "paste mode not restored"

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait()
        os.close(self.master)
        os.close(self.slave)


def case(name, body, piped=False):
    with tempfile.TemporaryDirectory(prefix="voyage-installer-tui-") as raw:
        root = Path(raw)
        sentinel = root / "existing-config"
        sentinel.write_text("preserve me")
        ui = Terminal(root, piped)
        try:
            ui.wait("What will this machine do?")
            body(ui)
            assert list(root.iterdir()) == [sentinel], list(root.iterdir())
            assert sentinel.read_text() == "preserve me"
        finally:
            ui.close()
    print(f"installer TUI {name}: passed")


def local(ui):
    ui.choose(0, "How should Helm access a model?")
    ui.choose(0, "Review your setup")
    ui.wait("Roles: Local work")
    ui.choose(0, "Simulation complete")
    ui.wait("Would install Helm.")
    ui.send("\r")
    ui.finish()


def back_and_unicode(ui):
    ui.choose(0, "How should Helm access a model?")
    ui.choose(2, "Choose an API provider")
    ui.choose(0, "Which model would you use?")
    ui.send("\r")
    ui.wait("Enter a value")
    ui.send("\x1b[200~model-雪\x1b[201~\r")
    ui.wait("Review your setup")
    ui.send("\x1b")
    ui.wait("Which model would you use?")
    ui.wait("model-雪")
    ui.send("\x1b[200~\x1b[2Jbad\x1b[201~")
    ui.wait("Paste rejected")
    ui.send("\r")
    ui.wait("Review your setup")
    ui.choose(1, "What will this machine do?")
    ui.choose(1, "Do you already have a Vessel?")
    ui.choose(3, "How will you control other Helms?")
    ui.choose(0, "Review your setup")
    assert "model-雪" not in ui.screen()
    ui.choose(0, "Simulation complete")
    ui.wait("DEFERRED:")
    ui.send("\r")
    ui.finish()


def vessel(ui):
    ui.choose(3, "Who needs to reach Vessel?")
    ui.choose(2, "How should machines reach Vessel?")
    ui.choose(2, "Choose a tunnel approach")
    ui.choose(0, "Name the first Vessel administrator")
    ui.choose(0, "When should Vessel run?")
    ui.choose(1, "Review your setup")
    ui.choose(0, "Simulation complete")
    ui.wait("temporary Cloudflare tunnel")
    ui.send("\r")
    ui.finish()


def resize(ui):
    ui.resize(10, 30)
    ui.wait("Resize to at least")
    ui.send("\r\r\r\r")
    for _ in range(5): ui.pump()
    ui.resize(20, 48)
    ui.wait("What will this machine do?")
    ui.choose(0, "How should Helm access a model?")
    ui.choose(0, "Review your setup")
    ui.wait("Roles: Local work")
    ui.send("\x03")
    ui.finish(130)


def interrupted(ui, sig):
    os.killpg(ui.process.pid, sig)
    ui.finish(130)


def main():
    case("piped local preview", local, piped=True)
    case("backtracking, Unicode, role changes", back_and_unicode)
    case("Vessel and temporary tunnel", vessel)
    case("resize blocks hidden navigation", resize)
    case("SIGTERM restoration", lambda ui: interrupted(ui, signal.SIGTERM))
    case("SIGHUP restoration", lambda ui: interrupted(ui, signal.SIGHUP))
    result = subprocess.run([str(BIN)], stdin=subprocess.DEVNULL, capture_output=True, timeout=3)
    assert result.returncode == 1 and b"interactive terminal" in result.stderr
    for option in ("--help", "--version"):
        result = subprocess.run([str(BIN), option], capture_output=True, timeout=3)
        assert result.returncode == 0 and b"voyage-installer" in result.stdout
    result = subprocess.run([str(BIN), "--install-for-real"], capture_output=True, timeout=3)
    assert result.returncode == 1
    print("installer CLI noninteractive/help/version/unknown argument: passed")


if __name__ == "__main__": main()
