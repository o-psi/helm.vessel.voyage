#!/usr/bin/env python3
"""Dedicated worker final acceptance across real HTTP, wire and journal boundaries."""
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from completion_gate import GateCase, PROPOSAL, FINAL, normalize, response, event
from remote_session import port, wait, HELM, VESSEL, TOKEN, KEY


PARTIAL = "reconciliation-partial-雪"


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        case = self.server.case
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            humans = [message.get('content', '') for message in body.get('input', body.get('messages', [])) if message.get('role') == 'user']
            humans = [content if isinstance(content, str) else ''.join(block.get('text', '') for block in content if block.get('type') in ['text', 'input_text']) for content in humans]
            if 'REMOTE_CHILD_TASK' in humans:
                case.child_requests.append(body)
                case.child_started.set()
                if case.mode not in ['childfailed', 'unrelated']:
                    case.child_release.wait(25)
                status = 400 if case.mode in ['childfailed', 'unrelated'] else 200
                data = b'{"error":{"message":"offline child failure"}}' if status == 400 else response(case.provider, 'CHILD_RESULT_42', 100)
                self.send_response(status)
                self.send_header('Content-Type', 'application/json' if status == 400 else 'text/event-stream')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                return
            step = len(case.requests)
            case.requests.append(body)
            outputs, systems = normalize(case.provider, body)
            saved = case.canonical()
            assert saved['messages'][-1]['role'] in ['user', 'tool', 'assistant']
            assert not any(message['role'] == 'system' for message in saved['messages'])
            assert [message['content'] for message in saved['messages'] if message['role'] == 'user'] == case.prior_prompts+['remote-gate:'+case.mode]
            snapshot = case.inspect()
            assert snapshot['run']['state'] == 'running', snapshot
            assert not any(item['event']['type'] == 'terminal' for item in case.events() if item['run_id'] == snapshot['run']['run_id'])
            if step >= (4 if case.mode == 'unrelated' else 3 if case.mode == 'childfailed' else 2):
                assert any(message['content'] == PROPOSAL for message in saved['messages']), saved
                assert 'reconcil' in systems.lower() and case.todo in systems, systems
            if case.mode == 'partial-cancel' and step == 2:
                if case.provider == 'openai-chat':
                    data = event({'choices':[{'delta':{'content':PARTIAL}}]})
                elif case.provider == 'openai-responses':
                    data = event({'type':'response.output_text.delta', 'delta':PARTIAL})
                else:
                    data = event({'type':'message_start', 'message':{'usage':{'input_tokens':1}}})+event({'type':'content_block_delta', 'index':0, 'delta':{'type':'text_delta', 'text':PARTIAL}})
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                self.wfile.write(data); self.wfile.flush()
                case.hold.set()
                case.release.wait(25)
                return
            if case.mode in ['cancel', 'disconnect', 'revoke', 'crash'] and step == 2:
                case.hold.set()
                case.release.wait(25)
                return
            value = case.respond(step, outputs)
            status = 400 if value == 'FAIL' else 200
            data = b'{"error":{"message":"offline gate failure"}}' if status != 200 else response(case.provider, value, step)
        except Exception as error:
            case.failures.append(repr(error))
            status, data = 400, b'{"error":{"message":"fixture assertion failed"}}'
        try:
            self.send_response(status)
            self.send_header('Content-Type', 'text/event-stream' if status == 200 else 'application/json')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


