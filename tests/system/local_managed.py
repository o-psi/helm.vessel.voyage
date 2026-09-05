#!/usr/bin/env python3
"""Real managed CLI processes, native HTTP, cancellation and authoritative SQLite."""
import json
import os
import re
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
            users = [message.get('content', '') for message in body.get('messages', body.get('input', [])) if message.get('role') == 'user']
            is_child = bool(users) and 'managed-child-terminal' in str(users[-1])
            selected = re.search(r'fixture prompt ([0-9a-f-]{36})', str(users[-1])) if users and not is_child else None
            session_key = selected.group(1) if selected else None
            with case.lock:
                first_root = not is_child and session_key not in case.admission_checked
                if first_root:
                    case.admission_checked.add(session_key)
                bucket = 'child' if is_child else 'parent'
                step = case.counts.get(bucket, 0)
                case.counts[bucket] = step + 1
                case.requests.append(body)
            # The first root dispatch precedes all provider bytes and tool effects.
            # Child/repeated dispatches can overlap parent checkpoint commits: raw
            # SQLite reads there would introduce fixture-only write contention.
            if first_root:
                query = 'SELECT r.active,o.run_id,o.confirmation FROM runs r LEFT JOIN local_cleanup_obligations o ON o.run_id=r.id WHERE r.active=1'
                rows = case.sql(query + (' AND r.session_id=?' if selected else ''), (session_key,) if selected else ())
                assert len(rows) == 1 and rows[0][0] == 1 and rows[0][1] and rows[0][2] is None, rows
            if case.mode == 'partial_hold':
                prefix = response(case.provider, 'managed-partial-before-cancel', step).replace(b'data: [DONE]\n\n', b'')
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                self.wfile.write(prefix)
                self.wfile.flush()
                case.started.set()
                assert case.release.wait(25)
                return
            if case.mode == 'hold' or (case.mode == 'child_hold' and is_child and step == 1):
                case.started.set()
                assert case.release.wait(25), 'fixture release timeout'
            if case.mode in ('child', 'child_hold'):
                if is_child:
                    value = ('process', {'action': 'start', 'command': 'sleep 120', 'name': 'child-owned-pty'}) if step == 0 else 'child-terminal-done'
                elif step == 0:
                    value = ('subagent', {'action': 'spawn', 'name': 'managed-child-one', 'task': 'managed-child-terminal-one'})
                elif step == 1 and case.mode == 'child_hold':
                    value = ('subagent', {'action': 'spawn', 'name': 'managed-child-two', 'task': 'managed-child-terminal-two'})
                elif step == (2 if case.mode == 'child_hold' else 1):
                    outputs = [message['content'] for message in body['messages'] if message.get('role') == 'tool']
                    child_id = json.loads(outputs[0])['id']
                    value = ('subagent', {'action': 'wait', 'id': child_id})
                else:
                    value = 'managed-final-雪'
            elif case.mode == 'question' and step == 0:
                value = ('questions', {'question': 'Choose an option', 'options': ['One', 'Two']})
            elif case.mode in ('shell', 'denied') and step == 0:
                value = ('shell', {'command': "printf 'one-effect\\n' >> effects.txt"})
            elif case.mode == 'shell_background' and step == 0:
                value = ('shell', {'command': 'sleep 120 >/dev/null 2>&1 & echo $! > shell-child.pid'})
            elif case.mode == 'terminal' and step == 0:
                value = ('process', {'action': 'start', 'command': 'sleep 120', 'name': 'managed-owned-pty'})
            elif case.mode == 'flood':
                value = 'provisional-large-output-' * 2000
            else:
                value = 'managed-final-雪'
            data = response(case.provider, value, step)
            if case.mode == 'flood':
                data = data.replace(b'data: [DONE]\n\n', b'') * 4 + b'data: [DONE]\n\n'
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
        self.processes = []
        self.reset()

    def reset(self, mode='success'):
        self.mode, self.requests, self.failures = mode, [], []
        self.counts = {}
        self.admission_checked = set()
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

    def command(self, session, revision=None, command=None, expiry=None, prompt=None):
        return [*self.args, 'submit', session, '--expected-revision', str(self.revision(session) if revision is None else revision),
                '--command-id', command or str(uuid.uuid4()), '--expires-at-ms', str(expiry or int(time.time() * 1000) + 120000), prompt or f'fixture prompt {session}']

    def spawn(self, command, **kwargs):
        process = subprocess.Popen([str(HELM), *command], env=self.env, **kwargs)
        self.processes.append(process)
        return process

    def close(self):
        self.release.set()
        for process in self.processes:
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:
                    process.wait(timeout=20)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            for stream in (process.stdout, process.stderr):
                if stream is not None:
                    stream.close()
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
            canonical = json.loads(case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0])
            runs = [json.loads(row[0]) for row in case.sql('SELECT record FROM runs WHERE session_id=?', (session,))]
            for kind in ('input_tokens', 'output_tokens'):
                assert canonical['usage'][kind] == sum(row['usage'][kind] for row in runs)
            assert 'managed-final-雪' in json.dumps(case.requests[0], ensure_ascii=False)
            before = len(case.requests)
            run(case.command(session, expiry=1), case.env, expected=1)
            assert len(case.requests) == before
        finally:
            case.close()


