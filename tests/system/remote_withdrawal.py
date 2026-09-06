#!/usr/bin/env python3
"""Permanent dedicated-grant withdrawal across local CLI, Journal, worker and Vessel."""
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import tempfile
import time
import uuid
from http.server import BaseHTTPRequestHandler

from remote_completion import Case
from remote_session import HELM, KEY, TOKEN, wait
from completion_gate import response


class LateProvider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        case = self.server.case
        case.requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        case.hold.set()
        case.release.wait(25)
        data = response(case.provider, ('write_file', {'path':'late-effect', 'content':'must never execute'}), 0)
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


class FreshProvider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        case = self.server.case
        case.requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        data = response(case.provider, 'NEW_EMPTY_GRANT', len(case.requests))
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def admin(case, *args, success=True):
    # Invalid provider configuration cannot obstruct this local authority operation.
    for attempt in range(5):
        result = subprocess.run([str(HELM), '--config', str(case.root/'invalid-provider.toml'), 'remote-consent', '--directory', str(case.root/'managed'), *args], env=case.env, capture_output=True, text=True, timeout=10)
        if success and result.returncode != 0 and result.stderr.strip() == 'Error: remote consent store busy; preserve the request and explicitly retry the identical operation':
            case.admin_busy = getattr(case, 'admin_busy', []) + [args]
            print('explicit identical administrative retry after typed Busy',case.provider,case.mode,args[0],attempt+1,flush=True)
            time.sleep(.05)
            continue
        break
    assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
    assert KEY not in result.stdout+result.stderr and TOKEN not in result.stdout+result.stderr
    return json.loads(result.stdout) if success else result


