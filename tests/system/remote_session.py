#!/usr/bin/env python3
"""Actual opt-in Vessel/Helm execution with isolated native HTTP and private storage."""
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.request
import urllib.error
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import response, event

ROOT=Path(__file__).resolve().parents[2]
HELM=Path(os.environ.get('HELM_BIN',ROOT/'target/release/helm')).resolve()
VESSEL=Path(os.environ.get('VESSEL_BIN',ROOT/'target/release/vessel')).resolve()
TOKEN='remote-fixture-operator-only-credential-32-bytes'
KEY='remote-fixture-provider-only-credential-32-bytes'

def port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0))
        return sock.getsockname()[1]

def wait(check, description, seconds=20):
    deadline=time.monotonic()+seconds
    while True:
        result=check()
        if result:
            return result
        assert time.monotonic()<deadline,description
        time.sleep(.05)

def split_response(provider,text):
    if provider=='openai-chat':
        return b''.join(event({'choices':[{'delta':{'content':c}}]}) for c in text)+b'data: [DONE]\n\n'
    if provider=='openai-responses':
        return b''.join(event({'type':'response.output_text.delta','delta':c}) for c in text)+response(provider,text,999)
    return event({'type':'message_start','message':{'usage':{'input_tokens':1}}})+b''.join(event({'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':c}}) for c in text)+event({'type':'message_delta','usage':{'output_tokens':1}})+event({'type':'message_stop'})

