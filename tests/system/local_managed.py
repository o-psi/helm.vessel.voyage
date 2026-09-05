#!/usr/bin/env python3
"""Real managed CLI processes, native HTTP, cancellation and authoritative SQLite."""
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import tempfile
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import response

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()


def run(args, env, expected=0):
    result = subprocess.run([str(HELM), *args], env=env, capture_output=True, text=True, timeout=30)
    assert result.returncode == expected, (args, result.returncode, result.stdout, result.stderr)
    return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({'data': [{'id': 'managed-fixture'}]}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        case = self.server.case
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        try:
            expected = {'openai-chat': '/v1/chat/completions', 'openai-responses': '/v1/responses', 'anthropic': '/v1/messages'}[case.provider]
            assert self.path == expected, self.path
            with case.lock:
                step = len(case.requests)
                case.requests.append(body)
            # Actual first dispatch must follow both durable admission and cleanup registration.
            rows = case.sql('SELECT r.active,o.confirmation FROM runs r JOIN local_cleanup_obligations o ON o.run_id=r.id WHERE r.active=1')
            assert rows == [(1, None)], rows
            if case.mode == 'hold':
                case.started.set()
                assert case.release.wait(25), 'fixture release timeout'
            if case.mode in ('shell', 'denied') and step == 0:
                value = ('shell', {'command': "printf 'one-effect\\n' >> effects.txt"})
            elif case.mode == 'terminal' and step == 0:
                value = ('process', {'action': 'start', 'command': 'sleep 120', 'name': 'managed-owned-pty'})
            else:
                value = 'managed-final-雪'
            data = response(case.provider, value, step)
            status = 500 if case.mode == 'failure' else 200
        except Exception as error:
            case.failures.append(repr(error))
            data = response(case.provider, 'fixture-failed', 999)
            status = 500
        self.send_response(status)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


class Case:
    def __init__(self, root, provider='openai-chat'):
        self.root = root
        self.provider = provider
        self.workspace = root / 'workspace'
        self.workspace.mkdir(parents=True)
        self.storage = root / 'managed-installation'
        self.database = self.storage / 'journal/journal.sqlite3'
        self.config = root / 'config.toml'
        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        self.server.daemon_threads = True
        self.server.case = self
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.env = dict(os.environ, XDG_DATA_HOME=str(root / 'data'), OPENAI_API_KEY='offline-fixture-key', ANTHROPIC_API_KEY='offline-fixture-key')
        self.config.write_text(f'provider="{provider}"\nmodel="managed-fixture"\nbase_url="http://127.0.0.1:{self.server.server_port}/v1"\nprovider_retry_attempts=1\n')
        self.args = ['--config', str(self.config), '--workspace', str(self.workspace), '--access', 'unrestricted', 'managed', '--directory', str(self.storage), '--json']
        self.lock = threading.Lock()
        self.reset()

    def reset(self, mode='success'):
        self.mode, self.requests, self.failures = mode, [], []
        self.started, self.release = threading.Event(), threading.Event()

    def sql(self, sql, params=()):
        with sqlite3.connect(self.database, timeout=5) as database:
            return database.execute(sql, params).fetchall()

    def create(self):
        session = str(uuid.uuid4())
        created = run([*self.args, 'create', '--id', session, '--name', 'managed name 雪'], self.env)[0]
        assert created['event'] == 'session_created' and created['session']['id'] == session
        assert created['session']['revision'] == 0
        return session

    def listing(self, limit=100):
        return run([*self.args, 'list', '--limit', str(limit)], self.env)[0]

    def revision(self, session):
        return next(s['revision'] for s in self.listing()['sessions'] if s['id'] == session)

    def command(self, session, revision=None, command=None, expiry=None, prompt='fixture prompt'):
        return [*self.args, 'submit', session, '--expected-revision', str(self.revision(session) if revision is None else revision),
                '--command-id', command or str(uuid.uuid4()), '--expires-at-ms', str(expiry or int(time.time() * 1000) + 120000), prompt]

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5)
        assert not self.failures, self.failures
        assert not list((self.root / 'data').glob('helm/sessions/*.json'))


def final(events, state='completed', cleanup='observed'):
    terminal = [event for event in events if event['event'] == 'run_terminal']
    assert len(terminal) == 1, events
    assert terminal[0]['run']['state'] == state, terminal
    assert terminal[0]['cleanup'] == cleanup, terminal
    assert not any(event['event'] == 'completion' for event in events)
    return terminal[0]


def transports(root):
    for provider in ('openai-chat', 'openai-responses', 'anthropic'):
        case = Case(root / provider, provider)
        try:
            session = case.create()
            run([*case.args, 'create', '--id', session], case.env, expected=1)
            listed = case.listing(1)
            assert len(listed['sessions']) == 1 and listed['next_after'] is None
            assert 'messages' not in json.dumps(listed) and 'provider_state' not in json.dumps(listed)
            command = case.command(session, revision=0)
            events = run(command, case.env)
            terminal = final(events)
            assert len(case.requests) == 1
            text = ''.join(event['text'] for event in events if event['event'] == 'provisional_text')
            assert text == 'managed-final-雪', events
            usage = terminal['run']['usage']
            stored_before = case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0]
            retry = run(command, case.env)
            assert retry[0]['event'] == 'existing_run' and retry[0]['run']['usage'] == usage
            assert len(case.requests) == 1
            assert case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0] == stored_before
            run([*command[:-1], 'changed payload'], case.env, expected=1)
            run(case.command(session, revision=0), case.env, expected=1)
            assert len(case.requests) == 1
            case.reset('shell')
            final(run(case.command(session), case.env))
            assert (case.workspace / 'effects.txt').read_text() == 'one-effect\n'
            assert len(case.requests) == 2
            assert case.sql('SELECT confirmation FROM local_cleanup_obligations') == [('observed',), ('observed',)]
        finally:
            case.close()


