#!/usr/bin/env python3
"""Offline inference permits: real native HTTP, CLI, effects and private ledger."""
import json
import os
from pathlib import Path
import signal
import shutil
import sqlite3
import subprocess
import tempfile
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import event, normalize, response

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get('HELM_BIN', ROOT/'target/release/helm')).resolve()
TITLE_MODEL = json.loads((ROOT/'helm/utility-models.json').read_text())['title']
KEY = 'offline-inference-fixture'


def human(body):
    result=[]
    for item in body.get('input',body.get('messages',[])):
        if item.get('role')!='user': continue
        content=item.get('content','')
        if isinstance(content,list):
            content=''.join(block.get('text','') for block in content
                            if block.get('type') in ('text','input_text'))
        if content: result.append(content)
    return result[-1] if result else ''


def wire(provider,value,serial,usage):
    raw=response(provider,value,serial)
    frames=[]
    for data in raw.split(b'\n\n'):
        if not data: continue
        if data==b'data: [DONE]': frames.append(data+b'\n\n'); continue
        item=json.loads(data.removeprefix(b'data: '))
        item.pop('usage',None)
        if 'response' in item:
            item['response'].pop('usage',None)
            if usage is not None: item['response']['usage']=usage
        elif 'message' in item:
            item['message'].pop('usage',None)
            if usage is not None:
                item['message']['usage']={k:v for k,v in usage.items() if k=='input_tokens'}
        elif item.get('type')=='message_delta':
            if usage is not None: item['usage']={k:v for k,v in usage.items() if k=='output_tokens'}
        elif 'choices' in item and usage is not None:
            item['usage']={('prompt_tokens' if k=='input_tokens' else 'completion_tokens'):v for k,v in usage.items()}
        frames.append(event(item))
    return b''.join(frames)


