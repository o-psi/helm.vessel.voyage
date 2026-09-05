#!/usr/bin/env python3
"""Named-profile administration and actual offline native execution; no host /etc edits."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT=Path(__file__).resolve().parents[2]
HELM=Path(os.environ.get('HELM_BIN',ROOT/'target/release/helm')).resolve()
class Provider(BaseHTTPRequestHandler):
    requests=[]
    failures=[]
    on_final=None
    def log_message(self,*_): pass
    def do_POST(self):
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        type(self).requests.append(body)
        try:
            assert self.headers['Authorization']=='Bearer profile-key-canary'
            last_user=max(i for i,m in enumerate(body['messages']) if m['role']=='user')
            tool=next((m for m in reversed(body['messages'][last_user+1:]) if m['role']=='tool'),None)
            if tool is None:
                delta={'tool_calls':[{'index':0,'id':'profile-call','type':'function','function':{'name':'shell','arguments':json.dumps({'command':'touch profile-effect'})}}]}
            else:
                delta={'content':'profile-fixture-complete'}
                callback=type(self).on_final
                type(self).on_final=None
                if callback is not None:callback()
            payload=('data: '+json.dumps({'choices':[{'delta':delta,'finish_reason':'stop'}]})+'\n\ndata: [DONE]\n\n').encode()
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        except Exception as error:
            self.failures.append(repr(error));self.send_error(500)

def handoff_refusal(common, flags, env, workspace):
    import pty, select, termios, fcntl, struct, signal, time
    master,slave=pty.openpty()
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
    previous=termios.tcgetattr(slave)
    process=subprocess.Popen([*common,*flags,'chat'],env=dict(env,TERM='xterm-256color'),cwd=workspace,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
    captured=bytearray()
    try:
        deadline=time.monotonic()+10
        while time.monotonic()<deadline and b'read-only' not in captured:
            if select.select([master],[],[],.1)[0]:captured.extend(os.read(master,65536))
            if process.poll() is not None:break
        assert b'read-only' in captured,captured[-2000:]
        os.write(master,b'/plain\r')
        deadline=time.monotonic()+10
        while time.monotonic()<deadline and process.poll() is None:
            if select.select([master],[],[],.1)[0]:captured.extend(os.read(master,65536))
        assert process.poll() is not None,'selected profile was lost into an unbound child: '+repr(captured[-2000:])
        while select.select([master],[],[],0)[0]:captured.extend(os.read(master,65536))
        assert process.returncode!=0 and b'selected policy profile' in captured and b'relaunch' in captured,captured[-2000:]
        assert termios.tcgetattr(slave)==previous,'full prior terminal mode must be restored'
    finally:
        if process.poll() is None:os.killpg(process.pid,signal.SIGKILL);process.wait(5)
        os.close(master);os.close(slave)

def main():
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider);threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-profiles-') as directory:
            root=Path(directory);workspace=root/'workspace';workspace.mkdir();profiles=root/'profiles'
            config=root/'config.toml';config.write_text(f'provider="openai-chat"\nmodel="profile-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="PROFILE_FIXTURE_KEY"\naccess="read-only"\nprovider_retry_attempts=1\n[env]\nSECRET="config-secret-canary"\n')
            env=dict(os.environ,HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),PROFILE_FIXTURE_KEY='profile-key-canary')
            default=subprocess.run([str(HELM),'policy','list'],env=env,cwd=workspace,capture_output=True,text=True,timeout=10)
            assert default.returncode==0,(default.stdout,default.stderr)
            assert len(json.loads(default.stdout)['profiles'])==3
            assert (root/'config/helm/profiles').is_dir()
            common=[str(HELM),'--config',str(config),'--workspace',str(workspace),'--policy-directory',str(profiles)]
            def invoke(*args,ok=True,input=None):
                prefix=common[:]
                if '--policy-directory' in args:del prefix[prefix.index('--policy-directory'):prefix.index('--policy-directory')+2]
                if '--workspace' in args:del prefix[prefix.index('--workspace'):prefix.index('--workspace')+2]
                result=subprocess.run([*prefix,*args],env=env,cwd=workspace,stdin=subprocess.DEVNULL if input is None else None,input=input,capture_output=True,text=True,timeout=30)
                assert (result.returncode==0)==ok,(args,result.returncode,result.stdout,result.stderr)
                assert 'config-secret-canary' not in result.stdout+result.stderr
                assert 'profile-key-canary' not in result.stdout+result.stderr
                return result
            def js(*args,**kw): return json.loads(invoke(*args,**kw).stdout)
            assert len(js('policy','list')['profiles'])==3
            created=js('policy','create','review','--preset','restricted')
            assert created['snapshot']['revision']==1
            selected=js('policy','inspect','review')
            flags=['--policy-profile','review','--policy-revision','1','--policy-digest',selected['digest']]
            handoff_config=root/'handoff.toml';handoff_config.write_text(config.read_text().replace('access="read-only"','access="unrestricted"'))
            handoff_common=[str(HELM),'--config',str(handoff_config),'--workspace',str(workspace),'--policy-directory',str(profiles)]
            handoff_refusal(handoff_common,flags,env,workspace)
            assert not Provider.requests
            invoke(*flags,'run','inspect safely')
            assert not (workspace/'profile-effect').exists()
            assert len(Provider.requests)==2
            sessions=[p for p in (root/'data/helm/sessions').glob('*.json') if json.loads(p.read_text())['messages']];assert len(sessions)==1
            saved=json.loads(sessions[0].read_text());assert 'policy_profile' not in sessions[0].read_text()
            assert saved['messages'][0]['content']=='inspect safely'
            exported=invoke('policy','export','review').stdout
            exportfile=root/'export.json';exportfile.write_text(exported)
            js('policy','import','imported','--input',str(exportfile));assert len(Provider.requests)==2
            js('policy','duplicate','review','copy')
            js('policy','delete','review','--expected-revision','1')
            before=len(Provider.requests);invoke(*flags,'run','stale selection',ok=False);assert len(Provider.requests)==before
            js('policy','create','review','--preset','restricted','--expected-revision','2')
            invoke(*flags,'run','recreated selection',ok=False);assert len(Provider.requests)==before
            autonomous=js('policy','inspect','autonomous')
            aflags=['--policy-profile','autonomous','--policy-revision','1','--policy-digest',autonomous['digest']]
            invoke(*aflags,'run','unconfirmed escalation',ok=False);assert len(Provider.requests)==before
            preview=js('policy','preview','autonomous','--revision','1','--digest',autonomous['digest'])
            assert preview['preview']['requires_confirmation']
            # Execute exactly the preview's emitted selection flags with the same Config/workspace.
            invoke(*shlex.split(preview['selection_flags']),'run','explicitly approved effect')
            assert (workspace/'profile-effect').exists();(workspace/'profile-effect').unlink()
            # Resume without selection uses original Config, never restores the earlier launch grant.
            latest=max((root/'data/helm/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
            resumed=json.loads(latest.read_text())['id']
            invoke('run','resume original authority','--resume',resumed)
            assert not (workspace/'profile-effect').exists()
            # Selected authority is bound to the actual saved workspace, never silently relabeled.
            other=root/'other';other.mkdir()
            count=len(Provider.requests)
            other_preview=js('--workspace',str(other),'policy','preview','autonomous','--revision','1','--digest',autonomous['digest'])
            invoke('--workspace',str(other),*aflags,'--policy-confirm',other_preview['preview']['transition_digest'],'run','mismatched workspace','--resume',resumed,ok=False)
            assert len(Provider.requests)==count
            managed=root/'managed'
            created=js('managed','--directory',str(managed),'--json','create')
            session_id=created['session']['id']
            current=js('policy','inspect','review')
            currentflags=['--policy-profile','review','--policy-revision',str(current['profile']['revision']),'--policy-digest',current['digest']]
            invoke(*currentflags,'managed','--directory',str(managed),'--json','submit',session_id,'--expected-revision','0','managed selected denial')
            assert not (workspace/'profile-effect').exists()
            fresh=js('policy','create','fresh','--preset','restricted')
            fsnapshot=js('policy','inspect','fresh')
            freshflags=['--policy-profile','fresh','--policy-revision','1','--policy-digest',fsnapshot['digest']]
            Provider.on_final=lambda:invoke('policy','delete','fresh','--expected-revision','1')
            count=len(Provider.requests)
            invoke(*freshflags,'chat','--plain',input='first profile turn\nrefused stale second turn\n/exit\n')
            assert len(Provider.requests)==count+2
            assert all('refused stale second turn' not in p.read_text() for p in (root/'data/helm/sessions').glob('*.json'))
            # The existing real nested-child fixture now runs with explicit restricted selection.
            # Preserve all of its scheduling, archive, denial and incomplete-accounting assertions.
            restricted=js('policy','inspect','restricted')
            wrapper=root/'selected-helm'
            launch=[str(HELM),'--policy-directory',str(profiles),'--policy-profile','restricted','--policy-revision','1','--policy-digest',restricted['digest']]
            wrapper.write_text('#!'+sys.executable+'\nimport os,sys\nargv='+repr(launch)+'\nos.execv(argv[0],argv+sys.argv[1:])\n')
            wrapper.chmod(0o700)
            import subagent_resources
            subagent_resources.run_resources(wrapper)
            # Unknown schema/fields are rejected before any store change or provider request.
            malformed=json.loads(exported);malformed['unknown_secret_field']='file-secret-canary';exportfile.write_text(json.dumps(malformed))
            invoke('policy','import','bad','--input',str(exportfile),ok=False)
            invoke('--policy-profile','review','run','missing exact binding',ok=False)
            assert not Provider.failures,Provider.failures
            print('policy profiles: CRUD, inert import/export, exact selection, escalation, denial, tombstone/recreation and resumed authority passed')
    finally:server.shutdown();server.server_close()
if __name__=='__main__':main()
