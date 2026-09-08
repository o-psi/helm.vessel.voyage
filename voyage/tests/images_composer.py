"""Offline Helm image composer regression, using real controlling Linux PTYs.

Run against already-built main binaries (no builds, desktop capture or live API):
  python3 voyage/tests/images_composer.py --bin-dir /home/psi/voyage/target/debug \
      --workflow /home/psi/voyage/voyage/tests/images_workflow.py
Evidence is retained in a private temporary directory, including raw PTY traces.
"""
import argparse
import base64
import errno
import fcntl
import http.server
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import urllib.request


def launch_pty(argv, env, cwd, trace):
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 35, 100, 0, 0))
    # Start a fresh single-threaded helper before acquiring the controlling TTY;
    # never run Python after forking this multi-threaded fixture process.
    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '_pty_exec', *argv],
        env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    os.close(slave)
    state = {'pid': process.pid, 'process': process, 'fd': master, 'output': bytearray(), 'status': None}
    def drain():
        with trace.open('wb') as output:
            while True:
                try:
                    if not select.select([master], [], [], .1)[0]:
                        continue
                    data = os.read(master, 65536)
                    if not data:
                        break
                    state['output'].extend(data)
                    output.write(data)
                    output.flush()
                except OSError as error:
                    if error.errno in (errno.EIO, errno.EBADF):
                        break
                    raise
    state['reader'] = threading.Thread(target=drain, daemon=True)
    state['reader'].start()
    return state


def send(pty, data):
    os.write(pty['fd'], data.encode() if isinstance(data, str) else data)


def paste(pty, text):
    send(pty, '\x1b[200~' + text + '\x1b[201~')


def rendered(pty, start=0):
    # Reconstruct cursor-addressed ratatui output. Stripping ANSI alone loses
    # unchanged cells (including spaces and letters) in differential frames.
    text = bytes(pty['output']).decode('utf-8', errors='replace')
    screen = [[' '] * 100 for _ in range(35)]
    row = col = 0
    for token in re.finditer(r'\x1b\[[0-?]*[ -/]*[@-~]|[^\x1b]', text):
        value = token.group()
        if value.startswith('\x1b['):
            command = value[-1]
            params = value[2:-1]
            if params.startswith('?'):
                continue
            nums = [int(x or 0) for x in params.split(';')] if params else [0]
            n = nums[0] or 1
            if command in ('H', 'f'):
                row, col = n - 1, (nums[1] or 1) - 1 if len(nums) > 1 else 0
            elif command == 'J' and nums[0] in (2, 3):
                screen = [[' '] * 100 for _ in range(35)]
            elif command == 'K' and 0 <= row < 35:
                lo, hi = (0, 100) if nums[0] == 2 else ((0, col + 1) if nums[0] == 1 else (col, 100))
                screen[row][max(0, lo):min(100, hi)] = [' '] * max(0, min(100, hi) - max(0, lo))
            elif command == 'G':
                col = n - 1
            elif command == 'A':
                row = max(0, row - n)
            elif command == 'B':
                row += n
            elif command == 'C':
                col += n
            elif command == 'D':
                col = max(0, col - n)
        elif value == '\r':
            col = 0
        elif value == '\n':
            row += 1
        elif value >= ' ':
            if 0 <= row < 35 and 0 <= col < 100:
                screen[row][col] = value
            col += 1
    return '\n'.join(''.join(line) for line in screen)


