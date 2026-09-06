#!/usr/bin/env python3
"""Offline chat restart, precedence and frontend preference regression."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get("HELM_BIN", ROOT / "target/release/helm")).resolve()


def main():
    with tempfile.TemporaryDirectory(prefix="helm-chat-preferences-") as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root), XDG_DATA_HOME=str(root / "data"),
                   XDG_CONFIG_HOME=str(root / "config"), TERM="dumb")
        preferences = root / "data/helm/chat-preferences.json"
        config = root / "settings.toml"
        config.write_text('provider = "openai-chat"\nmodel = "initial"\napi_key_required = false\nbase_url = "http://127.0.0.1:9/v1"\naccess = "read-only"\nmax_tokens = 1234\ntemperature = 0.25\n')

        def invoke(*args, input="/exit\n", ok=True):
            result = subprocess.run([str(HELM), *args], input=input, text=True,
                                    capture_output=True, env=env, cwd=root, timeout=15)
            assert (result.returncode == 0) == ok, (args, result.stdout, result.stderr)
            return result

        invoke("--config", str(config), "--workspace", str(root), "--verbose",
               "--log-format", "json", "chat", "--plain", input="/model chosen\n/new\n/exit\n")
        first = json.loads(preferences.read_text())
        assert first["config"]["model"] == "chosen"
        assert first["config"]["max_tokens"] == 1234
        assert first["config"]["workspace"] == str(root)
        assert first["state"]["presentation"]["verbose"] is True
        assert first["state"]["presentation"]["plain"] is True
        assert "messages" not in first and "policy_profile" not in first["config"]
        result = invoke("chat", input="/access\n/model newer\n/exit\n")
        assert "read-only" in result.stdout
        assert json.loads(preferences.read_text())["config"]["model"] == "newer"
        invoke("--set", "max_tokens=4321", "--access", "approval", "--verbose=false",
               "--log-format", "text", "chat", "--plain=false")
        saved = json.loads(preferences.read_text())
        assert saved["config"]["max_tokens"] == 4321
        assert saved["config"]["model"] == "newer"
        assert saved["config"]["access"] == "approval"
        assert saved["state"]["presentation"] == {
            "verbose": False, "plain": False, "log_format": "text", "activity": False, "tool_details": False}
        # Non-chat utilities neither consume nor overwrite chat defaults.
        before = preferences.read_bytes()
        invoke("--config", str(config), "config", input="")
        assert preferences.read_bytes() == before
        # Corruption is concealed; explicit config provides a replacement path.
        preferences.write_text('{"secret":"do-not-print-preference-content"')
        failure = invoke("chat", ok=False)
        assert "chat preferences" in failure.stderr
        assert "do-not-print-preference-content" not in failure.stderr
        invoke("--config", str(config), "chat")
        assert json.loads(preferences.read_text())["config"]["model"] == "initial"
        if os.name == "posix":
            assert preferences.stat().st_mode & 0o077 == 0
        if os.name == "posix":
            import fcntl
            import pty
            import struct
            import termios
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
            process = subprocess.Popen([str(HELM), "chat", "--plain=false"], stdin=slave,
                                       stdout=slave, stderr=slave, cwd=root,
                                       env=dict(env, TERM="xterm-256color"), start_new_session=True)
            os.close(slave)
            os.set_blocking(master, False)

            def wait_for(predicate):
                deadline = time.monotonic() + 12
                while time.monotonic() < deadline:
                    try:
                        os.read(master, 65536)
                    except (BlockingIOError, OSError):
                        pass
                    assert process.poll() is None, "TUI exited before settings were saved"
                    if predicate(json.loads(preferences.read_text())):
                        return
                    time.sleep(0.03)
                raise AssertionError("TUI settings did not persist")

            try:
                wait_for(lambda value: not value["state"]["presentation"]["plain"])
                os.write(master, b"\x0c\x0f")
                wait_for(lambda value: value["state"]["presentation"]["activity"] and value["state"]["presentation"]["tool_details"])
                os.write(master, b"/set max_tokens 2222\r")
                wait_for(lambda value: value["config"]["max_tokens"] == 2222)
                selected = json.loads(preferences.read_text())["state"]["presentation"]
                assert selected["activity"] and selected["tool_details"], selected
                os.write(master, b"/model tui-chosen\r")
                wait_for(lambda value: value["config"]["model"] == "tui-chosen")
                os.write(master, b"\x11")
                deadline = time.monotonic() + 10
                while process.poll() is None and time.monotonic() < deadline:
                    try:
                        os.read(master, 65536)
                    except (BlockingIOError, OSError):
                        pass
                    time.sleep(0.03)
                assert process.wait(timeout=1) == 0
                # A fresh plain process keeps TUI display choices and the selected model.
                invoke("chat", "--plain")
                saved = json.loads(preferences.read_text())
                assert saved["config"]["model"] == "tui-chosen"
                assert saved["state"]["presentation"]["activity"]
                assert saved["state"]["presentation"]["tool_details"]
            finally:
                if process.poll() is None:
                    import signal
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                os.close(master)
        print("chat preferences restart, override, privacy and recovery checks passed")


if __name__ == "__main__":
    main()
