#!/usr/bin/env python3
"""Saved workflows through actual TUI keys, native HTTP and canonical checkpoints."""
import fcntl
import hashlib
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

from saved_workflows import DOCUMENT
from policy_ceiling import rendered_screen

HELM = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()
LITERAL = '$(touch never-created) {{count}} 雪'


class Provider(BaseHTTPRequestHandler):
    requests = []
    failures = []
    case = None
    release = threading.Event()

    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({'data': [{'id': 'workflow-fixture'}]}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.requests.append(body)
        try:
            case = self.case
            assert body['model'] == 'workflow-fixture', body['model']
            assert self.headers['Authorization'] == 'Bearer synthetic-fixture-key'
            # At the actual provider boundary, canonical acceptance and attribution
            # must already be durable. This fixture never executes workflow prose.
            saved = case.saved()
            assert len(saved['workflow_runs']) == 1, saved
            invocation = saved['workflow_runs'][0]
            assert invocation == case.expected, invocation
            prompt = 'Review ' + json.dumps(LITERAL, ensure_ascii=False) + ' count=2'
            assert any(m['role'] == 'user' and m['content'] == prompt for m in saved['messages'])
            assert any(m['role'] == 'user' and m['content'] == prompt for m in body['messages'])
            if case.mode == 'cancel':
                delta = {'content': 'workflow-partial-before-cancel'}
            elif case.mode == 'denied' and not any(m['role'] == 'tool' for m in body['messages']):
                delta = {'tool_calls': [{'index': 0, 'id': 'denied-shell', 'type': 'function',
                         'function': {'name': 'shell', 'arguments': json.dumps({'command': 'touch must-not-exist'})}}]}
            else:
                if case.mode == 'denied':
                    result = next(m['content'] for m in body['messages'] if m['role'] == 'tool')
                    assert 'denied' in result.lower() or 'read-only' in result.lower(), result
                delta = {'content': 'tui-workflow-finished'}
            payload = ('data: ' + json.dumps({'choices': [{'delta': delta, 'finish_reason': None if case.mode == 'cancel' else 'stop'}]}) + '\n\n').encode()
            if case.mode != 'cancel':
                payload += b'data: [DONE]\n\n'
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            if case.mode != 'cancel':
                self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            self.wfile.flush()
            if case.mode == 'cancel':
                assert self.release.wait(15), 'cancel fixture was not released'
        except Exception as error:
            self.failures.append(repr(error))


class Case:
    def __init__(self, root, port, mode, *, launch_args=(), access="read-only"):
        self.root, self.mode = root, mode
        self.repo = root / '.helm/workflows'
        self.repo.mkdir(parents=True)
        self.user = root / 'config/helm/workflows'
        self.user.mkdir(parents=True)
        self.sessions = root / 'data/helm/sessions'
        self.document = DOCUMENT.replace('version="1.0"', 'version="2.0"')
        self.path = self.repo / 'review-change.toml'
        self.path.write_text(self.document)
        (self.user / 'review-change.toml').write_text(DOCUMENT)
        if mode == 'secret':
            self.path.write_text(self.document.replace('required=true', 'required=true\nsecret=true'))
        config = root / 'config.toml'
        config.write_text(f'provider="openai-chat"\nmodel="workflow-fixture"\nbase_url="http://127.0.0.1:{port}/v1"\napi_key_env="WORKFLOW_FIXTURE_KEY"\naccess="{access}"\nprovider_retry_attempts=1\n')
        env = dict(os.environ, TERM='xterm-256color', HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'), WORKFLOW_FIXTURE_KEY='synthetic-fixture-key')
        self.output = bytearray()
        self.reaped = False
        self.pid, self.master = pty.fork()
        if self.pid == 0:
            os.execve(str(HELM), [str(HELM), '--config', str(config), '--workspace', str(root), *launch_args, 'chat'], env)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 140, 0, 0))
        self.expected = None

    def drain(self):
        if select.select([self.master], [], [], 0.05)[0]:
            try:
                self.output.extend(os.read(self.master, 65536))
            except OSError:
                pass

    def wait(self, predicate, label, timeout=15):
        deadline = time.monotonic() + timeout
        while not predicate():
            assert time.monotonic() < deadline, (self.mode, label, bytes(self.output[-5000:]), Provider.failures)
            self.drain()

    def text(self, value, start=0):
        self.wait(lambda: len(self.output) > start and value in rendered_screen(bytes(self.output), 40, 140), value)

    def send(self, value):
        os.write(self.master, value if isinstance(value, bytes) else value.encode())

    def saved(self):
        files = list(self.sessions.glob('*.json'))
        assert len(files) == 1, files
        return json.loads(files[0].read_text())

    def assert_unaccepted(self):
        assert not Provider.requests, Provider.requests
        for path in self.sessions.glob('*.json'):
            saved = json.loads(path.read_text())
            assert not saved.get('workflow_runs') and not saved['messages'], saved

    def open(self, scope=None):
        self.send('/workflow review-change' + (' --scope ' + scope if scope else '') + '\r')
        self.text('Input 1/2')

    def fill(self):
        self.send(b'\t\x1b[200~' + LITERAL.encode() + b'\x1b[201~\r')
        self.text('SHA-256:')

    def expected_for(self, scope):
        document = self.path.read_bytes() if scope == 'repository' else DOCUMENT.encode()
        self.expected = dict(id='review-change', version='2.0' if scope == 'repository' else '1.0',
                             digest=hashlib.sha256(document).hexdigest(), scope=scope,
                             inputs={'target': LITERAL, 'count': 2})

    def finish(self):
        self.send(b'\x11')
        def exited():
            done, status = os.waitpid(self.pid, os.WNOHANG)
            if done:
                self.reaped = True
                assert os.waitstatus_to_exitcode(status) == 0, (self.mode, status)
                return True
            return False
        self.wait(exited, 'exit', 8)

    def close(self):
        Provider.release.set()
        if not self.reaped:
            os.kill(self.pid, signal.SIGKILL)
            os.waitpid(self.pid, 0)
            self.reaped = True
        os.close(self.master)


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for mode in ['changed', 'denied', 'cancel', 'secret']:
            Provider.requests, Provider.failures = [], []
            Provider.release.clear()
            with tempfile.TemporaryDirectory(prefix='helm-tui-workflow-') as directory:
                case = Case(Path(directory), server.server_port, mode)
                Provider.case = case
                try:
                    case.text('HELM')
                    if mode == 'secret':
                        case.send('/workflow review-change\r')
                        case.text('Input 1/2')
                        case.send(b'\t\x1b[200~hidden-input-canary\x1b[201~')
                        case.text('[hidden]')
                        assert b'hidden-input-canary' not in case.output
                        case.assert_unaccepted()
                        case.send(b'\x1b')
                        case.text('HELM')
                        case.finish()
                        print('workflow secret: hidden input cancelled without invocation or dispatch')
                        continue
                    scope = 'user' if mode == 'denied' else 'repository'
                    case.open(scope)
                    if mode == 'changed':
                        case.send('9\r')
                        case.text('workflow input fails type, bounds or choices')
                        case.assert_unaccepted()
                        case.send(b'\x15')  # Ctrl-U restores unset/default
                    case.fill()
                    case.expected_for(scope)
                    if mode == 'changed':
                        case.send('r')
                        case.text('repository workflow requires')
                        case.assert_unaccepted()
                        case.path.write_text(case.document + '\n')
                        case.send('tr')
                        case.text('Workflow changed; reopen')
                        case.assert_unaccepted()
                        start = len(case.output)
                        case.send(b'\x1b')  # preview -> inputs
                        case.text('Input 2/2', start)
                        case.send(b'\x1b')  # form -> preserved command draft
                        case.text('HELM', start)
                        case.send('\r')
                        case.text('Input 1/2', start)
                        case.fill()
                        case.expected_for(scope)
                    case.send('r' if scope == 'user' else 'tr')
                    if mode == 'cancel':
                        case.text('workflow-partial-before-cancel')
                        case.send('/workflow review-change\r')
                        case.text('Finish or cancel the active run')
                        assert len(case.saved()['workflow_runs']) == 1
                        case.send(b'\x1b')
                        case.wait(lambda: any(r.get('phase') == 'interrupted' for r in case.saved().get('run_summaries', [])), 'cancel persisted')
                        Provider.release.set()
                        saved = case.saved()
                        assert len(saved['workflow_runs']) == 1
                        assert not any(m['role'] == 'assistant' and m['content'] == 'workflow-partial-before-cancel' for m in saved['messages'])
                        assert 'workflow-partial-before-cancel' in json.dumps(saved), saved
                    else:
                        case.text('tui-workflow-finished')
                        case.wait(lambda: any(m['role'] == 'assistant' and m['content'] == 'tui-workflow-finished' for m in case.saved()['messages']), 'canonical response')
                    case.finish()
                    assert len(Provider.requests) == (2 if mode == 'denied' else 1), len(Provider.requests)
                    assert case.saved()['workflow_runs'] == [case.expected]
                    assert not (case.root / 'must-not-exist').exists()
                    assert not (case.root / 'never-created').exists()
                    assert not Provider.failures, Provider.failures
                    print(f'workflow {mode}: real TUI, exact attribution, canonical checkpoint and policy passed')
                finally:
                    case.close()
    finally:
        Provider.release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