class Case:
    def __init__(self, root, provider, mode, blocked_output=False):
        root.mkdir()
        self.root, self.provider, self.mode = root, provider, mode
        self.blocked_output, self.output_pipe = blocked_output, None
        self.workspace = root/'workspace'; self.workspace.mkdir()
        (self.workspace/'evidence.txt').write_text('Measured records: 42\n')
        self.requests, self.failures, self.processes, self.logs = [], [], [], []
        self.hold, self.release = threading.Event(), threading.Event()
        self.child_started, self.child_release = threading.Event(), threading.Event()
        self.child_requests = []
        self.prior_prompts = []
        self.todo = self.snapshot = None
        self.database = root/'managed/journal/journal.sqlite3'
        self.env = dict(os.environ, HOME=str(root/'home'), XDG_DATA_HOME=str(root/'data'), XDG_CONFIG_HOME=str(root/'config'), VESSEL_OPERATOR_TOKEN=TOKEN, REMOTE_PROVIDER_KEY=KEY, RUST_LOG='warn')
        self.http = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        self.origin = f'http://127.0.0.1:{port()}'
        self.http.daemon_threads = True
        self.http.case = self
        self.thread = threading.Thread(target=self.http.serve_forever, daemon=True)
        self.thread.start()
        try:
            config = root/'provider.toml'
            config.write_text(f'provider="{provider}"\nmodel="remote-gate-fixture"\nbase_url="http://127.0.0.1:{self.http.server_port}/v1"\napi_key_env="REMOTE_PROVIDER_KEY"\naccess="unrestricted"\ndeny_commands=["rm"]\nprovider_retry_attempts=1\nmax_tokens=256\n')
            self.server_command = [str(VESSEL), '--bind', self.origin.removeprefix('http://'), '--database', str(root/'vessel.db'), '--attachment-directory', str(root/'authority'), '--public-origin', self.origin, '--allow-insecure-loopback', '--remote-execution']
            self.server = self.spawn(self.server_command)
            wait(self.ready, 'Vessel ready')
            invitation = self.request('/v2/enrollment/invitations', {'ttl_ms':60000})
            enrolled = subprocess.run([str(HELM), 'attachment', '--directory', str(root/'enrollment'), '--origin', self.origin, '--allow-insecure-loopback', 'enroll', '--invitation-id', invitation['id'], '--invitation-key-stdin'], input=invitation['key']+'\n', text=True, capture_output=True, env=self.env, timeout=10)
            assert enrolled.returncode == 0, enrolled.stderr
            self.worker_command = [str(HELM), '--config', str(config), '--workspace', str(self.workspace), 'remote-worker', '--directory', str(root/'managed'), '--enrollment-directory', str(root/'enrollment'), '--origin', self.origin, '--allow-insecure-loopback']
            self.worker = self.spawn(self.worker_command)
            connection = wait(self.connected, 'worker connected')
            self.machine = connection['machine_id']
            self.base = '/v1/remote/'+self.machine
            if blocked_output:
                return
            self.session = self.command({'type':'list', 'after':None, 'limit':20})['reply']['sessions'][0]['id']
        except BaseException:
            self.close()
            raise

    def respond(self, step, outputs):
        if self.mode == 'projection-failure':
            if step == 0:
                return 'todo', {'action':'create', 'title':'projection failure obligation'}
            assert step == 1
            self.todo = json.loads(outputs[-1])['id']
            with sqlite3.connect(self.database) as db:
                db.execute("CREATE TRIGGER fixture_text_failure BEFORE INSERT ON remote_events WHEN json_extract(NEW.event,'$.event.type')='text_delta' BEGIN SELECT RAISE(ABORT, 'fixture text projection failure'); END")
            return PROPOSAL
        if self.mode == 'unrelated':
            if step == 0:
                return 'todo', {'action':'create', 'title':'prior pending obligation'}
            if step == 1:
                self.todo = json.loads(outputs[-1])['id']
                return 'subagent', {'action':'spawn', 'name':'prior failed child', 'task':'REMOTE_CHILD_TASK'}
            if step == 2:
                self.child_id = json.loads(outputs[-1])['id']
                return 'subagent', {'action':'wait', 'id':self.child_id}
            if step == 3:
                return PROPOSAL
            assert step == 4
            return 'Prior work remains unresolved'
        if self.mode.startswith('child'):
            if step == 0:
                return 'subagent', {'action':'spawn', 'name':'owned-remote-child', 'task':'REMOTE_CHILD_TASK'}
            if step == 1:
                self.todo = json.loads(outputs[-1])['id']
                assert self.child_started.wait(10), 'child did not start'
                if self.mode == 'childfailed':
                    return 'subagent', {'action':'wait', 'id':self.todo}
                return PROPOSAL
            if step == 2:
                if self.mode == 'childactive':
                    return 'still falsely claiming active child work is complete'
                if self.mode == 'childfailed':
                    return PROPOSAL
                self.child_release.set()
                return 'subagent', {'action':'wait', 'id':self.todo}
            if step == 3:
                if self.mode == 'childlate':
                    assert 'CHILD_RESULT_42' in outputs[-1], outputs[-1]
                return 'completion', {'action':'read', 'kind':'agent', 'id':self.todo}
            if step == 4:
                record = json.loads(outputs[-1])
                assert record['status'] == ('failed' if self.mode == 'childfailed' else 'completed'), record
                return 'completion', {'action':'snapshot'}
            if step == 5:
                self.snapshot = json.loads(outputs[-1])
                assert self.snapshot['total'] == 1 and self.snapshot['accounted'] == 0, self.snapshot
                return 'completion', {'action':'read', 'kind':'agent', 'id':self.todo}
            if step == 6:
                return 'completion', {'action':'account', 'kind':'agent', 'id':self.todo, 'revision':self.snapshot['revision'], 'fingerprint':self.snapshot['fingerprint'], 'disposition':'failure_with_impact' if self.mode == 'childfailed' else 'incorporated', 'reason':'Child failed; verification remains unavailable' if self.mode == 'childfailed' else 'Incorporated CHILD_RESULT_42'}
            assert step == 7, (step, outputs)
            assert not outputs[-1].startswith('Error'), outputs[-1]
            return 'Child failed; verification remains unavailable' if self.mode == 'childfailed' else FINAL
        if self.mode == 'denied':
            if step == 3:
                return 'shell', {'command':'rm -f evidence.txt'}
            if step == 4:
                assert 'denied' in outputs[-1].lower(), outputs[-1]
                assert (self.workspace/'evidence.txt').is_file()
                return FINAL
        if self.mode == 'unknown' and step == 2:
            return 'shell', {'command':"printf 'effect\\n' >> effects; sleep 30"}
        if self.mode == 'commit-failure' and step == 9:
            with sqlite3.connect(self.database) as db:
                db.execute("CREATE TRIGGER fixture_terminal_failure BEFORE INSERT ON remote_events WHEN json_extract(NEW.event,'$.event.type')='terminal' AND json_extract(NEW.event,'$.event.state')='completed' BEGIN SELECT RAISE(ABORT, 'fixture terminal publication failure'); END")
            return FINAL
        if self.mode == 'stale':
            if step == 9:
                return 'todo', {'action':'evidence', 'id':self.todo, 'text':'Changed evidence invalidates the previous review'}
            if step == 10:
                return FINAL
        original = self.mode
        if original in ['stale', 'commit-failure']:
            self.mode = 'verified'
        elif original == 'denied':
            self.mode = 'deferred'
        try:
            return GateCase.respond(self, step, outputs)
        finally:
            self.mode = original

    def spawn(self, command):
        log = tempfile.TemporaryFile(); self.logs.append(log)
        if self.blocked_output and 'remote-worker' in command:
            reader, writer = os.pipe()
            os.set_blocking(writer, False)
            try:
                while True:
                    os.write(writer, b'x'*4096)
            except BlockingIOError:
                pass
            os.set_blocking(writer, True)
            process = subprocess.Popen(command, env=self.env, stdin=subprocess.DEVNULL, stdout=writer, stderr=log)
            os.close(writer)
            self.output_pipe = reader
        else:
            process = subprocess.Popen(command, env=self.env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        self.processes.append(process)
        return process

    def request(self, path, body=None, expected=200):
        headers = {'Authorization':'Bearer '+TOKEN, 'Origin':self.origin, 'x-voyage-request':'2'}
        if body is not None:
            headers['Content-Type'] = 'application/json'
        request = urllib.request.Request(self.origin+path, headers=headers, data=None if body is None else json.dumps(body).encode())
        try:
            result = urllib.request.urlopen(request, timeout=8)
        except urllib.error.HTTPError as error:
            result = error
        with result:
            data = result.read().decode()
            assert result.status == expected, (path, result.status, data)
            if path.startswith('/v1/remote/') and result.status == 200:
                assert result.headers.get('Cache-Control') == 'no-store'
        assert KEY not in data and TOKEN not in data
        return json.loads(data) if data.startswith(('{', '[')) else data

    def ready(self):
        try:
            return self.request('/ready')
        except (OSError, urllib.error.URLError):
            return None

    def connected(self):
        assert self.worker.poll() is None, 'worker exited'
        connections = self.request('/v1/diagnostics')['connections']
        return connections[0] if connections else None

    def command(self, operation, identity=None, expiry=None):
        return self.request(self.base+'/command', {'command_id':identity or str(uuid.uuid4()), 'expires_at_ms':expiry or int(time.time()*1000)+120000, 'operation':operation})

    def inspect(self):
        return self.command({'type':'inspect', 'session_id':self.session})['reply']

    def events(self):
        records, cursor = [], 0
        while True:
            page = self.request(self.base+f'/events?session_id={self.session}&after={cursor}&limit=128')
            assert page['type'] == 'replay', page
            records.extend(page['events'])
            if not page['events'] or page['events'][-1]['cursor'] == page['latest']:
                return records
            cursor = page['events'][-1]['cursor']

    def canonical(self):
        # Read only at a held model-request boundary or after durable cleanup.
        with sqlite3.connect(self.database) as db:
            return json.loads(db.execute('SELECT state FROM sessions WHERE id=?', (self.session,)).fetchone()[0])

    def terminal(self):
        snapshot = self.inspect()
        return snapshot if snapshot['run']['cleanup'] == 'observed' else None

    def execute(self):
        identity, expiry = str(uuid.uuid4()), int(time.time()*1000)+120000
        operation = {'type':'submit', 'session_id':self.session, 'expected_revision':0, 'prompt':'remote-gate:'+self.mode}
        accepted = self.command(operation, identity, expiry)['reply']
        run_id = accepted['run']['run_id']
        if self.mode == 'commit-failure':
            self.commit_failure(operation, identity, expiry, run_id)
            return
        if self.mode in ['disconnect', 'revoke', 'crash']:
            self.transport_interruption(operation, identity, expiry, run_id)
            return
        if self.mode in ['cancel', 'partial-cancel', 'unknown']:
            if self.mode == 'unknown':
                wait(lambda: (self.workspace/'effects').exists(), 'effect before cancellation')
            else:
                assert self.hold.wait(15), self.failures
            if self.mode == 'partial-cancel':
                wait(lambda: PARTIAL in ''.join(item['event'].get('text', '') for item in self.events()), 'durable partial projection before cancellation')
            cancelled = self.command({'type':'cancel', 'session_id':self.session, 'run_id':run_id})['reply']
            assert cancelled['type'] == 'accepted', cancelled
        terminal = wait(self.terminal, 'durable gate outcome', 25)
        assert not self.failures, self.failures
        expected = 'completed' if self.mode in ['empty', 'verified', 'childlate'] else 'cancelled' if self.mode in ['cancel', 'partial-cancel', 'unknown'] else 'failed' if self.mode in ['failure', 'projection-failure'] else 'incomplete'
        assert terminal['run']['state'] == expected, terminal
        saved = self.canonical()
        ledgers = list((self.root/'data/helm/completion').glob('*/ledgers/*.json'))
        assert len(ledgers) == 1, ledgers
        envelope = json.loads(ledgers[0].read_text())
        assert envelope['scope']['session_id'] == self.session
        ledger = json.loads(envelope['ledger'])
        assert ledger['state']['status'] == 'sealed', ledger
        decision = ledger['state']['decision']
        assert decision['outcome'] == ('interrupted' if expected in ['cancelled', 'failed'] else expected), decision
        readiness = decision['readiness']
        assert readiness['total'] == (2 if self.mode == 'unrelated' else int(self.mode != 'empty')), readiness
        assert readiness['accounted'] == int(self.mode in ['verified', 'blocked', 'deferred', 'childfailed', 'childlate']), readiness
        if self.mode in ['blocked', 'deferred', 'childfailed']:
            assert readiness['incomplete'] == 1, readiness
        if self.mode.startswith('child'):
            assert len(self.child_requests) == 1, self.child_requests
            records = []
            for path in (self.root/'data/helm/subagents').glob('*.archive/*.json'):
                records.append(json.loads(path.read_text())['record'])
            for path in (self.root/'data/helm/subagents').glob('*.json'):
                records.extend(json.loads(path.read_text()).get('agents', {}).values())
            owned = [record for record in records if record['id'] == self.todo]
            assert len(owned) == 1, records
            assert owned[0]['status'] == {'childactive':'cancelled', 'childfailed':'failed', 'childlate':'completed'}[self.mode], owned
            assert owned[0]['completion']['run_id'] == ledger['run_id'] and owned[0]['completion']['session_id'] == self.session, owned
        assert not any(message['role'] == 'system' for message in saved['messages'])
        assert [message['content'] for message in saved['messages'] if message['role'] == 'user'] == ['remote-gate:'+self.mode]
        texts = [message['content'] for message in saved['messages'] if message['role'] == 'assistant' and not message.get('tool_calls')]
        assert texts.count(PROPOSAL) == int(self.mode != 'empty'), texts
        assert texts.count(FINAL) == int(expected == 'completed' or self.mode in ['stale', 'denied']), texts
        count = {'empty':1, 'verified':10, 'blocked':8, 'deferred':7, 'ignore':3, 'failure':3, 'cancel':3, 'stale':11, 'denied':5, 'unknown':3, 'childfailed':8, 'childactive':3, 'childlate':8, 'unrelated':5, 'partial-cancel':3, 'projection-failure':2}[self.mode]
        assert len(self.requests) == count, (self.mode, len(self.requests))
        if self.mode == 'partial-cancel':
            with sqlite3.connect(self.database) as db:
                record = json.loads(db.execute('SELECT record FROM runs WHERE id=?', (run_id,)).fetchone()[0])
            assert record['partial_text'] == PROPOSAL+PARTIAL, record
            assert not any(message['content'] == PARTIAL for message in saved['messages'])
        if self.mode == 'unknown':
            assert (self.workspace/'effects').read_text() == 'effect\n'
            unresolved = [call['id'] for message in saved['messages'] for call in message.get('tool_calls', []) if call['name'] == 'shell']
            assert len(unresolved) == 1
            assert not any(message.get('tool_call_id') == unresolved[0] for message in saved['messages']), saved
            denied = self.command({'type':'submit', 'session_id':self.session, 'expected_revision':terminal['session']['revision'], 'prompt':'must remain blocked'})['reply']
            assert denied['type'] == 'denied' and len(self.requests) == count, denied
        events = self.events()
        assert [item['event']['state'] for item in events if item['event']['type'] == 'terminal'] == [expected], events
        assert events[-1]['event'] == {'type':'cleanup', 'state':'observed'}, events[-1]
        if self.mode == 'partial-cancel':
            assert ''.join(item['event'].get('text', '') for item in events) == PROPOSAL+PARTIAL, events
        expected_text = ''.join(message['content'] for message in saved['messages'] if message['role'] == 'assistant')
        if self.mode == 'projection-failure':
            expected_text = ''
            with sqlite3.connect(self.database) as db:
                record = json.loads(db.execute('SELECT record FROM runs WHERE id=?', (run_id,)).fetchone()[0])
            assert record['usage'] == {'input_tokens':2, 'output_tokens':2}, record
            assert record['partial_text'] == '' and not record['final_checkpointed'], record
            assert saved['usage'] == record['usage']
        if self.mode == 'partial-cancel':
            expected_text += PARTIAL
        assert ''.join(item['event'].get('text', '') for item in events) == expected_text, events
        projection = json.dumps(events)
        assert 'provider_state' not in projection and 'arguments' not in projection
        before = json.dumps(saved, sort_keys=True)
        retry = self.command(operation, identity, expiry)['reply']
        assert retry['type'] == 'run' and retry['run_id'] == run_id and retry['state'] == expected, retry
        assert len(self.requests) == count and json.dumps(self.canonical(), sort_keys=True) == before
        self.worker.send_signal(signal.SIGINT); self.worker.wait(20)
        assert self.worker.returncode == 0
        self.worker = self.spawn(self.worker_command)
        wait(self.connected, 'observation after worker restart')
        retry = self.command(operation, identity, expiry)['reply']
        assert retry['run_id'] == run_id and retry['state'] == expected, retry
        assert len(self.requests) == count and self.events() == events
        if self.mode == 'unknown':
            self.worker.send_signal(signal.SIGINT); self.worker.wait(20)
            receipt = self.recover('--reconcile-tools', run_id, '--expected-revision', str(terminal['session']['revision']))['reconciliation']
            assert receipt['tool_call_ids'] == unresolved and not receipt['duplicate'], receipt
            retry = self.recover('--reconcile-tools', run_id, '--expected-revision', str(terminal['session']['revision']))['reconciliation']
            assert retry['duplicate'] and retry['tool_call_ids'] == unresolved, retry
            reconciled = self.canonical()
            assert reconciled['messages'][:-1] == saved['messages']
            result = reconciled['messages'][-1]
            assert result['tool_call_id'] == unresolved[0] and result['tool_success'] is False and 'outcome is unknown' in result['content'], result
            assert (self.workspace/'effects').read_text() == 'effect\n' and len(self.requests) == count
            self.worker = self.spawn(self.worker_command); wait(self.connected, 'reconciled receipt observation')
            assert self.command(operation, identity, expiry)['reply']['state'] == 'cancelled'
            assert not any(item['event'].get('state') == 'completed' for item in self.events())

    def verify_prior_isolation(self):
        saved = self.canonical()
        revision = self.inspect()['session']['revision']
        prior = {}
        for folder in ['completion', 'todos', 'subagents']:
            for path in (self.root/'data/helm'/folder).rglob('*.json'):
                # Runtime event epochs describe observation, not owned records.
                if path.name.endswith('.events.json'):
                    continue
                prior[path] = path.read_bytes()
        self.prior_prompts = ['remote-gate:unrelated']
        self.mode = 'empty'
        self.requests.clear()
        accepted = self.command({'type':'submit', 'session_id':self.session, 'expected_revision':revision, 'prompt':'remote-gate:empty'})['reply']
        run_id = accepted['run']['run_id']
        terminal = wait(self.terminal, 'new clean run ignores prior obligations')
        assert terminal['run']['state'] == 'completed' and len(self.requests) == 1, terminal
        assert not self.failures, self.failures
        assert self.canonical()['messages'][:len(saved['messages'])] == saved['messages']
        for path, original in prior.items():
            assert path.read_bytes() == original, ('new run mutated prior work', path)
        ledgers = list((self.root/'data/helm/completion').glob('*/ledgers/*.json'))
        assert len(ledgers) == 2
        new = [json.loads(json.loads(path.read_text())['ledger']) for path in ledgers if path not in prior]
        assert len(new) == 1 and new[0]['entries'] == [] and new[0]['state']['decision']['outcome'] == 'completed', new
        assert [item['event']['state'] for item in self.events() if item['run_id'] == run_id and item['event']['type'] == 'terminal'] == ['completed']

    def commit_failure(self, operation, identity, expiry, run_id):
        self.worker.wait(25)
        assert self.worker.returncode != 0 and not self.failures, self.failures
        assert len(self.requests) == 10
        saved = self.canonical()
        assert sum(message['content'] == FINAL for message in saved['messages']) == 1
        with sqlite3.connect(self.database) as db:
            record = json.loads(db.execute('SELECT record FROM runs WHERE id=?', (run_id,)).fetchone()[0])
            cleanup = db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?', (run_id,)).fetchone()[0]
            events = [json.loads(row[0])['event'] for row in db.execute('SELECT event FROM remote_events ORDER BY sequence')]
            db.execute('DROP TRIGGER fixture_terminal_failure')
        assert record['state'] == 'running' and record['final_checkpointed'] and cleanup is None, (record, cleanup)
        assert not any(event['type'] == 'terminal' for event in events), events
        ledger_path = next((self.root/'data/helm/completion').glob('*/ledgers/*.json'))
        ledger_bytes = ledger_path.read_bytes()
        assert json.loads(json.loads(ledger_bytes)['ledger'])['state']['decision']['outcome'] == 'completed'
        recovered = self.recover()
        assert recovered['run'] == {'id':run_id, 'state':'interrupted'}, recovered
        self.worker = self.spawn(self.worker_command); wait(self.connected, 'failed-publication recovery observation')
        snapshot = self.inspect()
        assert snapshot['run']['state'] == 'interrupted' and snapshot['run']['cleanup'] == 'unconfirmed', snapshot
        retry = self.command(operation, identity, expiry)['reply']
        assert retry['state'] == 'interrupted' and retry['run_id'] == run_id and len(self.requests) == 10, retry
        denied = self.command({'type':'submit', 'session_id':self.session, 'expected_revision':snapshot['session']['revision'], 'prompt':'must not bypass failed final publication cleanup'})['reply']
        assert denied['type'] == 'denied', denied
        assert ledger_path.read_bytes() == ledger_bytes, 'historical decision was rewritten'
        assert self.canonical()['messages'] == saved['messages']
        assert [item['event']['state'] for item in self.events() if item['event']['type'] == 'terminal'] == ['interrupted']

    def recover(self, *arguments):
        result = subprocess.run([str(HELM), 'remote-worker', '--directory', str(self.root/'managed'), '--recover', *arguments], env=self.env, text=True, capture_output=True, timeout=15)
        assert result.returncode == 0, (result.stdout, result.stderr)
        return json.loads(result.stdout)

    def transport_interruption(self, operation, identity, expiry, run_id):
        assert self.hold.wait(15), self.failures
        if self.mode == 'disconnect':
            self.server.terminate(); self.server.wait(15)
            self.worker.wait(20)
            assert self.worker.returncode != 0
        elif self.mode == 'revoke':
            current = self.connected()
            assert self.request('/v2/enrollment/revoke', {'machine_id':self.machine, 'expected_epoch':current['epoch'], 'transaction_id':str(uuid.uuid4())})['revoked']
            self.worker.wait(25)
            assert self.worker.returncode != 0
        else:
            self.worker.kill(); self.worker.wait(5)
        assert not self.failures and len(self.requests) == 3, self.failures
        saved = self.canonical()
        assert any(message['content'] == PROPOSAL for message in saved['messages'])
        assert not any(message['content'] == FINAL for message in saved['messages'])
        with sqlite3.connect(self.database) as db:
            record = json.loads(db.execute('SELECT record FROM runs WHERE id=?', (run_id,)).fetchone()[0])
            cleanup = db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?', (run_id,)).fetchone()[0]
            events = [json.loads(row[0])['event'] for row in db.execute('SELECT event FROM remote_events ORDER BY sequence')]
        assert not any(event.get('type') == 'terminal' and event.get('state') == 'completed' for event in events), events
        expected = 'interrupted' if self.mode == 'crash' else 'cancelled'
        if self.mode == 'crash':
            assert record['state'] == 'running' and cleanup is None, (record, cleanup)
            recovered = self.recover()
            assert recovered['run'] == {'id':run_id, 'state':'interrupted'}, recovered
            self.worker = self.spawn(self.worker_command); wait(self.connected, 'unconfirmed crash observation')
            snapshot = self.inspect()
            assert snapshot['run']['state'] == 'interrupted' and snapshot['run']['cleanup'] == 'unconfirmed', snapshot
            denied = self.command({'type':'submit', 'session_id':self.session, 'expected_revision':snapshot['session']['revision'], 'prompt':'must not bypass cleanup'})['reply']
            assert denied['type'] == 'denied' and len(self.requests) == 3, denied
            self.worker.send_signal(signal.SIGINT); self.worker.wait(20)
            assert self.recover('--acknowledge-cleanup', run_id)['cleanup'] == 'operator_attested'
            assert self.recover('--acknowledge-cleanup', run_id)['cleanup'] == 'operator_attested'
        else:
            assert record['state'] == 'cancelled' and cleanup == 'observed', (record, cleanup)
        if self.mode == 'revoke':
            self.request(self.base+'/command', {'command_id':identity, 'expires_at_ms':expiry, 'operation':operation}, expected=404)
            return
        if self.mode == 'disconnect':
            self.server = self.spawn(self.server_command); wait(self.ready, 'Vessel restarted')
        self.worker = self.spawn(self.worker_command); wait(self.connected, 'worker observation after interruption')
        retry = self.command(operation, identity, expiry)['reply']
        assert retry['run_id'] == run_id and retry['state'] == expected and len(self.requests) == 3, retry
        events = self.events()
        assert [item['event']['state'] for item in events if item['event']['type'] == 'terminal'] == [expected], events
        assert self.canonical()['messages'] == saved['messages'], 'recovery changed canonical proposal'

    def drain_output(self):
        if self.output_pipe is not None:
            os.set_blocking(self.output_pipe, False)
            try:
                while os.read(self.output_pipe, 65536):
                    pass
            except BlockingIOError:
                pass

    def execute_signal(self, interrupt):
        # Connection diagnostics prove the first biased connect select ran.
        # Its following local notice cannot finish while this pipe is full.
        time.sleep(0.1)
        assert self.worker.poll() is None
        self.worker.send_signal(interrupt)
        time.sleep(0.05)
        self.drain_output()
        self.worker.wait(3)
        assert self.worker.returncode == 0, self.worker.returncode
        assert not self.requests, 'signal test dispatched provider work'

    def close(self):
        self.release.set()
        self.child_release.set()
        self.drain_output()
        for process in reversed(self.processes):
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:
                    process.wait(20)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait(5)
        if self.failures:
            for log in self.logs:
                log.seek(0)
                print(log.read().decode(errors='replace'))
        if self.output_pipe is not None:
            os.close(self.output_pipe)
            self.output_pipe = None
        for log in self.logs:
            log.close()
        self.http.shutdown(); self.http.server_close(); self.thread.join(5)


def main():
    with tempfile.TemporaryDirectory(prefix='helm-remote-gate-') as directory:
        for interrupt in [signal.SIGINT, signal.SIGTERM, signal.SIGHUP]:
            case = Case(Path(directory)/('signal-'+str(interrupt)), 'openai-chat', 'signal', blocked_output=True)
            try:
                case.execute_signal(interrupt)
                print('remote completion: blocked notice signal', interrupt, 'passed', flush=True)
            finally:
                case.close()
        case = Case(Path(directory)/'projection-failure', 'openai-responses', 'projection-failure')
        try:
            case.execute()
            print('remote completion: completed-only projection failure passed', flush=True)
        except BaseException as error:
            case.failures.append(repr(error))
            raise
        finally:
            case.close()
        for provider in ['openai-chat', 'openai-responses', 'anthropic']:
            for mode in ['empty', 'verified', 'blocked', 'deferred', 'ignore', 'failure', 'cancel', 'stale', 'denied', 'unknown', 'disconnect', 'revoke', 'crash', 'childfailed', 'childactive', 'childlate', 'unrelated', 'commit-failure', 'partial-cancel']:
                case = Case(Path(directory)/(provider+'-'+mode), provider, mode)
                try:
                    case.execute()
                    if mode == 'unrelated':
                        case.verify_prior_isolation()
                    print('remote completion:', provider, mode, 'passed', flush=True)
                except BaseException as error:
                    case.failures.append('matrix failure: '+repr(error))
                    raise
                finally:
                    case.close()


if __name__ == '__main__':
    main()