def cancellation(root):
    case = Case(root)
    try:
        for interrupt in ('remote', 'signal', 'terminate', 'crash'):
            session = case.create()
            case.reset('hold')
            command = case.command(session)
            process = case.spawn(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
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
            elif interrupt in ('signal', 'terminate'):
                process.send_signal(signal.SIGINT if interrupt == 'signal' else signal.SIGTERM)
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
        case.reset('question')
        final(run(case.command(session), case.env))
        assert len(case.requests) == 2 and 'unavailable' in json.dumps(case.requests[-1])
        # Unsupported effectful configurations must fail before command admission/startup.
        original = case.config.read_text()
        before = case.sql('SELECT count(*) FROM commands')[0][0]
        case.config.write_text(original + 'command_timeout_secs=9223372036854775807\n')
        run(case.command(session), case.env, expected=1)
        assert case.sql('SELECT count(*) FROM commands')[0][0] == before
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
        # Stored metadata is untrusted even when it originated in an older local writer.
        state = json.loads(case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0])
        state['name'] = 'hostile\x1b[2J\u202e-name'
        with sqlite3.connect(case.database) as database:
            database.execute('UPDATE sessions SET state=? WHERE id=?', (json.dumps(state), session))
        human_args = [arg for arg in case.args if arg != '--json']
        human = subprocess.run([str(HELM), *human_args, 'list'], env=case.env, capture_output=True, text=True, timeout=20)
        assert human.returncode == 0 and session in human.stdout
        assert '\x1b' not in human.stdout and '\u202e' not in human.stdout and not human.stdout.startswith('{')
        alias = case.root / 'alias'
        alias.symlink_to(case.storage, target_is_directory=True)
        alias_args = list(case.args)
        alias_args[alias_args.index(str(case.storage))] = str(alias)
        run([*alias_args, 'list'], case.env, expected=1)
    finally:
        case.close()