class Provider(BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def do_POST(self):
        state=self.server.state
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        with state['lock']:
            state['requests'].append(body)
            step=len(state['requests'])
            hold=state['hold']
        if hold:
            state['started'].set()
            state['release'].wait(30)
            return
        if step==1:
            # Check the durable obligation before the first effect-producing provider byte.
            with sqlite3.connect(state['database']) as db:
                row=db.execute('SELECT r.active,o.confirmation FROM runs r JOIN local_cleanup_obligations o ON o.run_id=r.id WHERE active=1').fetchall()
            assert row==[(1,None)],row
            value=('write_file',{'path':'remote-effect.txt','content':'exactly-once remote effect'})
        else:
            value=f'Remote {KEY} 秘密🔐canary work finished.'
        data=response(state['provider'],value,step) if step==1 else split_response(state['provider'],value)
        try:
            self.send_response(200)
            self.send_header('Content-Type','text/event-stream')
            self.send_header('Content-Length',str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except (BrokenPipeError,ConnectionResetError):pass


def case(root,provider,profile=False,shutdown_failure=False):
    root.mkdir()
    workspace=root/'workspace';workspace.mkdir()
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_CONFIG_HOME=str(root/'config'),VESSEL_OPERATOR_TOKEN=TOKEN,REMOTE_PROVIDER_KEY=KEY,RUST_LOG='warn')
    origin=f'http://127.0.0.1:{port()}'
    state={'provider':provider,'requests':[],'lock':threading.Lock(),'hold':False,'started':threading.Event(),'release':threading.Event(),'database':root/'managed/journal/journal.sqlite3'}
    http=ThreadingHTTPServer(('127.0.0.1',0),Provider);http.daemon_threads=True;http.state=state
    thread=threading.Thread(target=http.serve_forever,daemon=True);thread.start()
    config=root/'provider.toml'
    config.write_text(f'provider = "{provider}"\nmodel = "remote-fixture"\nbase_url = "http://127.0.0.1:{http.server_port}/v1"\napi_key_env = "REMOTE_PROVIDER_KEY"\naccess = "unrestricted"\nmax_tokens = 256\nprovider_retry_attempts = 1\nredact_values = ["秘密🔐canary"]\n')
    children=[];logs=[]
    def spawn(command):
        log=tempfile.TemporaryFile();logs.append(log)
        process=subprocess.Popen(command,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=log)
        children.append(process);return process
    def request(path,body=None,authenticated=True,expected=200,extra=None):
        headers={'x-voyage-request':'2',**(extra or {})}
        if authenticated:headers['Authorization']='Bearer '+TOKEN
        if body is not None:headers['Content-Type']='application/json'
        req=urllib.request.Request(origin+path,headers=headers,data=None if body is None else json.dumps(body).encode())
        try:res=urllib.request.urlopen(req,timeout=8)
        except urllib.error.HTTPError as error:res=error
        with res:
            payload=res.read().decode()
            if path.startswith('/v1/remote/') and res.status==200:
                assert res.headers.get('Cache-Control')=='no-store', 'remote content cacheable'
            assert res.status in (expected if isinstance(expected,tuple) else (expected,)),(path,res.status,payload)
        assert KEY not in payload and TOKEN not in payload,'credential exported'
        return json.loads(payload) if payload.startswith(('{','[')) else payload
    def ready():
        try:return request('/ready',authenticated=False)
        except (OSError,urllib.error.URLError):return None
    try:
        server=spawn([str(VESSEL),'--bind',origin.removeprefix('http://'),'--database',str(root/'vessel.db'),'--attachment-directory',str(root/'authority'),'--public-origin',origin,'--allow-insecure-loopback','--remote-execution'])
        wait(ready,'server startup')
        invitation=request('/v2/enrollment/invitations',{'ttl_ms':60000})
        enrolled=subprocess.run([str(HELM),'attachment','--directory',str(root/'enrollment'),'--origin',origin,'--allow-insecure-loopback','enroll','--invitation-id',invitation['id'],'--invitation-key-stdin'],input=invitation['key']+'\n',text=True,capture_output=True,env=env,timeout=10)
        assert enrolled.returncode==0,(enrolled.stdout,enrolled.stderr)
        selection=[]
        if profile:
            common=[str(HELM),'--config',str(config),'--workspace',str(workspace),'--policy-directory',str(root/'profiles')]
            def profile_command(*arguments):
                result=subprocess.run([*common,'policy',*arguments],env=env,capture_output=True,text=True,timeout=10)
                assert result.returncode==0,(result.stdout,result.stderr)
                return json.loads(result.stdout)
            profile_command('create','remote-review','--preset','restricted')
            selected=profile_command('inspect','remote-review')
            selection=['--policy-directory',str(root/'profiles'),'--policy-profile','remote-review','--policy-revision','1','--policy-digest',selected['digest']]
        worker_command=[str(HELM),'--config',str(config),'--workspace',str(workspace),*selection,'remote-worker','--directory',str(root/'managed'),'--enrollment-directory',str(root/'enrollment'),'--origin',origin,'--allow-insecure-loopback']
        worker=spawn(worker_command)
        def connected():
            assert worker.poll() is None,'worker stopped'
            entries=request('/v1/diagnostics')['connections']
            return entries[0] if entries else None
        connection=wait(connected,'worker connection')
        machine=connection['machine_id']
        base=f'/v1/remote/{machine}'
        def command(operation,identity=None,expiry=None,**kwargs):
            return request(base+'/command',{'command_id':identity or str(uuid.uuid4()),'expires_at_ms':expiry or int(time.time()*1000)+120000,'operation':operation},**kwargs)
        request(base+'/command',{},authenticated=False,expected=401)
        command({'type':'list','after':None,'limit':20},extra={'Origin':'https://hostile.invalid'},expected=403)
        listed=command({'type':'list','after':None,'limit':20})['reply']['sessions']
        assert len(listed)==1 and listed[0]['revision']==0,listed
        session=listed[0]['id']
        unknown=command({'type':'inspect','session_id':str(uuid.uuid4())})
        assert unknown['reply']=={'type':'denied','code':'unauthorized'},unknown
        command({'type':'create','workspace_id':str(uuid.uuid4()),'name':'not allowed'},expected=403)
        identity=str(uuid.uuid4());expiry=int(time.time()*1000)+120000
        operation={'type':'submit','session_id':session,'expected_revision':0,'prompt':'Write the remote effect file then finish.'}
        accepted=command(operation,identity,expiry)['reply']
        assert accepted['type']=='execution_snapshot',accepted
        run_id=accepted['run']['run_id']
        def terminal():
            snapshot=command({'type':'inspect','session_id':session})['reply']
            return snapshot if snapshot.get('run',{}).get('cleanup')=='observed' else None
        completed=wait(terminal,'completed observed cleanup')
        assert completed['run']['state']=='completed',completed
        if shutdown_failure:
            state['hold']=True
            held=command({'type':'submit','session_id':session,'expected_revision':completed['session']['revision'],'prompt':'Hold until explicit worker shutdown.'})['reply']['run']['run_id']
            assert state['started'].wait(10),'provider did not start before shutdown'
            with sqlite3.connect(state['database']) as db:
                db.execute("CREATE TRIGGER fixture_cleanup_failure BEFORE UPDATE OF confirmation ON local_cleanup_obligations BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END")
            if shutdown_failure=='completed':
                # Terminal cleanup failure may close the lease before its cancel
                # acknowledgement is delivered. Durable Cancelled below proves
                # admission; transport uncertainty is not proof of rejection.
                reply=command({'type':'cancel','session_id':session,'run_id':held},expected=(200,409,504))
                if isinstance(reply,dict):assert reply['reply']['type']=='accepted',reply
            else:
                worker.send_signal(signal.SIGINT)
            worker.wait(25)
            assert worker.returncode!=0,'worker reported successful shutdown despite unconfirmed cleanup'
            with sqlite3.connect(state['database']) as db:
                record=json.loads(db.execute('SELECT record FROM runs WHERE id=?',(held,)).fetchone()[0])
                confirmation=db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?',(held,)).fetchone()[0]
                db.execute('DROP TRIGGER fixture_cleanup_failure')
            assert record['state']=='cancelled' and confirmation is None,(record,confirmation)
            count=len(state['requests'])
            worker=spawn(worker_command);wait(connected,'observe unconfirmed shutdown')
            snapshot=command({'type':'inspect','session_id':session})['reply']
            assert snapshot['run']['run_id']==held and snapshot['run']['cleanup']=='unconfirmed',snapshot
            denied=command({'type':'submit','session_id':session,'expected_revision':snapshot['session']['revision'],'prompt':'Cleanup blocker must survive shutdown.'})['reply']
            assert denied['type']=='denied' and len(state['requests'])==count,denied
            return
        if profile:
            assert not (workspace/'remote-effect.txt').exists(),'selected restricted profile allowed a write'
            count=len(state['requests'])
            with sqlite3.connect(state['database']) as db:
                before=db.execute('SELECT count(*) FROM runs').fetchone()[0]
            profile_command('delete','remote-review','--expected-revision','1')
            rejected=command({'type':'submit','session_id':session,'expected_revision':completed['session']['revision'],'prompt':'Stale profile must not admit.'})['reply']
            assert rejected['type']=='denied',rejected
            inspected=command({'type':'inspect','session_id':session})['reply']
            assert inspected['session']['revision']==completed['session']['revision'],inspected
            assert len(state['requests'])==count,'stale profile reached provider'
            with sqlite3.connect(state['database']) as db:
                assert db.execute('SELECT count(*) FROM runs').fetchone()[0]==before,'stale profile admitted canonical input'
            return
        assert (workspace/'remote-effect.txt').read_text()=='exactly-once remote effect'
        count=len(state['requests'])
        retry=command(operation,identity,expiry)['reply']
        assert retry['type']=='run' and retry['run_id']==run_id and retry['state']=='completed',retry
        assert len(state['requests'])==count,'duplicate effects'
        altered=dict(operation,prompt='Changed request must not reuse a receipt.')
        command(altered,identity,expiry,expected=409)
        assert len(state['requests'])==count,'altered receipt dispatched'
        # Reject admission atomically if its public outbox cannot be committed.
        with sqlite3.connect(state['database']) as db:
            before_runs=db.execute('SELECT count(*) FROM runs').fetchone()[0]
            db.execute("CREATE TRIGGER fixture_public_failure BEFORE INSERT ON remote_events BEGIN SELECT RAISE(ABORT, 'fixture public failure'); END")
        try:
            rejected=command({'type':'submit','session_id':session,'expected_revision':completed['session']['revision'],'prompt':'Must not dispatch without public receipt.'})['reply']
            assert rejected['type']=='denied',rejected
            assert len(state['requests'])==count,'failed admission dispatched provider'
            with sqlite3.connect(state['database']) as db:
                assert db.execute('SELECT count(*) FROM runs').fetchone()[0]==before_runs
            assert command({'type':'inspect','session_id':session})['reply']['session']['revision']==completed['session']['revision']
        finally:
            with sqlite3.connect(state['database']) as db:db.execute('DROP TRIGGER fixture_public_failure')
        events=request(base+f'/events?session_id={session}&after=0&limit=128')
        assert events['type']=='replay',events
        kinds=[event['event']['type'] for event in events['events']]
        assert 'tool_started' in kinds and 'tool_finished' in kinds and kinds[-1]=='cleanup',kinds
        assert events['events'][-1]['event']['state']=='observed'
        assert 'arguments' not in json.dumps(events)
        text=''.join(event['event'].get('text','') for event in events['events'])
        assert text=='Remote [REDACTED] [REDACTED] work finished.',text
        pages=[];cursor=0
        while cursor<events['latest']:
            page=request(base+f'/events?session_id={session}&after={cursor}&limit=1')
            assert len(page['events'])==1,page
            pages.extend(page['events']);cursor=pages[-1]['cursor']
        assert pages==events['events'],'page boundary changed public projection'
        with sqlite3.connect(state['database']) as db:
            stored=''.join(row[0] for row in db.execute('SELECT event FROM remote_events ORDER BY sequence'))
        assert KEY not in stored and '秘密🔐canary' not in stored,'secret entered public outbox'
        # A process restart observes the original receipt; never replays its tools.
        worker.send_signal(signal.SIGINT);worker.wait(25)
        worker=spawn(worker_command)
        newer=wait(connected,'reconnect')
        assert newer['connection_id']!=connection['connection_id']
        assert command(operation,identity,expiry)['reply']['run_id']==run_id
        assert len(state['requests'])==count
        # New exact-run cancellation while the provider is held.
        state['hold']=True
        cancel_run=command({'type':'submit','session_id':session,'expected_revision':completed['session']['revision'],'prompt':'Wait for cancellation.'})['reply']['run']['run_id']
        assert state['started'].wait(10),'provider did not start second turn'
        cancelled=command({'type':'cancel','session_id':session,'run_id':cancel_run})
        assert cancelled['reply']['type']=='accepted',cancelled
        observed=wait(terminal,'cancelled observed cleanup')
        assert observed['run']['run_id']==cancel_run and observed['run']['state']=='cancelled',observed
        state['release'].set()
        # Forced death leaves a durable cleanup obligation; only local explicit
        # recovery/attestation can clear it. Invalid provider configuration is irrelevant.
        state['started'].clear();state['release'].clear()
        crashed=command({'type':'submit','session_id':session,'expected_revision':observed['session']['revision'],'prompt':'Hold for forced death.'})['reply']['run']['run_id']
        assert state['started'].wait(10)
        invalid=root/'invalid.toml';invalid.write_text('invalid [ provider configuration')
        def recover(*extra,expected=0):
            result=subprocess.run([str(HELM),'--config',str(invalid),'--set','provider=invalid','remote-worker','--directory',str(root/'managed'),'--recover',*extra],env=env,capture_output=True,text=True,timeout=15)
            assert result.returncode==expected,(result.returncode,result.stdout,result.stderr)
            return json.loads(result.stdout) if result.stdout else None
        recover(expected=1)  # An active foreground owner cannot be recovered.
        worker.kill();worker.wait(5)
        recovered=recover()
        assert recovered['run']=={'id':crashed,'state':'interrupted'} and recovered['cleanup']=='unchanged',recovered
        recover('--acknowledge-cleanup',str(uuid.uuid4()),expected=1)
        worker=spawn(worker_command)
        wait(connected,'connection with unresolved cleanup')
        after_crash=command({'type':'inspect','session_id':session})['reply']
        assert after_crash['run']['cleanup']=='unconfirmed' and after_crash['run']['state']=='interrupted',after_crash
        denied=command({'type':'submit','session_id':session,'expected_revision':after_crash['session']['revision'],'prompt':'Must remain blocked.'})
        assert denied['reply']['type']=='denied',denied
        worker.send_signal(signal.SIGINT);worker.wait(25)
        attested=recover('--acknowledge-cleanup',crashed)
        assert attested['cleanup']=='operator_attested'
        assert recover('--acknowledge-cleanup',crashed)['cleanup']=='operator_attested'
        state['release'].set();state['hold']=False
        worker=spawn(worker_command);wait(connected,'restart after local attestation')
        restarted=command({'type':'inspect','session_id':session})['reply']
        assert restarted['run']['cleanup']=='operator_attested',restarted
        resumed=command({'type':'submit','session_id':session,'expected_revision':restarted['session']['revision'],'prompt':'Finish after explicit recovery.'})
        assert resumed['reply']['type']=='execution_snapshot',resumed
        final_snapshot=wait(terminal,'completion after explicit local recovery')
        assert final_snapshot['run']['state']=='completed'
        # Revocation closes the current lease, cancels owned work, and never
        # reauthorizes from its cached binding or from a previous command receipt.
        state['hold']=True;state['release'].clear();state['started'].clear()
        revoking=command({'type':'submit','session_id':session,'expected_revision':final_snapshot['session']['revision'],'prompt':'Wait until enrollment revocation.'})['reply']['run']['run_id']
        assert state['started'].wait(10)
        count=len(state['requests'])
        current=request('/v1/diagnostics')['connections'][0]
        revoked=request('/v2/enrollment/revoke',{'machine_id':machine,'expected_epoch':current['epoch'],'transaction_id':str(uuid.uuid4())})
        assert revoked['revoked']
        worker.wait(25)
        assert worker.returncode!=0,'revoked worker reported available'
        state['release'].set()
        with sqlite3.connect(state['database']) as db:
            record=json.loads(db.execute('SELECT record FROM runs WHERE id=?',(revoking,)).fetchone()[0])
            cleanup=db.execute('SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?',(revoking,)).fetchone()[0]
        assert record['state']=='cancelled' and cleanup=='observed',(record['state'],cleanup)
        assert len(state['requests'])==count,'revocation replayed provider work'
        request(base+'/command',{'command_id':identity,'expires_at_ms':expiry,'operation':operation},expected=404)
        assert not list((root/'data').glob('helm/sessions/*.json')),'legacy private session write'
        assert server.poll() is None
    finally:
        state['release'].set()
        for process in reversed(children):
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:process.wait(25)
                except subprocess.TimeoutExpired:process.kill();process.wait(5)
        for log in logs:
            log.seek(0);text=log.read().decode(errors='replace')
            assert KEY not in text and TOKEN not in text,'credential in binary logs'
            log.close()
        http.shutdown();http.server_close();thread.join(5)

if __name__=='__main__':
    with tempfile.TemporaryDirectory(prefix='voyage-remote-') as directory:
        for provider in ('openai-chat','openai-responses','anthropic'):
            case(Path(directory)/provider,provider)
        case(Path(directory)/'profile','openai-chat',profile=True)
        case(Path(directory)/'shutdown-failure','openai-chat',shutdown_failure=True)
        case(Path(directory)/'completed-cleanup-failure','openai-chat',shutdown_failure='completed')
    print('remote session: three native adapters, actual file effects, exact retry/restart, cancellation, forced-death recovery/attestation, revocation, private-scope denial, publication rollback, selected-profile freshness and unconfirmed-cleanup exit status passed')