def partial(provider):
    if provider=='openai-chat':
        return event({'usage':{'prompt_tokens':7},'choices':[{'delta':{'content':'held-input'}}]})
    if provider=='openai-responses':
        return event({'type':'response.in_progress','response':{'usage':{'input_tokens':7}}})+event({'type':'response.output_text.delta','delta':'held-input'})
    return event({'type':'message_start','message':{'usage':{'input_tokens':7}}})+event({'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'held-input'}})


class Handler(BaseHTTPRequestHandler):
    def log_message(self,*_): pass
    def do_GET(self):
        case=self.server.case
        models=['fixture']+([TITLE_MODEL] if case.mode in ('title','title-blocked') else [])
        raw=json.dumps({'data':[{'id':model} for model in models]}).encode()
        self.send_response(200);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
    def do_POST(self):
        case=self.server.case
        try:
            assert self.path=={'openai-chat':'/v1/chat/completions','openai-responses':'/v1/responses','anthropic':'/v1/messages'}[case.provider]
            assert (self.headers.get('x-api-key') if case.provider=='anthropic' else self.headers.get('Authorization')) == (KEY if case.provider=='anthropic' else 'Bearer '+KEY)
            body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            with case.lock:
                serial=len(case.requests);case.requests.append(body)
            if case.mode=='retry':
                raw=b'{"error":{"message":"offline retry fixture"}}'
                self.send_response(503);self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw);return
            if case.mode in ('partial','cancel','restart'):
                self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers()
                self.wfile.write(partial(case.provider));self.wfile.flush()
                if case.mode!='partial': assert case.release.wait(30),'held fixture deadline'
                return
            value=case.answer(body)
            usage=({'input_tokens':0} if case.mode=='usage' else {'input_tokens':True} if case.mode=='malformed' else None if case.mode=='unavailable' else {'input_tokens':3,'output_tokens':2})
            raw=wire(case.provider,value,serial,usage)
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
        except (BrokenPipeError,ConnectionResetError): pass
        except BaseException as error:
            case.failures.append(repr(error));self.send_error(500)


class Case:
    def __init__(self,provider,mode):
        self.provider=provider;self.mode=mode
        self.requests=[];self.failures=[];self.lock=threading.Lock();self.release=threading.Event()
        self.steps={};self.serial=0
        self.temp=tempfile.TemporaryDirectory(prefix='helm-inference-');self.root=Path(self.temp.name)
        self.workspace=self.root/'workspace';self.workspace.mkdir()
        self.env=dict(os.environ,HOME=str(self.root/'home'),XDG_CONFIG_HOME=str(self.root/'config'),XDG_DATA_HOME=str(self.root/'data'),HELM_BUDGET_KEY=KEY)
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler);self.server.case=self
        self.thread=threading.Thread(target=self.server.serve_forever,daemon=True);self.thread.start()
        if mode=='worktree':
            for command in (['git','init','-q',str(self.workspace)],['git','-C',str(self.workspace),'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-qm','fixture']):
                subprocess.run(command,check=True,capture_output=True,timeout=10)
        self.config=self.root/'config.toml'
        self.config.write_text(f'provider={json.dumps(provider)}\nmodel="fixture"\napi_key_env="HELM_BUDGET_KEY"\nbase_url="http://127.0.0.1:{self.server.server_port}/v1"\naccess="unrestricted"\nprovider_retry_attempts=4\nprovider_retry_initial_ms=1\nprovider_retry_max_ms=2\nsubagent_max_concurrency=2\n')
        if mode=='worktree':
            with self.config.open('a') as config:
                config.write(f'allow_read=[{json.dumps(str(self.root))}]\nallow_write=[{json.dumps(str(self.root))}]\n')
    def answer(self,body):
        outputs,_=normalize(self.provider,body)
        actor=human(body)
        with self.lock:
            step=self.steps.get(actor,0);self.steps[actor]=step+1
        if self.mode in ('child','nested','worktree'):
            if actor=='grand-work': return 'grandchild finished'
            if actor=='child-work' and self.mode in ('child','worktree'): return 'child finished'
            if step==0:
                task='grand-work' if actor=='child-work' else 'child-work'
                return 'subagent',{'action':'spawn','name':task,'task':task,'worktree':self.mode=='worktree'}
            if step==1:
                spawned=json.loads(outputs[-1])
                assert isinstance(spawned,dict) and isinstance(spawned.get('id'),str),('spawn did not create a child',spawned)
                return 'subagent',{'action':'wait','id':spawned['id']}
            return 'premature proposal after child'
        if actor=='effect' and step==0:
            return 'shell',{'command':"printf 'effect\\n' >> effect.txt"}
        return 'verified fixture completion'
    def command(self,args,workspace=None):
        return [str(HELM),'--config',str(self.config),'--workspace',str(workspace or self.workspace),*args]
    def invoke(self,args,ok=None,workspace=None):
        number=self.serial;self.serial+=1
        outpath=self.root/f'{number}.stdout';errpath=self.root/f'{number}.stderr'
        with outpath.open('wb') as out,errpath.open('wb') as err:
            child=subprocess.Popen(self.command(args,workspace),cwd=self.root,env=self.env,stdout=out,stderr=err,start_new_session=True)
            try: code=child.wait(timeout=45)
            except BaseException:
                os.killpg(child.pid,signal.SIGKILL);child.wait();raise
        stdout=outpath.read_bytes();stderr=errpath.read_bytes()
        assert len(stdout)<1024*1024 and len(stderr)<1024*1024,'fixture output bound'
        if ok is not None: assert (code==0)==ok,(args,code,stdout.decode(),stderr.decode())
        return code,stdout.decode(),stderr.decode()
    def inspect(self,session=None):
        args=['inference','inspect']+(['--session',session] if session else [])
        return json.loads(self.invoke(args,True)[1])
    def configure(self,limit,revision=0,session=None,warning=None):
        operation=str(uuid.uuid4())
        args=['inference','configure','--operation',operation,'--expected-revision',str(revision),'--reason','Explicit offline operator allowance']
        args+=['--unlimited'] if limit is None else ['--limit',str(limit)]
        if session:args+=['--session',session]
        if warning is not None:args+=['--warning',str(warning)]
        preview=json.loads(self.invoke(args,True)[1]);assert not preview['committed']
        first=json.loads(self.invoke(args+['--confirm'],True)[1]);retry=json.loads(self.invoke(args+['--confirm'],True)[1]);assert first==retry
        return first
    def sessions(self): return [json.loads(path.read_text()) for path in (self.root/'data/helm/sessions').glob('*.json')]
    def interrupted(self):
        assert all(session['run_summaries'][-1]['phase']!='completed' for session in self.sessions()),self.sessions()
    def held(self):
        outpath=self.root/'held.stdout';errpath=self.root/'held.stderr'
        with outpath.open('wb') as out,errpath.open('wb') as err:
            child=subprocess.Popen(self.command(['run','held']),cwd=self.root,env=self.env,stdout=out,stderr=err,start_new_session=True)
            try:
                deadline=time.monotonic()+15
                while 'held-input' not in outpath.read_text():
                    assert child.poll() is None,(outpath.read_text(),errpath.read_text())
                    assert time.monotonic()<deadline,'held output deadline'
                    time.sleep(.02)
                child.send_signal(signal.SIGKILL if self.mode=='restart' else signal.SIGINT)
                child.wait(timeout=15)
            finally:
                self.release.set()
                if child.poll() is None:os.killpg(child.pid,signal.SIGKILL);child.wait()
        assert child.returncode!=0
    def verify(self):
        if self.mode=='project':
            self.configure(2,warning=1)
            self.invoke(['run','effect'],True)
            self.invoke(['run','again'],False)
            assert (self.workspace/'effect.txt').read_text()=='effect\n'
            assert len(self.requests)==2
            self.configure(3,revision=1)
            self.invoke(['run','explicit additional work'],True)
            assert len(self.requests)==3
        elif self.mode=='session':
            self.invoke(['run','initial'],True)
            session=self.sessions()[0]['id'];self.configure(2,session=session,warning=2)
            self.invoke(['run','--resume',session,'effect'],False)
            assert (self.workspace/'effect.txt').read_text()=='effect\n'
            assert self.inspect(session)['status']['consumed']==2
            self.invoke(['run','fresh session'],True)
            assert len(self.requests)==3
        elif self.mode in ('retry','partial','malformed'):
            self.configure(2)
            self.invoke(['run',self.mode],False)
            assert len(self.requests)==(2 if self.mode=='retry' else 1)
            self.interrupted()
        elif self.mode in ('cancel','restart'):
            self.configure(1);self.held()
            session=self.sessions()[0]['id']
            self.invoke(['run','--resume',session,'resume after interruption'],False)
            assert len(self.requests)==1
            self.interrupted()
        elif self.mode in ('title','title-blocked'):
            self.configure(2 if self.mode=='title' else 1)
            self.invoke(['run','A useful title for this conversation'],True)
            assert len(self.requests)==(2 if self.mode=='title' else 1)
        elif self.mode=='plain':
            command=self.command(['chat','--plain'])
            completed=subprocess.run(command,cwd=self.root,env=self.env,input='/inference\nnormal prompt\n/quit\n',capture_output=True,text=True,timeout=30)
            assert completed.returncode==0,(completed.stdout,completed.stderr)
            assert '0 local attempts' in completed.stdout,completed.stdout
            assert len(self.requests)==1
            assert human(self.requests[0])=='normal prompt'
        elif self.mode in ('failure-admission','failure-finish'):
            self.configure(2)
            database=self.root/'data/helm/inference/journal.sqlite3'
            secret='private-fixture-diagnostic-never-display'
            with sqlite3.connect(database) as connection:
                if self.mode=='failure-admission':
                    connection.execute(f"CREATE TRIGGER failure BEFORE INSERT ON attempts BEGIN SELECT RAISE(ABORT,'{secret}'); END;")
                else:
                    connection.execute(f"CREATE TRIGGER failure BEFORE UPDATE ON attempts WHEN json_extract(NEW.record,'$.outcome')='completed' BEGIN SELECT RAISE(ABORT,'{secret}'); END;")
            _,out,err=self.invoke(['run','effect'],False)
            assert secret not in out+err
            assert not (self.workspace/'effect.txt').exists()
            assert len(self.requests)==(0 if self.mode=='failure-admission' else 1)
            self.interrupted()
            if self.mode=='failure-finish':
                assert self.sessions()[0]['usage']=={'input_tokens':3,'output_tokens':2},self.sessions()
        elif self.mode=='corrupt-session':
            self.invoke(['run','initial'],True)
            session=self.sessions()[0]['id'];self.configure(0,session=session)
            database=self.root/'data/helm/inference/journal.sqlite3'
            with sqlite3.connect(database) as connection:
                connection.execute('DELETE FROM limits WHERE scope=?',('session:'+session,))
            self.invoke(['run','--resume',session,'must not repair authority'],False)
            assert len(self.requests)==1
            with sqlite3.connect(database) as connection:
                assert connection.execute('SELECT count(*) FROM limits WHERE scope=?',('session:'+session,)).fetchone()[0]==0
        elif self.mode=='model':
            self.configure(2);self.invoke(['run','initial'],True)
            session=self.sessions()[0]['id']
            self.invoke(['--model','second-fixture','run','--resume',session,'second model'],True)
            self.invoke(['run','--resume',session,'exhausted'],False)
            assert [body['model'] for body in self.requests]==['fixture','second-fixture']
        elif self.mode in ('child','nested','worktree'):
            self.configure(3 if self.mode in ('child','worktree') else 5)
            self.invoke(['run','root-work'],False)
            assert len(self.requests)==(3 if self.mode in ('child','worktree') else 5)
            self.interrupted()
        else:
            assert self.mode in ('usage','unavailable');self.invoke(['run','reported zero'],True)
        report=self.inspect();status=report['status'];attempts=report['attempts']
        assert status['consumed']==len(self.requests),(status,self.requests)
        assert status['omitted_attempt_details']==0
        assert len(attempts)==len(self.requests)
        sessions={item['id']:item for item in self.sessions()}
        for attempt in attempts:
            owner=attempt['attribution'];saved=sessions[owner['session']]
            assert any(ref['run_id']==owner['run'] for ref in saved['completion_runs'])
            assert owner['provider']==self.provider
            assert attempt['project']==status['scope']['project']
        if self.mode in ('partial','cancel','restart'):
            assert attempts[0]['input_tokens']==7 and attempts[0]['output_tokens'] is None
            assert attempts[0]['outcome']==('failed' if self.mode=='partial' else 'unknown')
        if self.mode=='failure-finish':
            assert attempts[0]['outcome']=='unknown' and attempts[0]['input_tokens']==3 and attempts[0]['output_tokens']==2
        if self.mode in ('unavailable','malformed'):
            assert attempts[0]['input_tokens'] is None and attempts[0]['output_tokens'] is None
            assert attempts[0]['outcome']==('failed' if self.mode=='malformed' else 'completed')
        if self.mode=='usage':assert attempts[0]['input_tokens']==0 and attempts[0]['output_tokens'] is None
        if self.mode=='retry':assert all(row['input_tokens'] is None and row['outcome']=='failed' for row in attempts)
        if self.mode=='title':assert [row['attribution']['purpose'] for row in attempts]==['conversation','title']
        if self.mode in ('child','nested','worktree'):
            agents={row['attribution']['agent'] for row in attempts if row['attribution']['agent']}
            assert len(agents)==(1 if self.mode in ('child','worktree') else 2),agents
            assert len({row['attribution']['session'] for row in attempts})==1
            assert len({row['attribution']['run'] for row in attempts})==1
        if self.mode=='worktree':
            database=self.root/'data/helm/inference/journal.sqlite3'
            with sqlite3.connect(database) as connection:
                assert connection.execute('SELECT count(*) FROM projects').fetchone()[0]==1,'child worktree created a new budget project'
            records=[]
            for path in (self.root/'data/helm/subagents').glob('*.json'):
                tree=json.loads(path.read_text());records.extend(tree.get('agents',[]).values() if isinstance(tree.get('agents'),dict) else tree.get('agents',[]))
            for path in (self.root/'data/helm/subagents').glob('*.archive/*.json'):
                records.append(json.loads(path.read_text())['record'])
            child_records=[record for record in records if record['id'] in agents]
            assert len(child_records)==1,records
            child_record=child_records[0]
            assert child_record.get('worktree') and Path(child_record['worktree'])!=self.workspace,child_record
            assert child_record['completion']['session_id']==attempts[0]['attribution']['session'],child_record
            assert child_record['completion']['run_id']==attempts[0]['attribution']['run'],child_record
        audit=json.loads(self.invoke(['inference','audit'],True)[1])['audit']
        assert all(row['scope']==status['scope'] for row in audit)
        assert all(row['detail']['reason'].strip() for row in audit if row['event']=='configured')
        assert not self.failures,self.failures
        # Provider credentials and prompts are not fields in the private ledger.
        assert KEY not in json.dumps(report)
        print(f'PASS {self.provider} {self.mode}',flush=True)
    def close(self):
        self.release.set();self.server.shutdown();self.server.server_close();self.thread.join();self.temp.cleanup()

    def retain_failure(self):
        destination=os.environ.get('HELM_INFERENCE_EVIDENCE')
        if not destination:return
        # Only this synthetic case's regular artifacts are copied, with explicit bounds.
        target=Path(destination)/f'{self.provider}-{self.mode}-{uuid.uuid4()}'
        target.mkdir(parents=True,exist_ok=False)
        total=0;count=0;omitted=[]
        for directory,dirs,files in os.walk(self.root,followlinks=False):
            dirs[:]=[name for name in dirs if not (Path(directory)/name).is_symlink()]
            for name in files:
                source=Path(directory)/name;relative=source.relative_to(self.root)
                if source.is_symlink() or not source.is_file():continue
                size=source.stat().st_size
                if size>8*1024*1024 or total+size>32*1024*1024 or count>=1000:
                    omitted.append(str(relative));continue
                output=target/relative;output.parent.mkdir(parents=True,exist_ok=True)
                shutil.copyfile(source,output);total+=size;count+=1
        (target/'retention.json').write_text(json.dumps({'files':count,'bytes':total,'omitted':omitted},indent=2))
        print(f'Failure artifacts retained at {target}',flush=True)


def main():
    assert HELM.is_file(),HELM
    for provider in ('openai-chat','openai-responses','anthropic'):
        for mode in ('project','session','retry','usage','partial','cancel','restart','title','title-blocked','plain','model','child','nested','worktree','corrupt-session','unavailable','malformed','failure-admission','failure-finish'):
            case=Case(provider,mode)
            try:case.verify()
            except BaseException:
                try:case.retain_failure()
                except Exception as error:print(f'Failure artifact retention failed: {error}',flush=True)
                raise
            finally:case.close()
    print('inference permit matrix passed (57 native cases)',flush=True)

if __name__=='__main__':main()