def output_and_terminal_cleanup(root):
    case = Case(root)
    try:
        session = case.create()
        # Broken receipt output fails before dispatch and clears only the unused obligation.
        process = case.spawn(case.command(session),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        process.stdout.close()
        process.wait(timeout=20)
        assert process.returncode != 0
        assert not case.requests
        assert case.sql('SELECT confirmation FROM local_cleanup_obligations') == [('observed',)]
        # A full output pipe cannot hang the async worker or indefinitely retain the owner.
        case.reset('flood')
        process = case.spawn(case.command(session),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, pipesize=4096)
        process.wait(timeout=25)  # Deliberately do not drain stdout until the process exits.
        stdout, stderr = process.communicate(timeout=2)
        assert process.returncode != 0, (stdout[-200:], stderr)
        row = case.sql('SELECT record FROM runs ORDER BY rowid DESC LIMIT 1')[0][0]
        assert json.loads(row)['state'] == 'cancelled', row
        assert case.sql('SELECT confirmation FROM local_cleanup_obligations ORDER BY rowid DESC LIMIT 1') == [('observed',)]
        case.reset('shell_background')
        final(run(case.command(session), case.env))
        pid = int((case.workspace / 'shell-child.pid').read_text())
        stat = Path(f'/proc/{pid}/stat')
        assert not stat.exists() or stat.read_text().rsplit(')', 1)[1].split()[0] in ('Z', 'X'), 'shell descendant still executing after observed cleanup'
        case.reset('terminal')
        final(run(case.command(session), case.env))
        assert len(case.requests) == 2
        case.reset()
        final(run(case.command(session), case.env))
    finally:
        case.close()


def partial_cancellation(root):
    case = Case(root)
    reader = None
    try:
        session = case.create()
        case.reset('partial_hold')
        command = case.command(session)
        process = case.spawn(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        events, read_errors = [], []
        partial_saved = threading.Event()

        def capture_output():
            try:
                for line in process.stdout:
                    event = json.loads(line)
                    events.append(event)
                    if event.get('event') == 'provisional_text' and event.get('text') == 'managed-partial-before-cancel':
                        partial_saved.set()
            except (ValueError, OSError) as error:
                read_errors.append(repr(error))

        reader = threading.Thread(target=capture_output, daemon=True)
        reader.start()
        assert case.started.wait(15)
        # Agent awaits the durable partial checkpoint before this event. Avoid
        # racing that write with raw SQLite polling, then verify disk separately.
        assert partial_saved.wait(10), ('partial delta not observed before cancellation', read_errors)
        rows = case.sql('SELECT record FROM runs WHERE session_id=?', (session,))
        assert len(rows) == 1 and json.loads(rows[0][0])['partial_text'] == 'managed-partial-before-cancel', rows
        run_id = json.loads(rows[0][0])['id']
        run([*case.args, 'cancel', session, '--run', run_id], case.env)
        process.wait(timeout=25)
        reader.join(2)
        assert not reader.is_alive() and not read_errors, read_errors
        stderr = process.stderr.read()
        assert process.returncode != 0, stderr
        final(events, 'cancelled')
        canonical = case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0]
        assert 'managed-partial-before-cancel' not in canonical
        saved = json.loads(case.sql('SELECT record FROM runs WHERE id=?', (run_id,))[0][0])
        assert saved['partial_text'] == 'managed-partial-before-cancel' and saved['state'] == 'cancelled'
        assert run(command, case.env)[0]['run']['state'] == 'cancelled'
        assert len(case.requests) == 1
        case.release.set()
        case.reset()
        final(run(case.command(session), case.env))
        assert 'managed-partial-before-cancel' not in json.dumps(case.requests[0])
        assert json.loads(case.sql('SELECT record FROM runs WHERE id=?', (run_id,))[0][0])['partial_text'] == 'managed-partial-before-cancel'
    finally:
        try:
            case.close()
        finally:
            if reader is not None:
                reader.join(2)
                assert not reader.is_alive(), 'partial output reader did not stop'


def storage_failure(root):
    case = Case(root)
    try:
        session = case.create()
        case.reset('hold')
        process = case.spawn(case.command(session),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert case.started.wait(15)
        run_id = next(row['active_run']['id'] for row in case.listing()['sessions'] if row['id'] == session)
        database = sqlite3.connect(case.database, timeout=5)
        try:
            database.execute('BEGIN EXCLUSIVE')
            # Cancellation polling must fail closed, not treat an unreadable intent as absent.
            stdout, stderr = process.communicate(timeout=20)
            assert process.returncode != 0, (stdout, stderr)
            events = [json.loads(line) for line in stdout.splitlines()]
            assert not any(event['event'] == 'run_terminal' and event['run']['state'] == 'completed' for event in events)
            assert len(case.requests) == 1
        finally:
            database.rollback()
            database.close()
        entry = next(row for row in case.listing()['sessions'] if row['id'] == session)
        assert entry['pending_cleanup_run'] == run_id
        run([*case.args, 'recover', session], case.env)
        run(case.command(session), case.env, expected=1)
        run([*case.args, 'recover', session, '--acknowledge-cleanup', run_id], case.env)
        case.release.set()
        case.reset()
        final(run(case.command(session), case.env))
    finally:
        case.close()


def independent_sessions(root):
    case = Case(root)
    try:
        first = case.create()
        other_workspace = case.root / 'other-workspace'
        other_workspace.mkdir()
        second = str(uuid.uuid4())
        other_args = list(case.args)
        other_args[other_args.index(str(case.workspace))] = str(other_workspace)
        run([*other_args, 'create', '--id', second], case.env)
        case.reset('hold')
        process = case.spawn(case.command(first),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert case.started.wait(15)
        # An idle provider in one session cannot monopolize another session's fence.
        case.mode = 'success'
        second_command = case.command(second)
        second_command[second_command.index(str(case.workspace))] = str(other_workspace)
        final(run(second_command, case.env))
        assert len(case.requests) == 2
        case.release.set()
        stdout, stderr = process.communicate(timeout=20)
        assert process.returncode == 0, stderr
        final([json.loads(line) for line in stdout.splitlines()])
        assert len({json.loads(row[0])['machine_id'] for row in case.sql('SELECT record FROM runs')}) == 1
    finally:
        case.close()


def explicit_upgrade(root):
    case = Case(root)
    try:
        session = case.create()
        before = case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0]
        with sqlite3.connect(case.database) as database:
            database.executescript('DROP TABLE remote_cleanup_attestations; DROP TABLE remote_text; DROP TABLE remote_receipts; DROP TABLE remote_events; DROP TABLE remote_tools; DROP TABLE remote_session; DROP TABLE local_tool_reconciliations; UPDATE attachment_schema SET version=5;')
        case.config.write_text('invalid [ provider configuration')
        assert case.listing()['sessions'][0]['id'] == session
        assert case.sql('SELECT version FROM attachment_schema') == [(5,)], 'selection implicitly upgraded operator state'
        run([*case.args, 'upgrade'], case.env)
        assert case.sql('SELECT version FROM attachment_schema') == [(7,)]
        assert case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0] == before
        assert case.sql("SELECT count(*) FROM sqlite_master WHERE name='local_tool_reconciliations'") == [(1,)]
        assert not case.requests
    finally:
        case.close()


def child_cleanup(root):
    case = Case(root)
    try:
        session = case.create()
        case.reset('child')
        # The child's live terminal keeps readiness incomplete until owned cleanup.
        final(run(case.command(session), case.env, expected=1), 'incomplete')
        assert case.counts == {'parent': 4, 'child': 2}, case.counts
        # One active and one queued child must both stop before the fence hands off.
        session = case.create()
        case.config.write_text(case.config.read_text() + 'subagent_max_concurrency=1\n')
        case.reset('child_hold')
        process = case.spawn(case.command(session),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert case.started.wait(15)
        deadline = time.monotonic() + 15
        while case.counts.get('parent', 0) < 3 and time.monotonic() < deadline:
            time.sleep(0.01)
        assert case.counts.get('parent') == 3, case.counts
        run_id = next(row['active_run']['id'] for row in case.listing()['sessions'] if row['id'] == session)
        run([*case.args, 'cancel', session, '--run', run_id], case.env)
        stdout, stderr = process.communicate(timeout=25)
        assert process.returncode != 0, stderr
        final([json.loads(line) for line in stdout.splitlines()], 'cancelled')
        assert case.counts['child'] == 2, 'queued child dispatched'
        # Cleanup does not invent the missing result of the interrupted wait call.
        run(case.command(session), case.env, expected=1)
        revision = case.revision(session)
        reconcile = [*case.args, 'recover', session, '--reconcile-tools', run_id, '--expected-revision', str(revision)]
        first = run(reconcile, case.env)[0]['reconciliation']
        assert first['duplicate'] is False and first['tool_call_ids']
        snapshot = case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0]
        repeated = run(reconcile, case.env)[0]['reconciliation']
        assert repeated['duplicate'] is True and repeated['tool_call_ids'] == first['tool_call_ids']
        assert case.sql('SELECT state FROM sessions WHERE id=?', (session,))[0][0] == snapshot
        assert 'outcome is unknown' in snapshot and 'not retried' in snapshot
        assert case.counts['child'] == 2
        case.release.set()
        case.reset()
        final(run(case.command(session), case.env))
        assert 'outcome is unknown' in json.dumps(case.requests[0])
    finally:
        case.close()


def main():
    with tempfile.TemporaryDirectory(prefix='helm-local-managed-') as temporary:
        root = Path(temporary).resolve()
        transports(root / 'transports')
        cancellation(root / 'cancellation')
        failures_and_policy(root / 'failure-policy')
        output_and_terminal_cleanup(root / 'output-terminal')
        partial_cancellation(root / 'partial')
        storage_failure(root / 'storage-failure')
        independent_sessions(root / 'independent')
        explicit_upgrade(root / 'upgrade')
        child_cleanup(root / 'children')
    print('local managed CLI: native transports, exact retry, revision fences, cancellation and restart cleanup passed')


if __name__ == '__main__':
    main()
