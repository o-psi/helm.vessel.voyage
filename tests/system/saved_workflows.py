#!/usr/bin/env python3
"""Actual saved-workflow CLI, offline native provider and isolated session storage."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()
DOCUMENT = '''schema_version=1
id="review-change"
version="1.0"
description="Review local data"
prompt="Review {{target}} count={{count}}"
[parameters.target]
type="string"
required=true
[parameters.count]
type="integer"
default=2
minimum=1
maximum=4
[recommended]
provider="codex-compatibility"
model="not-selected"
access="unrestricted"
'''

class Provider(BaseHTTPRequestHandler):
    requests = []
    mode = 'success'
    failures = []
    sessions = None
    started = threading.Event()
    release = threading.Event()
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.reply({'data':[{'id':'workflow-fixture'}]})
    def reply(self, data, status=200):
        body=json.dumps(data).encode()
        self.send_response(status); self.send_header('Content-Type','application/json')
        self.send_header('Content-Length',str(len(body))); self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        type(self).requests.append(body)
        try:
            assert body['model']=='workflow-fixture', body
            assert self.headers['Authorization']=='Bearer synthetic-fixture-key'
            if self.mode!='no-save':
                saved=[json.loads(path.read_text()) for path in self.sessions.glob('*.json')]
                assert any(s.get('workflow_runs') for s in saved), saved
            if self.mode=='hold':
                payload=('data: '+json.dumps({'choices':[{'delta':{'content':'workflow-partial-before-cancel'}}]})+'\n\n').encode()
                self.send_response(200);self.send_header('Content-Type','text/event-stream');self.end_headers();self.wfile.write(payload);self.wfile.flush()
                self.started.set();assert self.release.wait(15)
                return
            if self.mode=='failure':
                return self.reply({'error':{'message':'fixture failure'}},500)
            if self.mode=='denied' and not any(m['role']=='tool' for m in body['messages']):
                delta={'tool_calls':[{'index':0,'id':'workflow-shell','type':'function','function':{'name':'shell','arguments':json.dumps({'command':'touch should-not-exist'})}}]}
            else:
                if self.mode=='denied':
                    tools=[m for m in body['messages'] if m['role']=='tool'];assert tools,body
                    assert 'denied' in tools[-1]['content'].lower() or 'approval' in tools[-1]['content'].lower(),tools
                delta={'content':'workflow-finished'}
            payload=('data: '+json.dumps({'choices':[{'delta':delta,'finish_reason':'stop'}]})+'\n\ndata: [DONE]\n\n').encode()
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        except Exception as error:
            self.failures.append(repr(error));self.reply({'error':{'message':'fixture assertion'}},500)

def main():
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-workflow-') as directory:
            root=Path(directory);workspace=root/'workspace';workspace.mkdir();user=root/'user';user.mkdir()
            repo=workspace/'.helm/workflows';repo.mkdir(parents=True)
            config=root/'config.toml';config.write_text(f'provider="openai-chat"\nmodel="workflow-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="WORKFLOW_FIXTURE_KEY"\naccess="approval"\nunattended_approval="deny"\nprovider_retry_attempts=1\n')
            env=dict(os.environ,HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),WORKFLOW_FIXTURE_KEY='synthetic-fixture-key')
            Provider.sessions=root/'data/helm/sessions'
            def invoke(*args, ok=True, cfg=config):
                result=subprocess.run([str(HELM),'--config',str(cfg),'--workspace',str(workspace),'workflow','--user-directory',str(user),*args],env=env,cwd=workspace,capture_output=True,text=True,timeout=20)
                assert (result.returncode==0)==ok,(args,result.returncode,result.stdout,result.stderr)
                return result
            def js(*args,**kwargs): return json.loads(invoke('--json',*args,**kwargs).stdout)
            (user/'review-change.toml').write_text(DOCUMENT)
            assert js('list',cfg=root/'missing-config')['workflows'][0]['scope']=='user'
            assert js('validate','review-change')['valid']
            selected=js('inspect','review-change');assert selected['document']['recommended']['access']=='unrestricted'
            literal='$(touch literal-effect) {{count}}\n雪'
            preview=js('preview','review-change','--input','target='+literal)
            assert preview['prompt']=='Review '+json.dumps(literal,ensure_ascii=False)+' count=2',preview
            assert preview['workflow']['inputs']=={'target':literal,'count':2}
            assert not Provider.requests and not Provider.sessions.exists()
            for extra in [[],['--input','target=x','--input','count=9'],['--input','target=x','--input','target=y'],['--input','unknown=x']]:invoke('run','review-change',*extra,ok=False)
            assert not Provider.requests and not Provider.sessions.exists()
            (repo/'review-change.toml').write_text(DOCUMENT.replace('version="1.0"','version="2.0"'))
            assert len(js('list')['workflows'])==2
            selected=js('inspect','review-change');assert selected['scope']=='repository';digest=selected['digest']
            invoke('preview','review-change','--input','target=x',ok=False)
            invoke('run','review-change','--input','target=x','--trust-repository','0'*64,ok=False)
            assert not Provider.requests
            changed=(repo/'review-change.toml').read_text()+'\n';(repo/'review-change.toml').write_text(changed)
            invoke('run','review-change','--input','target=x','--trust-repository',digest,ok=False)
            digest=js('inspect','review-change')['digest']
            result=invoke('run','review-change','--input','target='+literal,'--trust-repository',digest)
            assert 'workflow-finished' in result.stdout,result
            saved=[json.loads(p.read_text()) for p in Provider.sessions.glob('*.json')];assert len(saved)==1,saved
            invocation=saved[0]['workflow_runs'][0]
            assert invocation['version']=='2.0' and invocation['digest']==digest and invocation['inputs']['target']==literal
            assert saved[0]['model']=='workflow-fixture' and saved[0]['model_history']==[]
            assert saved[0]['messages'][0]['content']==preview['prompt']
            assert not (workspace/'literal-effect').exists()
            Provider.mode='denied';invoke('run','review-change','--scope','user','--input','target=deny')
            assert not (workspace/'should-not-exist').exists()
            Provider.mode='failure';invoke('run','review-change','--scope','user','--input','target=failure',ok=False)
            assert len(list(Provider.sessions.glob('*.json')))==3
            failed=[json.loads(p.read_text()) for p in Provider.sessions.glob('*.json') if json.loads(p.read_text())['workflow_runs'][0]['inputs']['target']=='failure']
            assert len(failed)==1 and failed[0]['run_summaries'][-1]['phase']=='interrupted',failed
            assert any(m['role']=='user' and 'failure' in m['content'] for m in failed[0]['messages']),failed
            Provider.mode='no-save' ;invoke('run','review-change','--scope','user','--input','target=ephemeral','--no-save')
            assert len(list(Provider.sessions.glob('*.json')))==3
            Provider.mode='hold'
            process=subprocess.Popen([str(HELM),'--config',str(config),'--workspace',str(workspace),'workflow','--user-directory',str(user),'run','review-change','--scope','user','--input','target=cancel'],env=env,cwd=workspace,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            try:
                assert Provider.started.wait(10)
                process.send_signal(signal.SIGINT)
                out,err=process.communicate(timeout=20)
                assert process.returncode!=0,(out,err)
                saved=[json.loads(p.read_text()) for p in Provider.sessions.glob('*.json')]
                cancelled=[s for s in saved if s['workflow_runs'][0]['inputs']['target']=='cancel']
                assert len(cancelled)==1,cancelled
                assert cancelled[0]['workflow_runs'][0]['id']=='review-change'
                assert any(m['role']=='user' and 'cancel' in m['content'] for m in cancelled[0]['messages']),cancelled
                assert cancelled[0]['run_summaries'][-1]['phase']=='interrupted',cancelled
            finally:
                Provider.release.set()
                if process.poll() is None:process.kill();process.wait(5)
            count=len(Provider.requests)
            secret=DOCUMENT.replace('required=true','required=true\nsecret=true');(user/'review-change.toml').write_text(secret)
            assert js('inspect','review-change','--scope','user')['document']['parameters']['target']['secret']
            for action in ['preview','run']:
                result=invoke(action,'review-change','--scope','user','--input','target=NEVER_EXPOSE_SECRET',ok=False)
                assert 'NEVER_EXPOSE_SECRET' not in result.stdout+result.stderr
                assert 'secret parameters are not supported' in result.stderr
            assert len(Provider.requests)==count
            assert 'NEVER_EXPOSE_SECRET' not in ''.join(p.read_text() for p in Provider.sessions.glob('*.json'))
            (user/'review-change.toml').write_text(DOCUMENT.replace('id="review-change"','id="run"'))
            invoke('list',ok=False)
            if os.name=='posix':
                (user/'review-change.toml').unlink();(user/'review-change.toml').symlink_to(repo/'review-change.toml');invoke('list',ok=False)
            assert not Provider.failures,Provider.failures
            print('saved workflow parsing/discovery/trust/native execution/policy/persistence/failure/no-save/secret rejection passed')
    finally:
        Provider.release.set()
        server.shutdown();server.server_close();thread.join(5)

if __name__=='__main__':main()
