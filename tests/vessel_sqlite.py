#!/usr/bin/env python3
"""Offline embedded SQLite creation, catalogue and restart regression; no live providers."""
import argparse
import http.server
import json
import os
from pathlib import Path
import shlex
import sqlite3
import subprocess
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'voyage/tests'))
from delivery_recovery import Fixture, wait_for


class ChatProvider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        assert self.path == '/v1/models', self.path
        payload = json.dumps({'data':[{'id':'fixture-model'}]}).encode()
        self.send_response(200)
        self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.bodies.append(body)
        payload = ('data: ' + json.dumps({'choices': [{'index': 0, 'delta': {'content': 'Fixture finished.'}, 'finish_reason': None}]}) + '\n\n'
                   + 'data: ' + json.dumps({'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]}) + '\n\ndata: [DONE]\n\n').encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


class SQLiteFixture(Fixture):
    def __init__(self, binaries):
        super().__init__(binaries)
        self.actual_binaries = binaries
        self.provider.RequestHandlerClass = ChatProvider
        self.env['SQLITE_FIXTURE_KEY'] = 'synthetic-key'
        self.launch_log = self.root / 'launches.txt'
        wrapper_dir = self.root / 'bin'
        wrapper_dir.mkdir()
        for binary in ('helm', 'vessel'):
            (wrapper_dir / binary).symlink_to(binaries / binary)
        wrapper = wrapper_dir / 'voyage'
        wrapper.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> ' + shlex.quote(str(self.launch_log)) + '\nexec ' + shlex.quote(str(binaries / 'voyage')) + ' "$@"\n')
        wrapper.chmod(0o700)
        self.binaries = wrapper_dir
        config = Path(self.env['XDG_CONFIG_HOME']) / 'helm/config.toml'
        config.parent.mkdir(mode=0o700)
        config.write_text('provider = "openai"\nmodel = "fixture-model"\naccess = "read-only"\ncontext_window = 0\n')
        config.chmod(0o600)
        connection = self.cli('connect', '--label', 'SQLite fixture', '--endpoint',
                              f'http://127.0.0.1:{self.provider.server_port}/v1', '--transports', 'openai-chat')
        account = self.cli('add', '--connection', connection['id'], '--account', 'fixture', '--env', 'SQLITE_FIXTURE_KEY')
        self.account = {'account_id': account['id'], 'connection_id': connection['id'],
                        'identity_generation': account['identity_generation'],
                        'connection_revision': connection['revision'], 'transport': 'openai_chat'}

    def cli(self, *args):
        result = subprocess.run([str(self.binaries / 'vessel'), 'auth', 'accounts', *args],
                                env=self.env, cwd=self.workspace, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)

    def owned_processes(self):
        wrapper = self.binaries
        self.binaries = self.actual_binaries
        try:
            return super().owned_processes()
        finally:
            self.binaries = wrapper

    def new(self):
        session = str(uuid.uuid4())
        self.sessions.append(session)
        command = {'op': 'start_account', 'command_id': str(uuid.uuid4()), 'session_id': session,
                   'workspace': str(self.workspace), 'account': self.account, 'model': 'fixture-model',
                   'reasoning_effort': None, 'service_tier': None}
        return command, self.request(command)

    def observers(self):
        return self.launch_log.read_text().splitlines().count('observe-suspended') if self.launch_log.exists() else 0


def bulk_catalogue(binaries):
    f = SQLiteFixture(binaries)
    try:
        f.directory.mkdir(mode=0o700)
        sessions = f.directory / 'sessions'
        sessions.mkdir(mode=0o700)
        identities = []
        for index in range(257):
            session, incarnation = str(uuid.uuid4()), str(uuid.uuid4())
            identities.append(session)
            directory = sessions / session
            directory.mkdir(mode=0o700)
            journal = directory / 'journal'
            journal.mkdir(mode=0o700)
            registration = {'protocol': 1, 'session_id': session, 'incarnation': incarnation,
                'command_id': str(uuid.uuid4()), 'workspace': str(f.workspace), 'state': 'starting',
                'name': f'Persisted {index}', 'token': 'synthetic-private',
                'executable': str(f.binaries / 'voyage')}
            for name, value in [('registration.json', registration), ('stopped.json', {
                'session_id': session, 'incarnation': incarnation, 'cleanup_observed': True, 'suspended': True})]:
                path = directory / name
                path.write_text(json.dumps(value))
                path.chmod(0o600)
            path = journal / 'journal.sqlite3'
            with sqlite3.connect(path) as db:
                db.executescript("CREATE TABLE attachment_schema(id INTEGER, version INTEGER); INSERT INTO attachment_schema VALUES(1,12);"
                    "CREATE TABLE sessions(id TEXT,revision INTEGER,state TEXT); CREATE TABLE runs(id TEXT,session_id TEXT,record TEXT);"
                    "CREATE TABLE process_observations(cursor INTEGER,session_id TEXT); CREATE TABLE process_lifecycle(session_id TEXT,archived INTEGER,deleted INTEGER);"
                    "CREATE TABLE local_cleanup_obligations(run_id TEXT,session_id TEXT,confirmation TEXT);")
                db.execute('INSERT INTO sessions VALUES(?,1,?)', (session, json.dumps({'name': f'Persisted {index}',
                    'model': 'fixture', 'created_at': '2026-01-01T00:00:00Z', 'messages': [], 'run_summaries': []})))
            path.chmod(0o600)
        f.start()
        def hydrated():
            entries = f.request({'op': 'catalogue'})
            return entries if len(entries) == 257 and all(p['catalogue']['summary'] and not p['catalogue']['stale'] for p in entries) else None
        wait_for(hydrated, timeout=30)
        cpu_before = int(Path(f'/proc/{f.supervisor.pid}/stat').read_text().split()[13]) + int(Path(f'/proc/{f.supervisor.pid}/stat').read_text().split()[14])
        start = time.monotonic()
        times = []
        for _ in range(30):
            tick = time.monotonic()
            assert len(f.request({'op': 'catalogue'})) == 257
            times.append(time.monotonic() - tick)
            time.sleep(.1)
        elapsed = time.monotonic() - start
        cpu_after = int(Path(f'/proc/{f.supervisor.pid}/stat').read_text().split()[13]) + int(Path(f'/proc/{f.supervisor.pid}/stat').read_text().split()[14])
        assert f.observers() == 0
        assert max(times) < 1, times
        f.record('257-voyage-catalogue', {'requests':30,'elapsed_seconds':elapsed,'max_request_seconds':max(times),
            'observer_starts':f.observers(),'supervisor_cpu_seconds':(cpu_after-cpu_before)/os.sysconf('SC_CLK_TCK')})
        session = identities[0]
        path = sessions / session / 'journal/journal.sqlite3'
        with sqlite3.connect(path) as db:
            db.execute("UPDATE sessions SET revision=2,state=json_set(state,'$.name','Renamed from journal')")
        wait_for(lambda: any(p['session_id'] == session and p['name'] == 'Renamed from journal' for p in f.request({'op':'catalogue'})))
        f.record('canonical-summary-invalidation', {'session_id':session,'revision':2})
    finally:
        f.close()


def scoped_catalogue(f, session, start):
    for rights in (['observe','lifecycle'], ['observe','lifecycle','history']):
        credential = f.request({'op':'grant','command_id':str(uuid.uuid4()),'grant_id':str(uuid.uuid4()),
            'principal_id':str(uuid.uuid4()),'session_id':session,'workspace':str(f.workspace),
            'rights':rights,'expires_at_ms':int(time.time()*1000)+60000,'endpoint':'http://127.0.0.1'})
        request = {'op':'granted','grant_id':credential['grant_id'],'token':credential['token'],'command':{'op':'catalogue'}}
        entries=f.request(request)
        assert len(entries)==1 and entries[0]['session_id']==session
        assert (entries[0]['catalogue']['summary'] is not None) == ('history' in rights)
        resolution = dict(request, expected_vessel_id=f.request({'op':'capabilities'})['vessel_id'], command=dict(start, op='resolve_start_account'))
        receipt = f.request(resolution)
        assert receipt['status'] == 'created'
        assert (receipt['process']['catalogue']['summary'] is not None) == ('history' in rights)
        f.request({'op':'revoke_grant','command_id':str(uuid.uuid4()),'grant_id':credential['grant_id'],'expected_revision':1})
        assert f.request(request,allow_error=True)['error'], 'revoked cached read succeeded'
    # Exercise the separate human-connection path with synthetic pre-enrolled
    # authority; this fixture does not claim to test invitation redemption/TLS.
    import hashlib
    vessel=f.request({'op':'capabilities'})['vessel_id']
    directory=f.directory/'access/connections'
    directory.mkdir(parents=True,exist_ok=True,mode=0o700)
    token='a'*64
    id=str(uuid.uuid4())
    grant={'schema_version':1,'grant_id':id,'principal_id':str(uuid.uuid4()),'vessel_id':vessel,
        'revision':1,'rights':['catalogue','create'],'expires_at_ms':int(time.time()*1000)+60000,'revoked':False,
        'token_hash':hashlib.sha256(token.encode()).hexdigest(),'workspaces':[{'id':str(uuid.uuid4()),'name':'Fixture','path':str(f.workspace),'provider_ready':True}]}
    path=directory/(id+'.json')
    def save():
        path.write_text(json.dumps(grant));path.chmod(0o600)
    save()
    request={'op':'granted','expected_vessel_id':vessel,'grant_id':id,'token':token,'command':{'op':'catalogue'}}
    assert all(p['catalogue']['summary'] is None for p in f.request(request))
    resolution = dict(request, command=dict(start, op='resolve_start_account'))
    assert f.request(resolution)['process']['catalogue']['summary'] is None
    grant['rights'].append('history');grant['revision']=2;save()
    assert any(p['catalogue']['summary'] for p in f.request(request))
    assert f.request(resolution)['process']['catalogue']['summary']
    grant['revoked']=True;grant['revision']=3;save()
    assert f.request(request,allow_error=True)['error']
    f.record('scoped-catalogue-history-right-and-revocation', {'process_and_connection_scopes':True})


def helm_first_send(f):
    from ui_journeys import launch, screen, send, paste
    from images_composer import stop_pty
    probe = subprocess.run([str(f.binaries / 'helm'), 'connect', '--directory', str(f.directory), '--no-start', 'list'], env=f.env, cwd=f.workspace, capture_output=True, text=True, timeout=15)
    (f.root / 'helm-list.txt').write_text(probe.stdout + probe.stderr)
    assert probe.returncode == 0, probe.stderr
    env = dict(f.env, TERM='xterm-256color')
    pty = launch([str(f.binaries / 'helm'), 'connect', '--directory', str(f.directory), '--no-start'],
                 env, f.workspace, f.root / 'helm-first-send.pty', 120, 32)
    try:
        wait_for(lambda: 'Voyage' in screen(pty) or 'voyage' in screen(pty), timeout=15)
        time.sleep(3.5)
        (f.root / 'helm-before-input.txt').write_text(screen(pty))
        send(pty, '\x0e')
        wait_for(lambda: 'First message' in screen(pty) or 'Choose account' in screen(pty), timeout=15)
        if 'Choose account' in screen(pty):
            wait_for(lambda: 'Signed in' in screen(pty), timeout=15)
            send(pty, '\r')
            wait_for(lambda: 'Enter Apply' in screen(pty) or 'First message' in screen(pty), timeout=15)
            if 'Enter Apply' in screen(pty):
                send(pty, '\r')
        wait_for(lambda: 'First message' in screen(pty), timeout=15)
        paste(pty, 'SQLite Helm first-send fixture')
        time.sleep(.2)
        send(pty, '\r')
        wait_for(lambda: any('SQLite Helm first-send fixture' in json.dumps(body) for body in f.provider.bodies), timeout=20)
        def finished():
            rows = f.request({'op':'catalogue'})
            for row in rows:
                if row['session_id'] not in f.sessions:
                    f.sessions.append(row['session_id'])
                data = row.get('catalogue',{}).get('summary')
                if data and data['run_state'] == 'completed' and row['state'] == 'suspended' and row['session_id'] != f.sessions[0]:
                    return row
            return None
        row = wait_for(finished)
        wait_for(lambda: 'Fixture finished.' in screen(pty), timeout=15)
        f.record('helm-first-send-through-sqlite', {'session_id':row['session_id'],'screen_shows_reply':True})
        # Revisit both endpoints of the two-entry picker after catalogue refresh
        # has replaced the inactive view with a compact projection.
        def transcript():
            return '\n'.join(line[30:] for line in screen(pty).splitlines())
        seen = []
        for key in ['\x1b[H', '\x1b[F', '\x1b[H']:
            time.sleep(2)
            send(pty, '\x1bOQ')
            wait_for(lambda: 'Find voyage' in screen(pty) or 'Type to filter' in screen(pty), timeout=10)
            send(pty, key)
            send(pty, '\r')
            expected = None if not seen else ('SQLite Helm first-send fixture' if seen[-1] == 'first SQLite turn' else 'first SQLite turn')
            wait_for(lambda: 'Fixture finished.' in transcript() and (expected is None or expected in transcript()), timeout=15)
            text = transcript()
            seen.append('first SQLite turn' if 'first SQLite turn' in text else 'SQLite Helm first-send fixture' if 'SQLite Helm first-send fixture' in text else None)
        assert seen[0] == seen[2] and seen[0] != seen[1] and all(seen), seen
        f.record('helm-reselected-conversation-restored', {'distinct_conversations':2,'revisited':True})
    finally:
        (f.root / 'helm-final-screen.txt').write_text(screen(pty))
        stop_pty(pty, wait_for)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    args = parser.parse_args()
    f = SQLiteFixture(args.bin_dir.resolve())
    try:
        f.start()
        f.request({'op': 'account_set_default', 'command_id': str(uuid.uuid4()),
                   'workspace': str(f.workspace), 'account': f.account, 'expected_revision': 0})
        start, process = f.new()
        session = start['session_id']
        summary = process['catalogue']['summary']
        assert summary['session_id'] == session and summary['revision'] == 0, process
        assert f.request(start) == process, 'creation receipt changed on exact retry'
        assert f.observers() == 0, 'creation needed suspended observer'
        changed = dict(start, model='different')
        assert f.request(changed, allow_error=True)['error'], 'changed creation payload accepted'
        f.record('atomic-creation-with-revision-and-exact-receipt', process)
        original = {'op': 'submit', 'command_id': str(uuid.uuid4()), 'expected_revision': summary['revision'],
                    'expires_at_ms': int(time.time() * 1000) + 60000, 'prompt': 'first SQLite turn'}
        f.command(session, original)
        completed = f.finished(session)
        f.suspended(session)
        def saved():
            rows = f.request({'op': 'catalogue'})
            row = next(p for p in rows if p['session_id'] == session)
            data = row['catalogue'].get('summary')
            return row if data and data['run_state'] == 'completed' and data['revision'] == completed['revision'] and data['pending_cleanup_run'] is None and row['state'] == 'suspended' else None
        row = wait_for(saved)
        before = f.observers()
        times = []
        for _ in range(20):
            tick = time.monotonic()
            assert saved()
            times.append(time.monotonic() - tick)
            time.sleep(.1)
        assert f.observers() == before, 'idle catalogue launched observers'
        f.record('catalogue-without-observer-startup', {'requests': 20, 'max_seconds': max(times), 'observer_starts': 0})
        journal = f.directory / 'sessions' / session / 'journal/journal.sqlite3'
        with sqlite3.connect(journal) as db:
            state = json.loads(db.execute('select state from sessions where id=?', (session,)).fetchone()[0])
            assert any(m['content'] == 'first SQLite turn' for m in state['messages'])
        f.supervisor.terminate()
        f.supervisor.wait(timeout=10)
        f.start()
        resolved = f.request(dict(start, op='resolve_start_account'))
        assert resolved['status'] == 'created' and resolved['process'] == process
        restored = wait_for(saved)['catalogue']['summary']
        assert restored == row['catalogue']['summary'], (row['catalogue']['summary'], restored)
        assert f.command(session, {'op': 'receipt', 'command_id': original['command_id']})['status'] == 'accepted'
        assert len(f.provider.bodies) == 1, 'restart replayed provider effects'
        with sqlite3.connect(f.directory / 'catalogue.sqlite3') as db:
            assert db.execute('pragma integrity_check').fetchone() == ('ok',)
            assert db.execute('select count(*) from voyages').fetchone()[0] == 1
            assert db.execute('select count(*) from creation_receipts').fetchone()[0] == 1
        f.record('restart-restores-catalogue-creation-and-canonical-receipt', {'provider_requests': len(f.provider.bodies)})
        # Journal unavailability must retain last-known details and classify staleness.
        original_mode = journal.stat().st_mode & 0o777
        journal.chmod(0o644)
        try:
            stale = wait_for(lambda: next((p for p in f.request({'op': 'catalogue'})
                if p['session_id'] == session and p['catalogue']['stale'] and p['catalogue']['error_code'] == 'journal_unavailable'), None))
            assert stale['catalogue']['summary']['revision'] == row['catalogue']['summary']['revision']
        finally:
            journal.chmod(original_mode)
        f.record('unavailable-journal-retains-details', {'revision': stale['catalogue']['summary']['revision']})
        scoped_catalogue(f, session, start)
        helm_first_send(f)
    finally:
        f.close()
    bulk_catalogue(args.bin_dir.resolve())


if __name__ == '__main__':
    main()