def stop_pty(pty, wait_for):
    if pty['status'] is not None:
        return
    def reaped():
        pid, status = os.waitpid(pty['pid'], os.WNOHANG)
        if pid:
            pty['status'] = status
            pty['process'].returncode = os.waitstatus_to_exitcode(status)
            return True
        return False
    try:
        send(pty, b'\x11')  # Ctrl+Q: detach without cancelling a voyage.
        wait_for(reaped, timeout=5)
    except (AssertionError, OSError):
        os.kill(pty['pid'], signal.SIGTERM)
        wait_for(reaped, timeout=5)
    finally:
        os.close(pty['fd'])
        pty['reader'].join(timeout=3)
    assert not pty['reader'].is_alive(), 'PTY output drainer did not finish'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--workflow', type=Path, default=Path(__file__).with_name('images_workflow.py'))
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location('images_workflow', args.workflow.resolve())
    workflow = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(workflow)
    wait_for = workflow.wait_for
    binary = args.bin_dir.resolve()
    for name in ('helm', 'vessel', 'voyage'):
        assert (binary / name).is_file(), binary / name
    root = Path(tempfile.mkdtemp(prefix='hic-'))
    print('evidence:', root, flush=True)
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color'}
    for key in ('HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME'):
        path = root / {'HOME': 'h', 'XDG_DATA_HOME': 'd', 'XDG_STATE_HOME': 's',
                       'XDG_CONFIG_HOME': 'c', 'XDG_CACHE_HOME': 'cache'}[key]
        path.mkdir(mode=0o700)
        env[key] = str(path)
    # Deliberately do not inherit DISPLAY, WAYLAND_DISPLAY, credentials, or config.
    workspace = root / 'workspace'
    workspace.mkdir()
    directory = Path(env['XDG_STATE_HOME']) / 'voyage' / 'vessel'
    pixels = workflow.png()
    image = base64.b64encode(pixels).decode()
    image_path = root / 'pixels.png'
    image_path.write_bytes(pixels)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), workflow.Provider)
    server.requests, server.fail, server.image = [], False, image
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / 'config.toml'
    config.write_text('provider = "openai-chat"\nmodel = "gpt-4o"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\naccess = "read-only"\ncontext_window = 0\n')
    config.chmod(0o600)
    log = (root / 'vessel.log').open('wb')
    supervisor = None
    ptys = []
    sessions = []
    checks = []

    def request(command):
        credential = json.loads((directory / 'process-http.json').read_text())
        req = urllib.request.Request(credential['endpoint'] + '/v1/vessel/command',
            data=json.dumps({'protocol': 1, 'command': command}).encode(),
            headers={'Authorization': 'Bearer ' + credential['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=10) as response:
            value = json.load(response)
        assert value.get('error') is None, value
        return value['result']

    def snapshot(sid):
        value = request({'op': 'snapshot', 'session_id': sid})
        assert value.get('error') is None, value
        return value.get('result', value)

    def finished(sid):
        def observe():
            value = snapshot(sid)
            return value if value.get('run', {}).get('state') == 'completed' and value.get('pending_cleanup_run') is None else None
        value = wait_for(observe)
        wait_for(lambda: request({'op': 'inspect', 'session_id': sid})['state'] == 'suspended')
        return value

    def launch():
        pty = launch_pty([str(binary / 'helm'), '--config', str(config), 'chat'], env, workspace,
                        root / f'helm-{len(ptys)}.pty')
        ptys.append(pty)
        wait_for(lambda: 'Ctrl+N' in rendered(pty))
        return pty

    def expect(pty, text, start=0):
        try:
            wait_for(lambda: len(pty['output']) > start and text in rendered(pty), timeout=15)
        except AssertionError:
            raise AssertionError(f'PTY did not render {text!r}; tail: {rendered(pty, start)[-3500:]}') from None

    def action(pty, keys, text):
        start = len(pty['output'])
        send(pty, keys)
        expect(pty, text, start)

    def modal_command(pty, text, expected):
        start = len(pty['output'])
        paste(pty, text)
        send(pty, '\r')
        expect(pty, expected, start)

    def drafts():
        return [(path, json.loads(path.read_text())) for path in root.rglob('helm-new-drafts/*.json')]

    def draft_with(text, count):
        return next(((p, d) for p, d in drafts() if d['text'] == text and len(d.get('images', [])) == count), None)

    def assert_payload(index, text):
        wait_for(lambda: len(server.requests) > index)
        content = server.requests[index]['messages'][-1]['content']
        assert [part['type'] for part in content] == (['text', 'image_url'] if text else ['image_url']), content
        if text:
            assert content[0]['text'] == text, content
        assert content[-1]['image_url']['url'] == 'data:image/png;base64,' + image

    def owned_pids():
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    argv = (entry / 'cmdline').read_bytes().split(b'\0')
                    if argv[:3] == [os.fsencode(binary / 'voyage'), b'serve', b'--directory'] and Path(os.fsdecode(argv[3])).parent == directory / 'sessions':
                        found.append(int(entry.name))
                except (OSError, IndexError):
                    pass
        return found

    try:
        supervisor = subprocess.Popen([str(binary / 'vessel'), 'local-serve', '--directory', str(directory),
            '--voyage-binary', str(binary / 'voyage')], env=env, cwd=workspace,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / 'process-http.json').exists())
        pty = launch()
        expect(pty, 'Draft')
        text = 'Preserve this synthetic image draft'
        paste(pty, text)
        wait_for(lambda: draft_with(text, 0))
        action(pty, '\x1b[17~', 'Images · local files')  # F6
        modal_command(pty, str(image_path), '2×2')
        output = rendered(pty)
        assert all(token in output for token in ('pixels.png', 'image/png', f'{len(pixels)} bytes'))
        wait_for(lambda: draft_with(text, 1))
        modal_command(pty, 'remove 1', 'No images attached.')
        wait_for(lambda: draft_with(text, 0))
        modal_command(pty, str(image_path), '2×2')
        modal_command(pty, 'screenshot', 'Type CAPTURE + Enter. Esc cancels.')
        action(pty, '\x1b', 'Screenshot cancelled')
        action(pty, '\x1b', 'Back to composer')
        path, saved = wait_for(lambda: draft_with(text, 1))
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert stat.S_IMODE(path.parent.stat().st_mode) == 0o700
        assert saved['images'][0]['data_base64'] == image
        assert not server.requests, 'editing/cancel sent a request'
        checks.append('F6 add/remove, metadata, screenshot cancellation preserve unsent text/image')
        stop_pty(pty, wait_for)
        assert os.waitstatus_to_exitcode(pty['status']) == 0, pty['status']
        # Recovery must use private stored bytes, not re-read a path after restart.
        image_path.unlink()
        pty = launch()
        expect(pty, text)
        action(pty, '\x1b[17~', '2×2')
        action(pty, '\x1b', 'Back to composer')
        sid = saved['id']
        sessions.append(sid)
        send(pty, '\r')
        assert_payload(0, text)
        value = finished(sid)
        assert image not in json.dumps(value)
        wait_for(lambda: json.loads(path.read_text())['finished'])
        checks.append('private 0600 draft/0700 directory, restart recovery without source file, text+image first send')
        expect(pty, 'Image received.')
        image_path.write_bytes(pixels)
        followup = 'Existing session synthetic followup'
        paste(pty, followup)
        action(pty, '\x1b[17~', 'Images · local files')
        modal_command(pty, str(image_path), '2×2')
        action(pty, '\x1b', 'Back to composer')
        send(pty, '\r')
        assert_payload(1, followup)
        finished(sid)
        assert sum(isinstance(m.get('content'), list) for m in server.requests[1]['messages']) >= 2
        checks.append('existing-session text+image send and retained first-turn image')
        action(pty, '\x0e', 'Draft')
        action(pty, '\x1b[17~', 'Images · local files')
        modal_command(pty, str(image_path), '2×2')
        path, saved = wait_for(lambda: draft_with('', 1))
        sessions.append(saved['id'])
        action(pty, '\x1b', 'Back to composer')
        send(pty, '\r')
        assert_payload(2, '')
        finished(saved['id'])
        wait_for(lambda: json.loads(path.read_text())['finished'])
        assert len(server.requests) == 3, 'duplicate or unexpected provider dispatch'
        checks.append('image-only new-voyage first send; exactly three dispatches')
        for terminal in ptys:
            assert image.encode() not in terminal['output'], 'base64 leaked into TUI'
        log.flush()
        assert image not in (root / 'vessel.log').read_text(errors='replace')
        checks.append('no base64 in TUI, public snapshot or supervisor log')
    finally:
        for terminal in ptys:
            stop_pty(terminal, wait_for)
        for pid in owned_pids():
            try:
                fd = os.pidfd_open(pid)
                try:
                    if pid in owned_pids():
                        signal.pidfd_send_signal(fd, signal.SIGTERM)
                finally:
                    os.close(fd)
            except ProcessLookupError:
                pass
        wait_for(lambda: not owned_pids())
        if supervisor is not None:
            supervisor.terminate()
            supervisor.wait(timeout=10)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        log.close()
        assert not thread.is_alive()
        print('cleanup: Helm children reaped; owned voyage processes absent; Vessel reaped; provider stopped', flush=True)
    assert all(os.waitstatus_to_exitcode(terminal['status']) == 0 for terminal in ptys), 'Helm did not detach cleanly'
    result = {'status': 'passed', 'checks': checks, 'sessions': sessions,
              'requests': len(server.requests), 'cleanup': 'observed',
              'not_covered': ['uncertain first-send recovery (no deterministic transport fault injection)']}
    (root / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
    print('PASS: ' + '; '.join(checks), flush=True)


if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '_pty_exec':
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        os.execve(sys.argv[2], sys.argv[2:], os.environ)
    main()