def cancellation(root):
    case = Case(root)
    try:
        for interrupt in ('remote', 'signal', 'crash'):
            session = case.create()
            case.reset('hold')
            command = case.command(session)
            process = subprocess.Popen([str(HELM), *command], env=case.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            assert case.started.wait(15), 'provider not reached'
            entry = next(s for s in case.listing()['sessions'] if s['id'] == session)
            run_id = entry['active_run']['id']
            assert entry['pending_cleanup_run'] == run_id
            # Exact retry observes the active command; a fresh contender cannot dispatch.
            assert run(command, case.env)[0]['event'] == 'existing_run'
            run(case.command(session), case.env, expected=1)
            run([*case.args, 'recover', session], case.env, expected=1)
            assert len(case.requests) == 1
            if interrupt == 'remote':
                # Broken provider configuration must not prevent cancellation/metadata access.
                original = case.config.read_text()
                case.config.write_text('invalid [ config')
                cancelled = run([*case.args, 'cancel', session, '--run', run_id], case.env)[0]
                assert cancelled['event'] == 'cancel_requested'
                case.config.write_text(original)
            elif interrupt == 'signal':
                process.send_signal(signal.SIGINT)
            else:
                process.kill()
            stdout, stderr = process.communicate(timeout=25)
            assert process.returncode != 0, (stdout, stderr)
            if interrupt != 'crash':
                final([json.loads(line) for line in stdout.splitlines()], 'cancelled')
            else:
                recovered = run([*case.args, 'recover', session], case.env)[0]
                assert recovered['run']['state'] == 'interrupted', recovered
                entry = next(s for s in case.listing()['sessions'] if s['id'] == session)
                assert entry['pending_cleanup_run'] == run_id
                run(case.command(session), case.env, expected=1)
                attested = run([*case.args, 'recover', session, '--acknowledge-cleanup', run_id], case.env)[0]
                assert attested['cleanup'] == 'operator_attested'
                assert case.sql('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?', (run_id,)) == [('operator_attested',)]
            case.release.set()
            case.reset()
            final(run(case.command(session), case.env))
    finally:
        case.close()


def failures_and_policy(root):
    case = Case(root)
    try:
        session = case.create()
        case.reset('failure')
        final(run(case.command(session), case.env, expected=1), 'failed')
        assert len(case.requests) == 1
        # Provider failure does not require replay or leave a confirmed-clean turn blocked.
        case.reset('denied')
        denied = case.command(session)
        denied[denied.index('unrestricted')] = 'approval'
        final(run(denied, case.env))
        assert not (case.workspace / 'effects.txt').exists(), 'unattended approval bypassed'
        assert len(case.requests) == 2
        outputs = json.dumps(case.requests[-1])
        assert 'denied' in outputs.lower() or 'approval' in outputs.lower(), outputs
        # Unsupported effectful configurations must fail before command admission/startup.
        original = case.config.read_text()
        before = case.sql('SELECT count(*) FROM commands')[0][0]
        marker = case.root / 'unsupported-started'
        case.config.write_text(original + '\n[mcp_servers.marker]\ncommand="sh"\nargs=["-c", "touch ' + str(marker) + '"]\n')
        run(case.command(session), case.env, expected=1)
        assert not marker.exists()
        assert case.sql('SELECT count(*) FROM commands')[0][0] == before
        case.config.write_text(original.replace('provider="openai-chat"', 'provider="codex-compatibility"'))
        run(case.command(session), case.env, expected=1)
        assert case.sql('SELECT count(*) FROM commands')[0][0] == before
        # Administrative selection does not load either unsupported or malformed config.
        case.config.write_text('malformed [ config')
        assert any(row['id'] == session for row in case.listing()['sessions'])
        run([*case.args, 'recover', session], case.env)
        case.config.write_text(original)
        # A new create with lost stdout remains discoverable; IDs are never replaced.
        lost_id = str(uuid.uuid4())
        result = subprocess.run([str(HELM), *case.args, 'create', '--id', lost_id], env=case.env,
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=20)
        assert result.returncode == 0, result.stderr
        assert any(row['id'] == lost_id for row in case.listing()['sessions'])
        run([*case.args, 'list', '--limit', '0'], case.env, expected=1)
        run([*case.args, 'list', '--limit', '101'], case.env, expected=1)
        # Selection typo cannot initialize a replacement installation or identity.
        missing = case.root / 'missing'
        args = list(case.args)
        args[args.index(str(case.storage))] = str(missing)
        run([*args, 'list'], case.env, expected=1)
        assert not missing.exists()
    finally:
        case.close()


def main():
    with tempfile.TemporaryDirectory(prefix='helm-local-managed-') as temporary:
        root = Path(temporary).resolve()
        transports(root / 'transports')
        cancellation(root / 'cancellation')
        failures_and_policy(root / 'failure-policy')
    print('local managed CLI: native transports, exact retry, revision fences, cancellation and restart cleanup passed')


if __name__ == '__main__':
    main()
