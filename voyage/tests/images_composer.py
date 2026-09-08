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
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import urllib.request
import urllib.error


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
    process = pty['process']
    try:
        if process.poll() is None:
            send(pty, b'\x11')  # Ctrl+Q detaches without cancelling the voyage.
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        code = process.returncode
        pty['status'] = code << 8 if code >= 0 else -code
        os.close(pty['fd'])
        pty['reader'].join(timeout=3)
    assert not pty['reader'].is_alive(), 'PTY output drainer did not finish'


class AdmissionProxy(http.server.ThreadingHTTPServer):
    """Drop an admitted reply, never a request; retain a bounded command audit."""
    daemon_threads = False

    def __init__(self, discovery):
        self.discovery = discovery
        self.original = discovery.read_bytes()
        self.credential = json.loads(self.original)
        self.block_resolve = threading.Event()
        self.block_resolve.set()
        self.dropped = threading.Event()
        self.blocked = threading.Event()
        self.commands, self.errors, self.responses = [], [], []
        self.lock = threading.Lock()
        super().__init__(('127.0.0.1', 0), FaultHandler)
        self.thread = threading.Thread(target=self.serve_forever)
        self.thread.start()
        replacement = dict(self.credential, endpoint=f'http://127.0.0.1:{self.server_port}')
        self.replace(json.dumps(replacement).encode())

    def replace(self, data):
        temporary = self.discovery.with_suffix('.proxy-tmp')
        fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as output:
            output.write(data)
        temporary.replace(self.discovery)
        assert stat.S_IMODE(self.discovery.stat().st_mode) == 0o600

    def close(self):
        # Discovery must never point at a dead proxy, including assertion failures.
        self.replace(self.original)
        self.shutdown()
        self.server_close()
        self.thread.join(timeout=5)
        assert not self.thread.is_alive(), 'fault proxy did not stop'
        assert self.discovery.read_bytes() == self.original


class FaultHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, payload=b'{}'):
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        self.reply(503)  # SSE is deliberately unavailable; polling must suffice.

    def do_POST(self):
        proxy = self.server
        self.connection.settimeout(10)
        try:
            if self.path == '/v1/vessel/events':
                self.close_connection = True
                self.reply(503)
                return
            assert self.path == '/v1/vessel/command', self.path
            body = self.rfile.read(int(self.headers['Content-Length']))
            command = json.loads(body)['command']
            assert self.headers['Authorization'] == 'Bearer ' + proxy.credential['token']
            with proxy.lock:
                proxy.commands.append(command)
            op = command['op']
            if op == 'resolve' and proxy.block_resolve.is_set():
                proxy.blocked.set()
                self.reply(503)
                return
            req = urllib.request.Request(proxy.credential['endpoint'] + self.path,
                data=body, headers={'Authorization': self.headers['Authorization'],
                                    'Content-Type': self.headers['Content-Type']})
            try:
                response = urllib.request.urlopen(req, timeout=10)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                status, payload = response.status, response.read()
            with proxy.lock:
                proxy.responses.append((command, status, json.loads(payload)))
            if op == 'submit_content' and not proxy.dropped.is_set():
                proxy.dropped_command = command
                proxy.admission = json.loads(payload)
                proxy.dropped.set()
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
                return
            self.reply(status, payload)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Helm may detach while a bounded upstream request completes.
        except Exception as error:
            proxy.errors.append(repr(error))
            self.close_connection = True


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
    # Never inherit real desktop addresses, credentials, or configuration.
    workspace = root / 'workspace'
    workspace.mkdir()
    directory = Path(env['XDG_STATE_HOME']) / 'voyage' / 'vessel'
    pixels = workflow.png()
    image = base64.b64encode(pixels).decode()
    image_path = root / 'synthetic pixels.png'
    image_path.write_bytes(pixels)
    # Only these private executables can be discovered as clipboard helpers.
    fixture_path = root / 'fixture-bin'
    fixture_path.mkdir(mode=0o700)
    state_path, calls = root / 'clipboard.json', root / 'clipboard.calls'
    def clipboard(mode='image', delay=0, text=''):
        temporary = state_path.with_suffix('.tmp')
        temporary.write_text(json.dumps(dict(mode=mode, delay=delay, text=text, image=image)))
        temporary.replace(state_path)
    clipboard()
    helper = f"""#!{sys.executable}
import base64, json, os, sys, time
from pathlib import Path
state = json.loads(Path({str(state_path)!r}).read_text())
args = sys.argv[1:]
with open({str(calls)!r}, 'a') as log:
    log.write(json.dumps([os.getpid(), Path(sys.argv[0]).name, args]) + '\\n')
name = Path(sys.argv[0]).name
listing = args == (['--list-types'] if name == 'wl-paste' else ['-selection', 'clipboard', '-out', '-target', 'TARGETS'])
mime = 'text/plain;charset=utf-8' if state['mode'] == 'text' else 'image/png'
if not listing:
    expected = ['--no-newline', '--type', mime] if name == 'wl-paste' else ['-selection', 'clipboard', '-out', '-target', mime]
    if args != expected:
        sys.exit(91)
if state['mode'] == 'error':
    sys.stderr.write('synthetic clipboard unavailable; install clipboard helper or paste an image path')
    sys.exit(1)
if listing:
    print(mime)
else:
    time.sleep(60 if state['mode'] == 'timeout' else state['delay'])
    data = state['text'].encode() if state['mode'] == 'text' else base64.b64decode(state['image'])
    if state['mode'] == 'oversize':
        data += b'x' * (25 * 1024 * 1024)
    sys.stdout.buffer.write(data)
"""
    for name in ('wl-paste', 'xclip'):
        executable = fixture_path / name
        executable.write_text(helper)
        executable.chmod(0o700)
    env['PATH'] = str(fixture_path)  # No accidental desktop helper fallback.
    env['WAYLAND_DISPLAY'] = 'synthetic-only'
    env['DISPLAY'] = ':9876'
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
    failures = []
    proxy = None

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

    def drafts():
        return [(path, json.loads(path.read_text())) for path in root.rglob('helm-new-drafts/*.json')]

    def draft_with(text, count):
        return next(((p, d) for p, d in drafts() if d['text'] == text and len(d.get('images', [])) == count), None)

    def assert_payload(index, *parts):
        wait_for(lambda: len(server.requests) > index)
        content = server.requests[index]['messages'][-1]['content']
        expected = [{'type': 'image_url', 'image_url': {'url': 'data:image/png;base64,' + image, 'detail': 'auto'}}
                    if part is None else {'type': 'text', 'text': part} for part in parts]
        if isinstance(content, str):
            assert parts == (content,), (parts, content)
        else:
            assert content == expected, (expected, content)

    def helper_pids():
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    args = (entry / 'cmdline').read_bytes().split(b'\0')
                    if any(arg in (os.fsencode(fixture_path / 'wl-paste'),
                                   os.fsencode(fixture_path / 'xclip')) for arg in args):
                        found.append(int(entry.name))
                except OSError:
                    pass
        return found

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
        # Normal bracketed paste must not invoke even a broken clipboard helper.
        clipboard(mode='error')
        text = 'beforeafter'
        paste(pty, text)
        wait_for(lambda: draft_with(text, 0))
        assert not calls.exists(), 'ordinary terminal paste read the clipboard'
        send(pty, '\x1b[17~')  # F6 has no image dialog or capture behavior.
        time.sleep(.2)
        assert 'Images · local files' not in rendered(pty)
        assert draft_with(text, 0)
        checks.append('ordinary bracketed text never reads clipboard; F6 stays in composer')

        # Insert at the caret, not at the end; late acquisition stays at its anchor.
        send(pty, '\x1b[D' * 5)
        clipboard(delay=1)
        send(pty, '\x16')
        wait_for(lambda: calls.exists() and 'image/png' in calls.read_text())
        send(pty, '\r')
        time.sleep(.1)
        assert not server.requests, 'Enter dispatched before asynchronous paste completed'
        send(pty, 'typed')
        path, saved = wait_for(lambda: draft_with('before[Image 1]typedafter', 1))
        expect(pty, 'image/png')
        expect(pty, f'{len(pixels)} bytes')
        expect(pty, 'clipboard')
        checks.append('Ctrl+V async acquisition, ordinary metadata rows, anchored typing')

        # Left/right cross the whole marker; Backspace/Delete remove its bytes too.
        send(pty, '\x1b[D' * 5 + '\x7f')
        wait_for(lambda: draft_with('beforetypedafter', 0))
        clipboard()
        (fixture_path / 'wl-paste').rename(fixture_path / 'wl-paste.disabled')
        send(pty, '\x1bv')  # Alt+V, forcing native X11 fallback
        wait_for(lambda: draft_with('before[Image 1]typedafter', 1))
        assert 'xclip' in calls.read_text(), 'X11 fallback was not exercised'
        (fixture_path / 'wl-paste.disabled').rename(fixture_path / 'wl-paste')
        send(pty, '\x1b[D\x1b[C\x7f')
        wait_for(lambda: draft_with('beforetypedafter', 0))
        send(pty, '\x1b[2;2~')  # Shift+Insert with explicit Shift modifier
        wait_for(lambda: draft_with('before[Image 1]typedafter', 1))
        send(pty, '\x1b[D\x1b[3~')
        wait_for(lambda: draft_with('beforetypedafter', 0))
        assert all(not d.get('images') for _, d in drafts())
        checks.append('Alt+V/Shift+Insert; atomic left/right/backspace/delete and stored-byte removal')

        # Exact quoted paths and file URIs bypass clipboard, even when it errors.
        clipboard(mode='error')
        before_calls = calls.read_text()
        paste(pty, '"' + str(image_path) + '"')
        wait_for(lambda: draft_with('before[Image 1]typedafter', 1))
        send(pty, '\x7f')
        wait_for(lambda: draft_with('beforetypedafter', 0))
        paste(pty, image_path.as_uri())
        path, saved = wait_for(lambda: draft_with('before[Image 1]typedafter', 1))
        assert calls.read_text() == before_calls
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert stat.S_IMODE(path.parent.stat().st_mode) == 0o700
        assert saved['images'][0]['data_base64'] == image
        assert not server.requests
        stop_pty(pty, wait_for)
        image_path.unlink()
        pty = launch()
        expect(pty, '[Image 1]')
        sid = saved['id']
        sessions.append(sid)
        send(pty, '\r')
        assert_payload(0, 'before', None, 'typedafter')
        value = finished(sid)
        assert image not in json.dumps(value)
        wait_for(lambda: json.loads(path.read_text())['finished'])
        checks.append('quoted path/file URI; private draft restart after source deletion; exact ordered first send')
        expect(pty, 'Image received.')
        clipboard()
        paste(pty, 'existing-before')
        send(pty, '\x16')
        expect(pty, '[Image 1]')
        paste(pty, 'existing-after')
        send(pty, '\r')
        assert_payload(1, 'existing-before', None, 'existing-after')
        finished(sid)
        assert sum(isinstance(m.get('content'), list) for m in server.requests[1]['messages']) >= 2
        checks.append('existing-voyage ordered text/image/text and retained first image')
        action(pty, '\x0e', 'Draft')
        paste(pty, '')  # Empty bracketed paste is the native-read fallback.
        path, saved = wait_for(lambda: draft_with('[Image 1]', 1))
        sessions.append(saved['id'])
        send(pty, '\r')
        assert_payload(2, None)
        finished(saved['id'])
        wait_for(lambda: json.loads(path.read_text())['finished'])
        checks.append('empty bracketed paste fallback; image-only new voyage')

        action(pty, '\x0e', 'Draft')
        paste(pty, 'kept')
        wait_for(lambda: draft_with('kept', 0))
        clipboard(mode='text', text=' clipboard text ')
        send(pty, '\x16')
        wait_for(lambda: draft_with('kept clipboard text ', 0))
        kept = 'kept clipboard text '
        checks.append('native text/plain;charset=utf-8 fallback preserves exact whitespace')
        for mode in ('missing', 'error', 'oversize', 'timeout'):
            clipboard(mode=mode)
            if mode == 'missing':
                for name in ('wl-paste', 'xclip'):
                    (fixture_path / name).rename(fixture_path / (name + '.disabled'))
            start = len(pty['output'])
            send(pty, '\x16')
            expect(pty, 'Paste failed:', start)
            wait_for(lambda: not helper_pids())
            assert draft_with(kept, 0), mode
            expected_error = {'missing': 'wl-paste', 'error': 'wl-paste',
                              'oversize': 'limit', 'timeout': 'timed out'}[mode]
            try:
                if mode in ('missing', 'error'):
                    wait_for(lambda: any(re.search(r'wl-paste|xclip|install|paste.*path', line, re.I)
                                         for line in rendered(pty).splitlines() if 'Paste failed:' in line), timeout=3)
                else:
                    expect(pty, expected_error)
            except AssertionError as error:
                failures.append(f'{mode}: {error}')
                print(f'FAIL: {mode} has no actionable diagnostic; continuing independent cases', flush=True)
            if mode == 'missing':
                for name in ('wl-paste', 'xclip'):
                    (fixture_path / (name + '.disabled')).rename(fixture_path / name)
            checks.append(mode + ' leaves draft intact without dispatch')
        clipboard(delay=5)
        before_calls = calls.read_text()
        send(pty, '\x16')
        wait_for(lambda: calls.read_text() != before_calls)
        send(pty, '\x1b')
        expect(pty, 'cancelled')
        assert draft_with(kept, 0)
        assert len(server.requests) == 3
        checks.append('Esc cancels acquisition without dispatch or draft loss')
        wait_for(lambda: not helper_pids())

        # A pre-marker saved draft must append a marker without changing IDs.
        image_path.write_bytes(pixels)
        paste(pty, image_path.as_uri())
        path, saved = wait_for(lambda: draft_with(kept + '[Image 1]', 1))
        stop_pty(pty, wait_for)
        legacy = dict(saved, text=kept)
        legacy.pop('markers', None)
        path.write_text(json.dumps(legacy))
        image_path.unlink()
        pty = launch()
        expect(pty, '[Image 1]')
        # Trigger persistence after loading; compare identity and payload fields.
        send(pty, 'x')
        _, migrated = wait_for(lambda: draft_with(kept + '[Image 1]x', 1))
        for key in ('id', 'turn', 'start', 'submit', 'attempted', 'start_attempted'):
            assert migrated[key] == saved[key], key
        assert migrated['images'] == saved['images']
        sessions.append(saved['id'])
        send(pty, '\r')
        assert_payload(3, kept, None, 'x')
        finished(saved['id'])
        assert len(server.requests) == 4
        checks.append('legacy text/images migration appends marker; exact payload and command IDs retained')
        # Actual uncertain FIRST image send: admission succeeds, HTTP reply is lost.
        stop_pty(pty, wait_for)
        proxy = AdmissionProxy(directory / 'process-http.json')
        pty = launch()
        action(pty, '\x0e', 'Draft')
        clipboard()
        paste(pty, 'uncertain-before')
        send(pty, '\x16')
        wait_for(lambda: draft_with('uncertain-before[Image 1]', 1))
        paste(pty, 'uncertain-after')
        text = 'uncertain-before[Image 1]uncertain-after'
        path, original = wait_for(lambda: draft_with(text, 1))
        sid = original['id']
        sessions.append(sid)
        view_identity = json.dumps([None, None, str(directory), sid], separators=(',', ':')).encode()
        view_path = directory.with_name('helm-views') / (hashlib.sha256(view_identity).hexdigest() + '.json')
        send(pty, '\r')
        assert proxy.dropped.wait(30), 'first SubmitContent did not reach real admission'
        assert proxy.admission['error'] is None, proxy.admission
        admission = proxy.admission['result']['result']
        assert admission['status'] == 'accepted' and admission['duplicate'] is False, admission
        assert admission['command_id'] == original['turn'], admission
        assert_payload(4, 'uncertain-before', None, 'uncertain-after')
        assert proxy.blocked.wait(20), 'Helm did not attempt uncertain receipt resolution'
        saved = json.loads(path.read_text())
        assert saved['attempted'] and not saved['finished'] and saved['receipt'] is None, saved.keys()
        assert saved['submit']['op'] == 'submit_content'
        assert proxy.dropped_command['command_id'] == saved['turn'] == original['turn']
        for key, value in saved['submit'].items():
            assert proxy.dropped_command[key] == value, key
        for key in ('id', 'turn', 'text', 'markers', 'images'):
            assert saved[key] == original[key], key
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert stat.S_IMODE(path.parent.stat().st_mode) == 0o700
        frozen = path.read_bytes()
        stop_pty(pty, wait_for)
        assert os.waitstatus_to_exitcode(pty['status']) == 0, 'uncertain Helm failed Ctrl+Q detach'
        assert path.read_bytes() == frozen, 'detach changed uncertain recovery record'
        assert not any(c['op'] == 'cancel' for c in proxy.commands), 'detach cancelled admitted run'
        finished(sid)
        proxy.block_resolve.clear()
        pty = launch()
        wait_for(lambda: json.loads(path.read_text())['finished'])
        recovered = json.loads(path.read_text())
        assert recovered['receipt']['command_id'] == saved['turn']
        assert recovered['receipt']['status'] == 'accepted', recovered['receipt']
        assert recovered['receipt']['run_id'] == admission['run_id'], recovered['receipt']
        for key in saved:
            if key not in ('receipt', 'finished'):
                assert recovered[key] == saved[key], 'recovery changed ' + key
        def cleared_view():
            if not view_path.exists():
                return False
            view = json.loads(view_path.read_text())
            return view['text'] == '' and not view.get('images') and not view.get('markers') and view['pending'] is None
        wait_for(cleared_view)
        expect(pty, 'Image received.')
        stop_pty(pty, wait_for)
        assert len(server.requests) == 5, 'uncertain turn was dispatched more than once'
        ops = [c['op'] for c in proxy.commands]
        assert ops.count('submit_content') == 1, ops
        assert ops.count('upload_image') == 1, ops
        assert 'cancel' not in ops, ops
        resolves = [c for c, status, response in proxy.responses if c['op'] == 'resolve' and status == 200]
        assert any(c['command_id'] == saved['turn'] and c.get('original') == saved['submit'] for c in resolves), resolves
        assert not proxy.errors, proxy.errors
        (root / 'fault-audit.json').write_text(json.dumps({
            'command_id': saved['turn'], 'ops': ops, 'admission': proxy.admission,
            'receipt': recovered['receipt']}, indent=2) + '\n')
        proxy.close()
        proxy = None
        checks.append('lost actual first-image admission reply; blocked Resolve; Ctrl+Q; restart receipt recovery; unchanged IDs/bytes; cleared view; no upload/submit replay')
        for terminal in ptys:
            assert image.encode() not in terminal['output'], 'base64 leaked into TUI'
        log.flush()
        assert image not in (root / 'vessel.log').read_text(errors='replace')
        checks.append('no base64 in TUI, public snapshot or supervisor log')
    finally:
        for terminal in ptys:
            stop_pty(terminal, wait_for)
        if proxy is not None:
            proxy.close()
        leaked_helpers = helper_pids()
        for pid in leaked_helpers:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
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
        assert not leaked_helpers, f'Clipboard helper cleanup leaked PIDs: {leaked_helpers}'
        print('cleanup: Helm children reaped; owned voyage processes absent; Vessel reaped; provider stopped', flush=True)
    assert all(os.waitstatus_to_exitcode(terminal['status']) == 0 for terminal in ptys), 'Helm did not detach cleanly'
    result = {'status': 'failed' if failures else 'passed', 'checks': checks, 'failures': failures, 'sessions': sessions,
              'requests': len(server.requests), 'cleanup': 'observed',
              'not_covered': []}
    (root / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
    assert not failures, '\n'.join(failures)
    print('PASS: ' + '; '.join(checks), flush=True)


if __name__ == '__main__':
    if len(sys.argv) > 2 and sys.argv[1] == '_pty_exec':
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        os.execve(sys.argv[2], sys.argv[2:], os.environ)
    main()