def execute(root, provider, mode):
    case = Case(root, provider, 'unknown' if mode == 'owned' else 'cancel')
    try:
        (root/'invalid-provider.toml').write_text('provider = [invalid')
        status = admin(case, 'inspect')
        assert status['revision'] == 0 and not status['withdrawn'] and status['receipt'] is None
        assert status['session_id'] == case.session
        operation = str(uuid.uuid4())
        selected = ['--session-id',case.session,'--operation-id',operation,'--expected-revision','0']
        preview = admin(case, 'preview', *selected)
        assert preview['binding'] == status['binding'] and not preview['already_withdrawn']
        admin(case, 'withdraw', *selected, '--confirm', 'incorrect', success=False)
        assert not admin(case, 'inspect')['withdrawn']
        receipt_identity = None
        if mode not in ['idle','lost-ack']:
            if mode == 'late':
                case.http.RequestHandlerClass = LateProvider
            receipt_identity, expiry = str(uuid.uuid4()), int(time.time()*1000)+120000
            request = {'type':'submit','session_id':case.session,'expected_revision':0,'prompt':'remote-gate:'+case.mode}
            reply = case.command(request, receipt_identity, expiry)['reply']
            assert reply['type'] == 'execution_snapshot', reply
            run_id = reply['run']['run_id']
            if mode == 'owned':
                wait(lambda: (case.workspace/'effects').exists(), 'owned effect before withdrawal')
                assert (case.workspace/'effects').read_text() == 'effect\n'
            else:
                assert case.hold.wait(15), case.failures
        if mode == 'crash':
            case.worker.kill(); case.worker.wait(10)
        # These reads happen at a held provider/effect boundary, not during checkpoints.
        with sqlite3.connect(case.database) as db:
            old_public = db.execute('SELECT sequence,event FROM remote_events ORDER BY sequence').fetchall()
            old_commands = db.execute('SELECT id,digest,run_id FROM commands ORDER BY id').fetchall()
        before_count = len(case.requests)
        if mode == 'busy-settled':
            original = ('withdraw', *selected, '--confirm', preview['confirmation_digest'])
            with sqlite3.connect(case.database) as locked:
                locked.execute('BEGIN IMMEDIATE')
                busy = admin(case, *original, success=False)
                assert busy.stderr.strip() == 'Error: remote consent store busy; preserve the request and explicitly retry the identical operation', busy.stderr
                locked.rollback()
            case.admin_busy = getattr(case, 'admin_busy', []) + [original]
            print('explicit Busy receipt recovery after authenticated transport loss', provider, flush=True)
            connection = case.connected()
            assert case.request('/v2/enrollment/revoke', {'machine_id':case.machine, 'expected_epoch':connection['epoch'], 'transaction_id':str(uuid.uuid4())})['revoked']
            case.worker.wait(25)
            assert case.worker.returncode != 0
            settled_status = admin(case, 'inspect')
            assert not settled_status['withdrawn'] and settled_status['run']['run_id'] == run_id, settled_status
            assert settled_status['run']['state'] == 'cancelled' and settled_status['run']['cleanup'] == 'observed', settled_status
        if mode == 'lost-ack':
            for attempt in range(5):
                reader, writer = os.pipe(); os.close(reader)
                try:
                    child = subprocess.Popen([str(HELM),'remote-consent','--directory',str(root/'managed'),'withdraw',*selected,'--confirm',preview['confirmation_digest']],env=case.env,stdout=writer,stderr=subprocess.PIPE,text=True)
                finally:
                    os.close(writer)
                _, error = child.communicate(timeout=10)
                assert child.returncode != 0, 'no stdout reader can acknowledge this commit'
                if error.strip() == 'Error: remote consent store busy; preserve the request and explicitly retry the identical operation':
                    print('explicit identical lost-ack retry after typed Busy',attempt+1,flush=True)
                    time.sleep(.05)
                    continue
                break
            assert admin(case,'inspect')['withdrawn'], error
        receipt = admin(case, 'withdraw', *selected, '--confirm', preview['confirmation_digest'])
        assert receipt['request'] == preview['request'] and receipt['revision'] == 1
        expected_cancel = None if mode in ['idle','lost-ack'] else run_id
        if receipt['cancellation_requested'] != expected_cancel:
            with sqlite3.connect(case.database) as db:
                diagnostic = {'receipt': receipt, 'runs': db.execute('SELECT record FROM runs').fetchall(),
                              'cleanup': db.execute('SELECT run_id,confirmation FROM local_cleanup_obligations').fetchall(),
                              'cancellation': db.execute('SELECT * FROM local_cancel_intents').fetchall(),
                              'busy': getattr(case, 'admin_busy', [])}
            print('withdrawal found an already settled run', json.dumps(diagnostic), flush=True)
            # An unsuccessful external commit may make the connected worker lose
            # authority and finish cancellation before the explicit identical retry.
            # Prove that disposition; never turn an absent target into a stop claim.
            assert mode in ['held','late','owned','busy-settled'] and receipt['cancellation_requested'] is None, diagnostic
            assert any(args[0] == 'withdraw' for args in diagnostic['busy']), diagnostic
            assert len(diagnostic['runs']) == 1, diagnostic
            settled = json.loads(diagnostic['runs'][0][0])
            assert settled['id'] == run_id and settled['state'] == 'cancelled', diagnostic
            assert settled['terminal_reason'] == 'run cancelled' and not settled['final_checkpointed'], diagnostic
            assert diagnostic['cleanup'] == [(run_id, 'observed')], diagnostic
            assert diagnostic['cancellation'] == [], diagnostic
            assert admin(case, 'inspect')['receipt'] == receipt, diagnostic
        # It acknowledges withdrawal, never cleanup. Output released afterward cannot dispatch tools.
        assert 'stopped' not in json.dumps(receipt) and 'observed' not in json.dumps(receipt)
        case.release.set()
        case.worker.wait(20)
        assert case.worker.returncode != 0, 'worker must report lost dedicated authority'
        wait(lambda: not case.request('/v1/diagnostics')['connections'], 'withdrawn worker disconnected')
        assert len(case.requests) == before_count and not case.failures, case.failures
        assert not (case.workspace/'late-effect').exists()
        current = admin(case, 'inspect')
        assert current['withdrawn'] and current['revision'] == 1 and current['receipt'] == receipt
        if mode == 'crash':
            assert current['pending_cleanup_run'] == run_id
            recovered = subprocess.run([str(HELM),'remote-worker','--directory',str(root/'managed'),'--recover'],env=case.env,capture_output=True,text=True,timeout=10)
            assert recovered.returncode == 0, recovered.stderr
            assert admin(case,'inspect')['pending_cleanup_run'] == run_id, 'recovery must not imply stopped effects'
        with sqlite3.connect(case.database) as db:
            published = db.execute('SELECT sequence,event FROM remote_events ORDER BY sequence').fetchall()
            assert published[:len(old_public)] == old_public, 'withdrawal rewrote prior events'
            assert all(cursor <= receipt['last_public_cursor'] for cursor,_ in published), 'publication crossed withdrawal commit'
            assert db.execute('SELECT next_sequence-1 FROM remote_session').fetchone() == (receipt['last_public_cursor'],)
            old_public = published
            assert db.execute('SELECT id,digest,run_id FROM commands ORDER BY id').fetchall() == old_commands
            before_history = db.execute('SELECT state FROM sessions').fetchone()[0]
            if mode not in ['idle','lost-ack']:
                record = json.loads(db.execute('SELECT record FROM runs WHERE id=?', (run_id,)).fetchone()[0])
                assert record['state'] == ('interrupted' if mode == 'crash' else 'cancelled'), record
                assert db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?', (run_id,)).fetchone() == ((None,) if mode == 'crash' else ('observed',))
        if mode == 'crash':
            attested = subprocess.run([str(HELM),'remote-worker','--directory',str(root/'managed'),'--recover','--acknowledge-cleanup',run_id],env=case.env,capture_output=True,text=True,timeout=10)
            assert attested.returncode == 0, attested.stderr
            current = admin(case,'inspect')
            assert current['receipt'] == receipt
            with sqlite3.connect(case.database) as db:
                assert db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?',(run_id,)).fetchone() == ('operator_attested',)
        assert current['pending_cleanup_run'] is None
        assert admin(case, 'withdraw', *selected, '--confirm', preview['confirmation_digest']) == receipt
        assert admin(case, 'preview', *selected)['already_withdrawn']
        changed = list(selected); changed[changed.index(operation)] = str(uuid.uuid4())
        admin(case, 'withdraw', *changed, '--confirm', preview['confirmation_digest'], success=False)
        # Restart never regrants, submits a provider request, changes canonical history or reconnects.
        case.worker = case.spawn(case.worker_command)
        case.worker.wait(15)
        assert case.worker.returncode != 0
        assert not case.request('/v1/diagnostics')['connections']
        assert len(case.requests) == before_count
        with sqlite3.connect(case.database) as db:
            assert db.execute('SELECT state FROM sessions').fetchone()[0] == before_history
            assert db.execute('SELECT sequence,event FROM remote_events ORDER BY sequence').fetchall() == old_public
        # Current authenticated routing no longer has a worker; old Vessel cache is not a new grant.
        case.request(case.base+'/command', {'command_id':str(uuid.uuid4()),'expires_at_ms':int(time.time()*1000)+120000,'operation':{'type':'inspect','session_id':case.session}}, expected=404)
        if mode == 'owned':
            assert (case.workspace/'effects').read_text() == 'effect\n'
            with sqlite3.connect(case.database) as db:
                revision = db.execute('SELECT revision FROM sessions').fetchone()[0]
            command=[str(HELM),'remote-worker','--directory',str(root/'managed'),'--recover','--reconcile-tools',run_id,'--expected-revision',str(revision)]
            recovered=subprocess.run(command,env=case.env,capture_output=True,text=True,timeout=10)
            assert recovered.returncode == 0, recovered.stderr
            assert 'unknown' in case.canonical()['messages'][-1]['content'].lower()
            assert (case.workspace/'effects').read_text() == 'effect\n'
            assert admin(case,'inspect')['receipt'] == receipt
        if mode == 'idle':
            # A deliberate new empty installation permits future work, never adopts retired history.
            fresh = list(case.worker_command)
            fresh[fresh.index('--directory')+1] = str(root/'future-managed')
            case.http.RequestHandlerClass = FreshProvider
            case.worker = case.spawn(fresh)
            wait(case.connected, 'new explicit empty grant')
            listed = case.command({'type':'list','after':None,'limit':20})['reply']['sessions']
            assert len(listed) == 1 and listed[0]['id'] != case.session and listed[0]['revision'] == 0
            denied = case.command({'type':'inspect','session_id':case.session})['reply']
            assert denied == {'type':'denied','code':'unauthorized'}, denied
            case.session = listed[0]['id']
            accepted = case.command({'type':'submit','session_id':case.session,'expected_revision':0,'prompt':'fresh explicit work'})['reply']
            assert accepted['type'] == 'execution_snapshot', accepted
            completed = wait(case.terminal, 'new grant executes useful work')
            assert completed['run']['state'] == 'completed'
            assert len(case.requests) == before_count+1
            with sqlite3.connect(root/'future-managed/journal/journal.sqlite3') as db:
                saved = json.loads(db.execute('SELECT state FROM sessions').fetchone()[0])
                assert [m['content'] for m in saved['messages']] == ['fresh explicit work','NEW_EMPTY_GRANT']
            assert admin(case,'inspect')['receipt'] == receipt
        print('remote withdrawal',provider,mode,'passed',flush=True)
        return mode in ['held','late'] and receipt['cancellation_requested'] is not None
    finally:
        case.close()


def main():
    with tempfile.TemporaryDirectory(prefix='voyage-withdrawal-') as temporary:
        for provider in ['openai-chat','openai-responses','anthropic']:
            live_cancellation = []
            for mode in ['idle','held','late','owned','crash']:
                live_cancellation.append(execute(Path(temporary)/(provider+'-'+mode),provider,mode))
            assert any(live_cancellation), 'each native provider must prove a real held-provider cancellation target'
        execute(Path(temporary)/'lost-ack','openai-chat','lost-ack')
        execute(Path(temporary)/'busy-settled','openai-chat','busy-settled')


if __name__ == '__main__':
    main()
