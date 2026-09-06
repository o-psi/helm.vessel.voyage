#!/usr/bin/env python3
"""Real outbound pending-control workflow, isolated synthetic enrollment, no providers."""
import concurrent.futures
import fcntl
import http.client
import json
import os
import pathlib
import queue
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
HELM = pathlib.Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()
VESSEL = pathlib.Path(os.environ.get('VESSEL_BIN', ROOT / 'target/release/vessel')).resolve()
OLD_HELM = os.environ.get('PRE_CONTROL_HELM_BIN')
OLD_VESSEL = os.environ.get('PRE_CONTROL_VESSEL_BIN')
TOKEN = 'coordination-fixture-operator-token-at-least-32-bytes'
CANARY = 'private-session-draft-provider-state-must-not-be-published'


def main():
    with tempfile.TemporaryDirectory(prefix='voyage-control-') as temporary:
        root = pathlib.Path(temporary)
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_CONFIG_HOME=str(root / 'config'), VESSEL_OPERATOR_TOKEN=TOKEN, RUST_LOG='trace')
        invalid = root / 'invalid.toml'
        invalid.write_text('provider = [ invalid TOML')
        with socket.socket() as allocated:
            allocated.bind(('127.0.0.1', 0))
            port = allocated.getsockname()[1]
        address = f'127.0.0.1:{port}'
        origin = 'http://' + address
        children, secrets, notices = [], [TOKEN, CANARY], []
        log = tempfile.TemporaryFile()
        sessions = root / 'sessions'
        sessions.mkdir(mode=0o700)
        installation = root / 'installation'
        session_id = str(uuid.uuid4())
        session = dict(id=session_id, revision=7, created_at='2026-09-05T00:00:00Z',
                       updated_at='2026-09-05T00:00:00Z', workspace=str(root), model=CANARY,
                       name=CANARY, draft=CANARY,
                       messages=[dict(role='user', content=CANARY, provider_state={'canary': CANARY})],
                       usage=dict(input_tokens=0, output_tokens=0))
        source = sessions / (session_id + '.json')
        source.write_text(json.dumps(session))
        source.chmod(0o600)
        original = source.read_bytes()

        def safe(text):
            for secret in secrets:
                assert secret not in text, 'private value escaped output'

        def request(path, body=None, expected=200, authenticated=True, headers=None):
            fields = dict(headers or {})
            if authenticated:
                fields['Authorization'] = 'Bearer ' + TOKEN
            if body is not None:
                fields.update({'Content-Type': 'application/json', 'x-voyage-request': '2'})
            req = urllib.request.Request(origin + path, headers=fields,
                data=None if body is None else json.dumps(body).encode())
            try:
                response = urllib.request.urlopen(req, timeout=8)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                payload = response.read().decode()
                assert response.status == expected, (path, response.status, expected, payload)
            safe(payload)
            return json.loads(payload) if payload and expected==200 else payload

        def start_server(binary=VESSEL, enabled=True):
            command = [str(binary), '--bind', address, '--database', str(root / 'vessel.db'),
                       '--attachment-directory', str(root / 'authority'), '--public-origin', origin + '/',
                       '--allow-insecure-loopback']
            if enabled:
                command += ['--coordination-control']
            process = subprocess.Popen(command, env=env, stdout=log, stderr=log)
            children.append(process)
            deadline = time.monotonic() + 10
            while True:
                assert process.poll() is None, 'Vessel startup failed'
                try:
                    request('/ready', authenticated=False)
                    return process
                except (OSError, urllib.error.URLError):
                    assert time.monotonic() < deadline, 'Vessel startup timed out'
                    time.sleep(.03)

        def command(identity, *args, binary=HELM):
            return [str(binary), '--config', str(invalid), '--set', 'provider=not-a-provider',
                    'attachment', '--directory', str(identity), '--allow-insecure-loopback', *map(str, args)]

        def cli(identity, *args, expected=0, key=None, binary=HELM):
            result = subprocess.run(command(identity, *args, binary=binary), input=key,
                capture_output=True, text=True, env=env, timeout=12)
            safe(result.stdout + result.stderr)
            assert result.returncode == expected, (args, result.returncode, result.stderr)
            return [json.loads(line) for line in result.stdout.splitlines()]

        def enroll(identity):
            req = urllib.request.Request(origin + '/v2/enrollment/invitations', data=b'{"ttl_ms":60000}',
                headers={'Authorization': 'Bearer ' + TOKEN, 'Content-Type': 'application/json', 'x-voyage-request': '2'})
            with urllib.request.urlopen(req, timeout=5) as response:
                invitation = json.load(response)
            secrets.append(invitation['key'])
            return cli(identity, '--origin', origin, 'enroll', '--invitation-id', invitation['id'],
                       '--invitation-key-stdin', key=invitation['key'] + '\n')[0]

        def host(identity, *extra, binary=HELM, coordination=True, stdout=subprocess.PIPE):
            args = ['connect'] + (['--coordination'] if coordination else []) + list(extra)
            process = subprocess.Popen(command(identity, *args, binary=binary), stdin=subprocess.DEVNULL,
                stdout=stdout, stderr=subprocess.PIPE, text=True, env=env)
            children.append(process)
            process.messages = queue.Queue()
            if stdout == subprocess.PIPE:
                def drain():
                    for line in process.stdout:
                        notices.append(line)
                        process.messages.put(line)
                    process.messages.put(None)
                threading.Thread(target=drain, daemon=True).start()
            return process

        def notice(process, event='coordination_control', timeout=10):
            deadline = time.monotonic() + timeout
            while True:
                line = process.messages.get(timeout=max(.01, deadline - time.monotonic()))
                assert line is not None, ('host exited', process.poll(), process.stderr.read())
                safe(line)
                value = json.loads(line)
                if value.get('event') == event:
                    return value

        def finish(process, expected, sent_signal=None):
            if sent_signal is not None:
                process.send_signal(sent_signal)
            process.wait(timeout=8)
            safe(process.stderr.read())
            assert process.returncode == expected, (process.returncode, expected)

        def inspect_path(record):
            a = record['address']
            return f'/v2/coordination/{a["installation_id"]}/{a["session_id"]}'

        def wait_leases(path, count, revision=None):
            deadline = time.monotonic() + 10
            while True:
                value = request(path, headers={'Origin': origin})
                if len(value['control_leases']) == count and (revision is None or value['record']['revision'] == revision):
                    return value
                assert time.monotonic() < deadline, 'control lease observation timed out'
                time.sleep(.05)

        def mutation(record, operation):
            return dict(command_id=str(uuid.uuid4()), expires_at_ms=int(time.time()*1000)+60000,
                        address=record['address'], expected_revision=record['revision'], operation=operation)

        def source_args(selected=session_id):
            return ['--session', selected, '--session-directory', str(sessions),
                    '--installation-directory', str(installation)]

        def confirmation(preview):
            op = preview['metadata']['operation']
            return ['--registration-id', op['command_id'], '--registration-expires-at-ms', str(op['expires_at_ms']),
                    '--confirm-registration', preview['confirmation']]

        try:
            server = start_server()
            assert request('/v1/diagnostics')['coordination_control'] == 'configured'
            identities = [root / name for name in ['source', 'participant', 'outsider', 'old-peer']]
            infos = [enroll(identity) for identity in identities]
            a, b, c, legacy = identities
            if OLD_HELM:
                old = host(legacy, binary=OLD_HELM, coordination=False)
                assert notice(old, 'attachment_connected')['mode'] == 'presence_only'
                finish(old, 130, signal.SIGINT)

            # Source ownership fails before identity initialization or publication.
            lock_path = sessions / ('.' + session_id + '.lock')
            with open(lock_path, 'w') as lock:
                lock_path.chmod(0o600)
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                cli(a, 'connect', '--coordination', *source_args(), expected=1)
                assert not installation.exists()
            preview = cli(a, 'connect', '--coordination', *source_args())[0]
            assert preview['status'] == 'confirmation_required'
            intent_before = (installation / 'coordination-intents.json').read_bytes()
            assert CANARY.encode() not in intent_before
            assert source.read_bytes() == original
            record_address = preview['metadata']['operation']['address']
            path = inspect_path({'address': record_address})
            request(path, expected=403)
            cli(a, 'connect', '--coordination', *source_args(), '--registration-id', str(uuid.uuid4()), expected=1)
            assert (installation / 'coordination-intents.json').read_bytes() == intent_before

            source_host = host(a, *source_args(), *confirmation(preview))
            registered = notice(source_host, 'coordination_registered')['reply']
            assert not registered['duplicate']
            record = registered['record']
            assert record['execution_authority'] == 'none' and record['status'] == 'configured'
            first = wait_leases(path, 1, 1)
            generation = first['control_leases'][0]['server_generation']
            assert source.read_bytes() == original
            # The original owner was released; publishing metadata never retains execution ownership.
            with open(lock_path) as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            outsider = host(c)
            assert notice(outsider)['views'] == []
            participant = host(b)
            assert notice(participant)['views'] == []
            binding = lambda index: dict(machine_id=infos[index]['machine_id'], epoch=infos[index]['epoch'])
            configure = mutation(record, dict(type='configure', coordinator=binding(1), participants=[binding(0), binding(1)]))
            request('/v2/coordination/command', configure, authenticated=False, expected=401)
            request('/v2/coordination/command', configure, headers={'Origin': 'https://foreign.invalid'}, expected=403)
            # Duplicate origin/request/auth headers retain existing strict operator boundaries.
            for duplicate, value in [('Origin', origin), ('x-voyage-request', '2'), ('Authorization', 'Bearer '+TOKEN)]:
                peer = http.client.HTTPConnection(address, timeout=5)
                peer.putrequest('POST', '/v2/coordination/command')
                fields = [('Authorization','Bearer '+TOKEN), ('Origin',origin), ('x-voyage-request','2'),
                          ('Content-Type','application/json'), (duplicate,value)]
                data = json.dumps(configure)
                for key, item in fields:
                    peer.putheader(key,item)
                peer.putheader('Content-Length',str(len(data)))
                peer.endheaders(data.encode())
                response=peer.getresponse()
                assert response.status in (401,403), (duplicate,response.status)
                safe(response.read().decode())
                peer.close()
            result = request('/v2/coordination/command', configure, headers={'Origin': origin})
            record = result['record']
            assert record['revision'] == 2 and record['coordinator_epoch'] == 2
            current = wait_leases(path, 2, 2)
            assert all(lease['record_revision'] == 2 for lease in current['control_leases'])
            assert any(lease['coordinator'] and lease['machine'] == binding(1) for lease in current['control_leases'])
            # Competing exact revisions admit one transaction; immutable retry observes its original receipt.
            contenders = [mutation(record, configure['operation']) for _ in range(2)]
            def submit(body):
                req = urllib.request.Request(origin+'/v2/coordination/command',data=json.dumps(body).encode(),headers={
                    'Authorization':'Bearer '+TOKEN,'Content-Type':'application/json','x-voyage-request':'2'})
                try:
                    response=urllib.request.urlopen(req,timeout=8)
                except urllib.error.HTTPError as error:
                    response=error
                with response:
                    text=response.read().decode();safe(text)
                    return response.status,json.loads(text) if response.status==200 else None
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                results=list(pool.map(submit,contenders))
            assert sorted(status for status,_ in results)==[200,409]
            record=next(body['record'] for status,body in results if status==200)
            retried=request('/v2/coordination/command',configure)
            assert retried['duplicate'] and retried['record']['revision']==2
            assert request(path)['record']['revision']==3
            altered=dict(configure,expected_revision=3)
            request('/v2/coordination/command',altered,expected=409)

            # Repeat registration after local edits uses original durable metadata, not current revision.
            finish(source_host,130,signal.SIGINT)
            session['revision']=8
            source.write_text(json.dumps(session));edited=source.read_bytes()
            retry_preview=cli(a,'connect','--coordination',*source_args())[0]
            assert retry_preview['metadata']==preview['metadata'] and retry_preview['confirmation']==preview['confirmation']
            source_host=host(a,*source_args(),*confirmation(retry_preview))
            historical=notice(source_host,'coordination_registered')['reply']
            assert historical['duplicate'] and historical['record']['source_revision']==7 and historical['record']['revision']==1
            assert source.read_bytes()==edited and (installation/'coordination-intents.json').read_bytes()==intent_before
            wait_leases(path,2,3)
            time.sleep(11)
            assert source_host.poll() is None and participant.poll() is None
            assert len(request(path)['control_leases'])==2

            # Restart invalidates persisted leases; surviving enrollment reconnects with a new generation.
            server.send_signal(signal.SIGTERM);server.wait(timeout=8)
            for process in [source_host,participant,outsider]:
                finish(process,1)
            server=start_server()
            assert request(path)['control_leases']==[]
            participant=host(b)
            resumed=notice(participant)
            assert resumed['views'][0]['lease']['server_generation']>generation
            wait_leases(path,1,3)
            revoked=request('/v2/coordination/command',mutation(record,dict(type='revoke')))['record']
            assert revoked['status']=='revoked' and revoked['revision']==4
            assert request(path)['control_leases']==[]
            notice(participant)
            assert request(path)['control_leases']==[]
            finish(participant,130,signal.SIGINT)

            # Expired unaccepted intent is observed before an explicit fresh preview.
            second_id=str(uuid.uuid4())
            second=dict(session,id=second_id,revision=1)
            second_path=sessions/(second_id+'.json')
            second_path.write_text(json.dumps(second));second_path.chmod(0o600)
            expired=cli(c,'connect','--coordination',*source_args(second_id),
                        '--registration-expires-at-ms',str(int(time.time()*1000)-1))[0]
            absent=cli(c,'connect','--coordination',*source_args(second_id),*confirmation(expired),expected=1)
            assert absent[0]['reply']['type']=='not_registered'
            fresh=cli(c,'connect','--coordination',*source_args(second_id),'--new-registration')[0]
            assert fresh['metadata']['operation']['command_id']!=expired['metadata']['operation']['command_id']
            recovered=host(c,*source_args(second_id),*confirmation(fresh))
            second_record=notice(recovered,'coordination_registered')['reply']['record']
            second_route=inspect_path(second_record)
            wait_leases(second_route,1,1)
            finish(recovered,130,signal.SIGINT)
            saved_journal=(installation/'coordination-intents.json').read_bytes()
            rotated=cli(c,'rotate')[0]
            assert rotated['epoch']==infos[2]['epoch']+1
            assert request(second_route)['control_leases']==[]
            new_binding=dict(machine_id=rotated['machine_id'],epoch=rotated['epoch'])
            rebind=mutation(second_record,dict(type='rebind_source',expected_source=second_record['source'],source=new_binding))
            wrong=dict(rebind,operation=dict(rebind['operation'],expected_source=binding(0)))
            request('/v2/coordination/command',wrong,expected=409)
            rebound=request('/v2/coordination/command',rebind)['record']
            assert rebound['address']==second_record['address'] and rebound['coordinator']==second_record['coordinator']
            assert request('/v2/coordination/command',rebind)['duplicate']
            recovered=host(c,*source_args(second_id),*confirmation(fresh))
            observed=notice(recovered,'coordination_registered')['reply']
            assert observed['duplicate'] and observed['record']==second_record
            nominated=notice(recovered)
            assert nominated['views'][0]['lease'] is None
            assert request(second_route)['control_leases']==[]
            configured=request('/v2/coordination/command',mutation(rebound,dict(type='configure',coordinator=new_binding,participants=[new_binding])))['record']
            wait_leases(second_route,1,configured['revision'])
            assert (installation/'coordination-intents.json').read_bytes()==saved_journal
            finish(recovered,130,signal.SIGINT)

            # A blocked output cannot retain the enrollment or a live control lease.
            read_fd,write_fd=os.pipe()
            try:
                os.set_blocking(write_fd,False)
                while True:
                    try: os.write(write_fd,b'x'*4096)
                    except BlockingIOError: break
                os.set_blocking(write_fd,True)
                blocked=host(c,stdout=write_fd)
                os.close(write_fd);write_fd=None
                finish(blocked,1)
                wait_leases(second_route,0)
                assert cli(c,'status')[0]['status']=='active'
            finally:
                os.close(read_fd)
                if write_fd is not None: os.close(write_fd)
            # Busy SQLite is bounded and does not turn a failed mutation into a new revision.
            database=root/'authority/enrollment.sqlite3'
            blocked_mutation=mutation(configured,dict(type='revoke'))
            with sqlite3.connect(database) as held:
                held.execute('BEGIN IMMEDIATE')
                started=time.monotonic()
                request('/v2/coordination/command',blocked_mutation,expected=503)
                assert time.monotonic()-started<8
                held.rollback()
            assert request(second_route)['record']==configured

            # Preserve corrupt intent bytes and source history instead of inventing a replacement.
            journal=installation/'coordination-intents.json'
            journal.write_bytes(b'corrupt intent evidence')
            cli(a,'connect','--coordination',*source_args(),expected=1)
            assert journal.read_bytes()==b'corrupt intent evidence' and source.read_bytes()==edited
            journal.write_bytes(intent_before)

            # A control client refuses an old server that cannot negotiate the feature.
            server.send_signal(signal.SIGTERM);server.wait(timeout=8)
            if OLD_VESSEL:
                server=start_server(binary=OLD_VESSEL,enabled=False)
                cli(c,'connect','--coordination',expected=1)
                server.send_signal(signal.SIGTERM);server.wait(timeout=8)
            # Unknown control schema refuses startup while preserving the existing database.
            with sqlite3.connect(database) as corrupt:
                corrupt.execute('UPDATE coordination_schema SET version=999')
            before_database=database.read_bytes()
            refused=subprocess.Popen([str(VESSEL),'--bind',address,'--database',str(root/'vessel.db'),
                '--attachment-directory',str(root/'authority'),'--public-origin',origin,
                '--allow-insecure-loopback','--coordination-control'],env=env,stdout=log,stderr=log)
            children.append(refused);refused.wait(timeout=10)
            assert refused.returncode!=0 and database.read_bytes()==before_database
            for line in notices:
                safe(line)
            log.seek(0);safe(log.read().decode(errors='replace'))
        finally:
            for process in reversed(children):
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)
            log.close()
    compatibility='both saved pre-control peers verified' if OLD_HELM and OLD_VESSEL else 'saved pre-control peer checks not requested'
    print('coordination control: registration, private retry, roles, CAS, expiry, restart, origin/auth and revocation passed; '+compatibility)


if __name__=='__main__':
    main()
