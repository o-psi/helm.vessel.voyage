#!/usr/bin/env python3
"""Actual CLI setup/discovery and native inference against bounded HTTP fixtures."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import signal
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BIN = str(Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve())
CANARY = 'fixture-key-must-not-leak-123456'
class Handler(BaseHTTPRequestHandler):
    requests = []
    mode = 'ok'
    def log_message(self, *_): pass
    def reply(self, status, value, **headers):
        data = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Length', str(len(data)))
        for k,v in headers.items(): self.send_header(k,v)
        self.end_headers()
        try: self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError): pass
    def do_GET(self):
        self.requests.append((self.path, self.headers.get('Authorization'), None))
        if self.mode == 'timeout': time.sleep(2)
        if self.mode == 'redirect': return self.reply(302, {}, Location=f'http://127.0.0.1:{self.server.server_port}/stolen')
        if self.mode == 'auth': return self.reply(401, {'error':CANARY})
        if self.mode == 'missing': return self.reply(404, {})
        if self.mode == 'echo-key': return self.reply(200, {'data':[{'id':CANARY}]})
        if self.mode == 'badmodels': return self.reply(200, {'data':[{'id':'bad\x1bmodel'}]})
        if self.mode == 'malformed': return self.reply(200, b'not-json-'+CANARY.encode())
        if self.mode == 'oversized': return self.reply(200, b'x' * (1024*1024+1))
        if self.mode == 'empty': return self.reply(200, {'data':[]})
        self.reply(200, {'data':[{'id':'z-fixture'},{'id':'a-fixture'},{'id':'a-fixture'}]})
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.requests.append((self.path, self.headers.get('Authorization'), body))
        if self.mode == 'postredirect': return self.reply(307, {}, Location=f'http://127.0.0.1:{self.server.server_port}/stolen')
        if self.mode == 'badprotocol': return self.reply(200, {'unrelated':'response'})
        if body.get('stream'):
            if self.path.endswith('/chat/completions'):
                unsupported = 'max_tokens' if self.mode == 'modern' else 'max_completion_tokens'
                if unsupported in body:
                    return self.reply(400, {'error':f'unsupported {unsupported}'})
            if self.path.endswith('/responses'):
                data={'type':'response.completed','response':{'id':'r','output':[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'fixture OK'}]}],'usage':{'input_tokens':1,'output_tokens':1}}}
            else:
                data={'choices':[{'delta':{'content':'fixture OK'},'finish_reason':'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':1}}
            return self.reply(200, ('data: '+json.dumps(data)+'\n\ndata: [DONE]\n\n').encode(), **{'Content-Type':'text/event-stream'})
        assert body.get('max_tokens',body.get('max_output_tokens',body.get('max_completion_tokens'))) == 16
        assert not body.get('tools')
        value={'id':'r','output':[]} if self.path.endswith('/responses') else {'choices':[{'message':{'role':'assistant','content':'OK'}}]}
        self.reply(200,value)

def main():
    with tempfile.TemporaryDirectory() as temporary:
        root=Path(temporary)
        env=dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), OPENAI_API_KEY=CANARY, FIXTURE_KEY=CANARY)
        sentinel=root/'bin';sentinel.mkdir()
        marker=root/'codex-started'
        codex=sentinel/'codex';codex.write_text('#!/bin/sh\ntouch '+str(marker)+'\nexit 99\n');codex.chmod(0o700)
        env['PATH']=str(sentinel)+os.pathsep+env.get('PATH','')
        server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        threading.Thread(target=server.serve_forever,daemon=True).start()
        base=f'http://127.0.0.1:{server.server_port}/v1'
        def run(*args, ok=True):
            p=subprocess.run([BIN,*args],env=env,cwd=root,text=True,capture_output=True,timeout=15)
            assert (p.returncode == 0)==ok,(args,p.returncode,p.stdout,p.stderr)
            assert CANARY not in p.stdout+p.stderr
            return p
        try:
            assert len(json.loads(run('local-provider','presets').stdout)['presets'])==4
            assert Handler.requests==[]
            # A non-writable/unsupported publication directory must fail before HTTP.
            before=len(Handler.requests)
            run('local-provider','setup','custom','--endpoint',base,'--output','/proc/helm-fixture-publication.toml',ok=False)
            assert len(Handler.requests)==before
            for preset in ['ollama','lm-studio','vllm','llama-cpp','custom']:
                for transport in ['chat','responses']:
                    cfg=root/f'{preset}-{transport}.toml'
                    report=json.loads(run('local-provider','setup',preset,'--endpoint',base,'--transport',transport,'--output',str(cfg)).stdout)['provider']
                    assert report['selected_model']=='a-fixture' and report['models']==['a-fixture','z-fixture']
                    assert report['credential_source']=='none' and report['protocol_validated']
                    assert 'api_key_required = false' in cfg.read_text()
                    before=len(Handler.requests)
                    run('local-provider','setup',preset,'--endpoint',base,'--output',str(cfg),ok=False)
                    assert len(Handler.requests)==before
                    doctor=json.loads(run('--config',str(cfg),'doctor').stdout)
                    assert doctor['provider_ready'] and doctor['endpoint_diagnostics']['credential_source']=='none'
                    assert doctor['provider_credential_present'] is None
                    assert len(Handler.requests)==before
                    run('--config',str(cfg),'config')
                    run('--config',str(cfg),'models','--json')
                    result=run('--config',str(cfg),'--workspace',str(root),'run','--no-save','Reply OK')
                    assert 'fixture OK' in result.stdout
                    assert not {'max_tokens','max_output_tokens','max_completion_tokens'} & Handler.requests[-1][2].keys()
                    run('--config',str(cfg),'--workspace',str(root),'--set','max_tokens=123','run','--no-save','Reply OK')
                    field = 'max_output_tokens' if transport == 'responses' else 'max_tokens'
                    assert Handler.requests[-1][2][field] == 123
            assert all(auth is None for _,auth,_ in Handler.requests)
            run('local-provider','probe','custom','--endpoint',base,'--api-key-env','FIXTURE_KEY')
            assert Handler.requests[-1][1]=='Bearer '+CANARY
            authenticated=root/'authenticated.toml'
            run('local-provider','setup','custom','--endpoint',base,'--api-key-env','FIXTURE_KEY','--output',str(authenticated))
            Handler.mode='postredirect'
            before=len(Handler.requests)
            run('local-provider','probe','custom','--endpoint',base,'--api-key-env','FIXTURE_KEY',ok=False)
            assert len(Handler.requests)==before+2
            run('--config',str(authenticated),'--workspace',str(root),'run','--no-save','Reply OK',ok=False)
            assert not any(path=='/stolen' for path,_,_ in Handler.requests)
            Handler.mode='modern'
            modern=root/'modern.toml'
            run('local-provider','setup','custom','--endpoint',base,'--chat-max-completion-tokens','--output',str(modern))
            assert Handler.requests[-1][2]['max_completion_tokens']==16
            run('--config',str(modern),'--workspace',str(root),'run','--no-save','Reply OK')
            assert not {'max_tokens','max_output_tokens','max_completion_tokens'} & Handler.requests[-1][2].keys()
            run('--config',str(modern),'--workspace',str(root),'--set','max_tokens=123','run','--no-save','Reply OK')
            assert Handler.requests[-1][2]['max_completion_tokens'] == 123
            Handler.mode='ok'
            for provider in ['openai-responses','anthropic']:
                p=run('--config',str(root/'ollama-chat.toml'),'--set','provider='+provider,'config')
                assert 'api_key_required = true' in p.stdout
            before=len(Handler.requests)
            if os.name=='posix':
                dangling=root/'dangling.toml';dangling.symlink_to(root/'missing-target')
                run('local-provider','setup','custom','--endpoint',base,'--output',str(dangling),ok=False)
                assert dangling.is_symlink() and not (root/'missing-target').exists()
            env['EMPTY_FIXTURE_KEY']=''
            run('local-provider','probe','custom','--endpoint',base,'--api-key-env','EMPTY_FIXTURE_KEY',ok=False)
            assert len(Handler.requests)==before
            for mode, expected in [('auth','authentication failure'),('badmodels','model-list failure'),('malformed','protocol failure'),('oversized','protocol failure'),('badprotocol','protocol failure'),('redirect','protocol failure'),('empty','model-list unavailable')]:
                Handler.mode=mode; out=root/f'failed-{mode}.toml'
                p=run('local-provider','setup','custom','--endpoint',base,'--output',str(out),ok=False)
                assert expected in p.stderr and not out.exists(),p.stderr
            assert not any(path=='/stolen' for path,_,_ in Handler.requests)
            Handler.mode='ok'
            for field,value in [('--model','prefix-'+CANARY),('--endpoint',base+'/'+CANARY)]:
                rejected=root/('secret-'+field[2:]+'.toml')
                before=len(Handler.requests)
                args=['local-provider','setup','custom','--api-key-env','FIXTURE_KEY','--output',str(rejected)]
                if field!='--endpoint': args.extend(['--endpoint',base])
                args.extend([field,value])
                run(*args,ok=False)
                assert len(Handler.requests)==before and not rejected.exists()
            Handler.mode='redirect'
            before=len(Handler.requests)
            run('local-provider','probe','custom','--endpoint',base,'--api-key-env','FIXTURE_KEY',ok=False)
            assert len(Handler.requests)==before+1 and Handler.requests[-1][1]=='Bearer '+CANARY
            Handler.mode='echo-key'
            run('local-provider','probe','custom','--endpoint',base,'--api-key-env','FIXTURE_KEY',ok=False)
            Handler.mode='ok'
            concurrent=root/'concurrent.toml'
            args=[BIN,'local-provider','setup','custom','--endpoint',base,'--output',str(concurrent)]
            processes=[subprocess.Popen(args,env=env,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True) for _ in range(2)]
            results=[p.communicate(timeout=15) for p in processes]
            assert sorted(p.returncode for p in processes)==[0,1],results
            assert 'api_key_required = false' in concurrent.read_text()
            for mode in ['auth','malformed','oversized','echo-key','badmodels','redirect']:
                Handler.mode=mode
                before=len(Handler.requests)
                run('--config',str(authenticated),'models','--json',ok=False)
                assert len(Handler.requests)==before+1
            Handler.mode='missing'
            run('local-provider','probe','custom','--endpoint',base,ok=False)
            report=json.loads(run('local-provider','probe','custom','--endpoint',base,'--model','manual').stdout)
            assert report['discovery']=='unavailable_manual_model' and report['selected_model']=='manual'
            manual=root/'manual.toml'
            run('local-provider','setup','custom','--endpoint',base,'--model','manual','--output',str(manual))
            run('--config',str(manual),'--workspace',str(root),'run','--no-save','Reply OK')
            Handler.mode='timeout'
            p=run('local-provider','probe','custom','--endpoint',base,'--timeout-secs','1',ok=False)
            assert 'timeout' in p.stderr
            if os.name=='posix':
                pending=root/'cancelled.toml'
                before=len(Handler.requests)
                process=subprocess.Popen([BIN,'local-provider','setup','custom','--endpoint',base,'--output',str(pending)],env=env,cwd=root,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                until=time.monotonic()+5
                while len(Handler.requests)==before and time.monotonic()<until: time.sleep(.01)
                assert len(Handler.requests)>before
                process.send_signal(signal.SIGINT)
                process.communicate(timeout=5)
                assert process.returncode!=0 and not pending.exists()
            before=len(Handler.requests)
            for bad in ['http://example.org/v1',base+'?key=secret',base+'#secret','https://user:secret@example.org/v1']:
                run('local-provider','probe','custom','--endpoint',bad,ok=False)
            assert len(Handler.requests)==before
            Handler.mode='ok'
            missing=root/'no-key.toml'; missing.write_text('api_key_env="MISSING_FIXTURE_KEY"\n')
            run('--config',str(missing),'models',ok=False)
            # Exercise the real fixed-candidate scanner on one available documented
            # port. Never take over a port owned by an existing local service.
            class ScanHandler(Handler):
                requests=[]
                mode='ok'
            scanner=None
            for port in [1234,8000,8080]:
                try: scanner=ThreadingHTTPServer(('127.0.0.1',port),ScanHandler);break
                except OSError: pass
            assert scanner is not None,'no free fixed candidate for scanner fixture'
            threading.Thread(target=scanner.serve_forever,daemon=True).start()
            before=len(Handler.requests)
            prior={name:env.get(name) for name in ['HTTP_PROXY','http_proxy','NO_PROXY','no_proxy']}
            env['HTTP_PROXY']=env['http_proxy']=base
            env['NO_PROXY']=env['no_proxy']=''
            try:
                records=json.loads(run('local-provider','scan').stdout)
                assert len(records)==4
                assert {record['endpoint'] for record in records}=={f'http://127.0.0.1:{port}/v1' for port in [11434,1234,8000,8080]}
                record=next(r for r in records if r['endpoint']==f'http://127.0.0.1:{scanner.server_port}/v1')
                assert record['models']==['a-fixture','z-fixture'] and not record['protocol_validated']
                assert ScanHandler.requests==[('/v1/models',None,None)]
                assert len(Handler.requests)==before,'scan unexpectedly used environment proxy'
            finally:
                scanner.shutdown();scanner.server_close()
                for name,value in prior.items():
                    if value is None:env.pop(name,None)
                    else:env[name]=value
            assert not marker.exists()
            print('local provider: five presets, two native transports, no-auth/explicit-key isolation, create-only setup, bounds/errors/manual fallback and inference passed')
        finally: server.shutdown();server.server_close()
if __name__=='__main__': main()
