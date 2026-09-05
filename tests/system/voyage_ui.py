#!/usr/bin/env python3
"""Offline PTY coverage for recent conversations and saved voyage scope drafts."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from policy_ceiling import rendered_screen

HELM = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()
TOKEN = 'voyage-ui-operator-secret-canary-0123456789'
REMOTE = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
OWNER = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'


class Server(BaseHTTPRequestHandler):
    discovery = 0
    model_posts = 0
    failures = []

    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path == '/v1/diagnostics':
            if self.headers.get('Authorization') != 'Bearer ' + TOKEN:
                self.failures.append('missing operator authorization')
                self.send_error(401)
                return
            type(self).discovery += 1
            body = {'connections': [{'machine_id': REMOTE, 'owner_id': OWNER, 'epoch': 1}]}
        else:
            body = {'data': [{'id': 'voyage-ui-fixture'}]}
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        type(self).model_posts += 1
        self.send_error(500, 'No execution is authorized by this fixture')


class Terminal:
    def __init__(self, root, port, resume=None):
        self.root = root
        self.rows, self.columns = 32, 120
        self.output = bytearray()
        self.reaped = False
        config = root / 'fixture.toml'
        config.write_text(f'provider="openai-chat"\nmodel="voyage-ui-fixture"\nbase_url="http://127.0.0.1:{port}/v1"\napi_key_env="VOYAGE_FIXTURE_KEY"\naccess="read-only"\nprovider_retry_attempts=1\n')
        env = dict(os.environ, TERM='xterm-256color', HOME=str(root / 'home'),
                   XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'),
                   VOYAGE_FIXTURE_KEY='synthetic-provider-key')
        args = [str(HELM), '--config', str(config), '--workspace', str(root), 'chat']
        if resume:
            args += ['--resume', resume]
        self.pid, self.master = pty.fork()
        if self.pid == 0:
            os.execve(str(HELM), args, env)
        self.resize(32, 120, 1200, 640)

    def resize(self, rows, columns, width, height):
        self.rows, self.columns = rows, columns
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, width, height))

    def drain(self):
        if select.select([self.master], [], [], .05)[0]:
            try:
                self.output.extend(os.read(self.master, 65536))
            except OSError:
                pass

    def screen(self):
        return rendered_screen(bytes(self.output), self.rows, self.columns)

    def wait(self, predicate, label, timeout=15):
        deadline = time.monotonic() + timeout
        while not predicate():
            assert time.monotonic() < deadline, (label, self.screen())
            self.drain()

    def text(self, value):
        self.wait(lambda: value in self.screen(), value)

    def send(self, value):
        os.write(self.master, value if isinstance(value, bytes) else value.encode())

    def sessions(self):
        return [json.loads(p.read_text()) for p in (self.root / 'data/helm/sessions').glob('*.json')]

    def drafts(self):
        result = []
        for path in (self.root / 'data').rglob('*.json'):
            try:
                record = json.loads(path.read_text())
            except (ValueError, OSError):
                continue
            if isinstance(record, dict) and 'schema' in record and 'value' in record:
                record = record['value']
            if isinstance(record, dict) and isinstance(record.get('draft'), dict) and 'participants' in record['draft']:
                result.append(record)
        return result

    def finish(self):
        self.send(b'\x11')
        def exited():
            done, status = os.waitpid(self.pid, os.WNOHANG)
            if done:
                self.reaped = True
                assert os.waitstatus_to_exitcode(status) == 0, (status, self.screen())
                return True
            return False
        self.wait(exited, 'clean exit', 8)

    def close(self):
        if not self.reaped:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
        os.close(self.master)


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-voyage-ui-') as directory:
            root = Path(directory)
            terminal = Terminal(root, server.server_port)
            try:
                terminal.text('Recent · Ctrl+S')
                terminal.send('/name First conversation\r')
                terminal.text('Session renamed')
                terminal.send('unsent voyage draft 雪')
                terminal.text('unsent voyage draft 雪')
                terminal.resize(40, 80, 600, 1000)
                terminal.wait(lambda: 'Recent · Ctrl+S' not in terminal.screen(), 'portrait hides sidebar')
                terminal.send(b'\x13')
                terminal.text('Recent · Ctrl+S')
                terminal.send(b'\x1b')
                terminal.wait(lambda: 'Recent · Ctrl+S' not in terminal.screen(), 'portrait drawer closes')
                terminal.text('unsent voyage draft 雪')
                terminal.resize(32, 120, 1200, 640)
                terminal.text('Recent · Ctrl+S')
                terminal.send(b'\x0e')
                terminal.text('New session:')
                terminal.send('/name Second conversation\r')
                terminal.text('Session renamed')
                terminal.send(b'\x13\x1b[B\r')
                terminal.text('unsent voyage draft 雪')
                first = next(s for s in terminal.sessions() if s['name'] == 'First conversation')
                assert first['draft'] == 'unsent voyage draft 雪' and not first['messages']

                terminal.send(b'\x16')
                terminal.text('Voyages')
                terminal.send('n\r')
                terminal.text('Give this voyage a name')
                terminal.send('Fleet review\r')
                terminal.text('2/5')
                terminal.send('Inspect selected machines\r')
                terminal.text('3/5')
                terminal.send('\r')
                terminal.text('Select at least one Helm')
                terminal.send(' \r')
                terminal.text('4/5')
                terminal.send(' \r')
                terminal.text('5/5')
                terminal.text('This Helm')
                terminal.send('\r')
                terminal.text('Voyage draft saved')
                terminal.wait(lambda: len(terminal.drafts()) == 1, 'local draft persisted')
                local = terminal.drafts()[0]
                assert local['draft']['name'] == 'Fleet review'
                assert len(local['draft']['participants']) == 1
                assert local['draft']['coordinator'] in local['draft']['participants']
                terminal.send('v')
                terminal.text('Operator token:')
                terminal.send(f'http://127.0.0.1:{server.server_port}\t')
                terminal.send(b'\x1b[200~' + TOKEN.encode() + b'\x1b[201~')
                terminal.text('•')
                assert TOKEN.encode() not in terminal.output
                terminal.send('\r')
                terminal.text('1 remote Helms observed')
                assert Server.discovery == 1
                terminal.send('nRemote review\rCheck remote scope\r')
                terminal.text('3/5')
                terminal.send(b'\x1b[B \r')
                terminal.text('4/5')
                terminal.send(' \r')
                terminal.text('5/5')
                terminal.text(REMOTE)
                terminal.send('\r')
                terminal.text('Voyage draft saved')
                terminal.wait(lambda: len(terminal.drafts()) == 2, 'remote draft persisted')
                remote = next(d for d in terminal.drafts() if d['draft']['name'] == 'Remote review')
                assert remote['draft']['participants'] == [REMOTE]
                assert remote['draft']['coordinator'] == REMOTE
                terminal.send(b'\x1b')
                terminal.text('unsent voyage draft 雪')
                terminal.finish()
                assert TOKEN.encode() not in terminal.output
            finally:
                terminal.close()

            terminal = Terminal(root, server.server_port, first['id'])
            try:
                terminal.text('unsent voyage draft 雪')
                terminal.send(b'\x16')
                terminal.text('Fleet review')
                terminal.text('Remote review')
                terminal.send('\r')
                terminal.text('1/5')
                terminal.send('\r\r')
                terminal.text('3/5')
                terminal.text('This Helm')
                terminal.text('[x] Helm ' + REMOTE)
                terminal.send('\r')
                terminal.text('4/5')
                terminal.text('[x] Helm ' + REMOTE)
                terminal.send(b'\x1b\x1b')
                terminal.text('unsent voyage draft 雪')
                terminal.finish()
                assert TOKEN.encode() not in terminal.output
            finally:
                terminal.close()
            for path in (root / 'data').rglob('*'):
                if path.is_file():
                    assert TOKEN.encode() not in path.read_bytes(), path
            assert not Server.failures, Server.failures
            assert Server.model_posts == 0, 'scope setup must not execute model work'
            assert Server.discovery == 1, 'restart must not reuse persisted credentials'
            print('voyage UI: orientation, drawer, session drafts, validation, local/remote scope, restart, and secret isolation passed')
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
