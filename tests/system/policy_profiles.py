#!/usr/bin/env python3
"""Named-profile administration and actual offline native execution; no host /etc edits."""
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT=Path(__file__).resolve().parents[2]
HELM=Path(os.environ.get('HELM_BIN',ROOT/'target/release/helm')).resolve()
class Provider(BaseHTTPRequestHandler):
    requests=[]
    failures=[]
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
            payload=('data: '+json.dumps({'choices':[{'delta':delta,'finish_reason':'stop'}]})+'\n\ndata: [DONE]\n\n').encode()
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
        except Exception as error:
            self.failures.append(repr(error));self.send_error(500)

def main():
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider);threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-profiles-') as directory:
            root=Path(directory);workspace=root/'workspace';workspace.mkdir();profiles=root/'profiles'
            config=root/'config.toml';config.write_text(f'provider="openai-chat"\nmodel="profile-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="PROFILE_FIXTURE_KEY"\naccess="read-only"\nprovider_retry_attempts=1\n[env]\nSECRET="config-secret-canary"\n')
            env=dict(os.environ,HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),PROFILE_FIXTURE_KEY='profile-key-canary')
            common=[str(HELM),'--config',str(config),'--workspace',str(workspace),'--policy-directory',str(profiles)]
            def invoke(*args,ok=True):
                prefix=common[:-2] if '--policy-directory' in args else common
                result=subprocess.run([*prefix,*args],env=env,cwd=workspace,stdin=subprocess.DEVNULL,capture_output=True,text=True,timeout=30)
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
            invoke(*flags,'run','inspect safely')
            assert not (workspace/'profile-effect').exists()
            assert len(Provider.requests)==2
            sessions=list((root/'data/helm/sessions').glob('*.json'));assert len(sessions)==1
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
            # Unknown schema/fields are rejected before any store change or provider request.
            malformed=json.loads(exported);malformed['unknown_secret_field']='file-secret-canary';exportfile.write_text(json.dumps(malformed))
            invoke('policy','import','bad','--input',str(exportfile),ok=False)
            invoke('--policy-profile','review','run','missing exact binding',ok=False)
            assert not Provider.failures,Provider.failures
            print('policy profiles: CRUD, inert import/export, exact selection, escalation, denial, tombstone/recreation and resumed authority passed')
    finally:server.shutdown();server.server_close()
if __name__=='__main__':main()
