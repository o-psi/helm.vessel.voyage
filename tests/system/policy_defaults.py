#!/usr/bin/env python3
"""Operator-private defaults through actual CLI/native tools; no host config mutation."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from http.server import ThreadingHTTPServer
from policy_profiles import Provider, handoff_refusal
ROOT=Path(__file__).resolve().parents[2]
HELM=Path(os.environ.get('HELM_BIN',ROOT/'target/release/helm')).resolve()
def main():
    Provider.requests=[];Provider.failures=[];Provider.on_final=None
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider);threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-defaults-') as directory:
            root=Path(directory);work=root/'work';work.mkdir();other=root/'other';other.mkdir();profiles=root/'profiles';defaults=root/'defaults'
            source=root/'source.toml';enabled=root/'enabled.toml'
            original=f'# preserve this exact comment\nprovider="openai-chat"\nmodel="profile-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="PROFILE_FIXTURE_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\n[env]\nSECRET="defaults-config-secret-canary"\n'
            source.write_text(original)
            env=dict(os.environ,HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),PROFILE_FIXTURE_KEY='profile-key-canary')
            def invoke(*args,config=enabled,workspace=work,ok=True,input=None):
                result=subprocess.run([str(HELM),'--config',str(config),'--workspace',str(workspace),'--policy-directory',str(profiles),*args],cwd=work,env=env,stdin=subprocess.DEVNULL if input is None else None,input=input,capture_output=True,text=True,timeout=30)
                assert (result.returncode==0)==ok,(args,result.returncode,result.stdout,result.stderr)
                assert 'defaults-config-secret-canary' not in result.stdout+result.stderr
                assert 'profile-key-canary' not in result.stdout+result.stderr
                return result
            def js(*args,**kwargs):return json.loads(invoke(*args,**kwargs).stdout)
            anchor=js('policy','defaults','init','--directory',str(defaults),config=source)
            enable=['policy','defaults','enable','--directory',str(defaults),'--store-id',anchor['store_id'],'--output',str(enabled)]
            js(*enable,config=source);js(*enable,config=source)
            assert source.read_text()==original and enabled.read_text().startswith(original)
            malformed=root/'malformed.toml';malformed.write_text('api_key_required = "defaults-config-secret-canary"\n')
            invoke(*enable,config=malformed,ok=False)
            js('policy','list')
            restricted=js('policy','inspect','restricted');autonomous=js('policy','inspect','autonomous')
            def set_default(name,revision,scope='--global'):
                record=js('policy','inspect',name)
                return js('policy','defaults','set',scope,'--profile-directory',str(profiles),name,'--revision',str(record['profile']['revision']),'--digest',record['digest'],'--expected-revision',str(revision))
            set_default('restricted',0)
            before=len(Provider.requests);invoke('run','inspect through persistent defaults');assert len(Provider.requests)==before+2
            assert not (work/'profile-effect').exists()
            handoff_refusal([str(HELM),'--config',str(enabled),'--workspace',str(work)],[],env,work)
            # Unactivated broader B then equal C cannot erase the restrictive A baseline.
            set_default('autonomous',1);set_default('autonomous',2)
            before=len(Provider.requests);invoke('run','unactivated intermediate cannot grant',ok=False);assert len(Provider.requests)==before
            assert js('policy','defaults','preview')['preview']['requires_confirmation']
            set_default('restricted',3)
            invoke('run','restrictive candidate remains usable');assert not (work/'profile-effect').exists()
            # Clearing a global restriction does not activate its fallback in every workspace.
            js('policy','defaults','clear','--global','--expected-revision','4')
            before=len(Provider.requests);invoke('run','unconfirmed fallback',ok=False);assert len(Provider.requests)==before
            preview=js('policy','defaults','preview');assert preview['preview']['requires_confirmation']
            confirm=preview['preview']['transition_digest']
            js('policy','defaults','activate','--expected-revision','0','--confirm',confirm)
            invoke('run','explicit workspace fallback');assert (work/'profile-effect').exists();(work/'profile-effect').unlink()
            invoke('run','other workspace has no approval',workspace=other,ok=False)
            invoke('policy','defaults','activate','--expected-revision','0','--confirm',confirm,workspace=other,ok=False)
            # An explicit root selection is independent of an unavailable defaults source.
            renamed=root/'renamed-defaults';defaults.rename(renamed)
            before=len(Provider.requests);invoke('run','missing enabled source',ok=False);assert len(Provider.requests)==before
            flags=['--policy-profile','restricted','--policy-revision','1','--policy-digest',restricted['digest']]
            invoke(*flags,'run','explicit safe override');assert not (work/'profile-effect').exists()
            renamed.rename(defaults)
            set_default('restricted',0,'--project')
            invoke('run','workspace override survives global fallback');assert not (work/'profile-effect').exists()
            # Managed admission and saved-session reconstruction use the actual session workspace.
            managed=root/'managed'
            created=js('managed','--directory',str(managed),'--json','create')
            session_id=created['session']['id']
            invoke('managed','--directory',str(managed),'--json','submit',session_id,'--expected-revision','0','managed defaults denial')
            assert not (work/'profile-effect').exists()
            ordinary=max((root/'data/helm/sessions').glob('*.json'),key=lambda p:p.stat().st_mtime_ns)
            saved_id=json.loads(ordinary.read_text())['id']
            invoke('run','saved actual workspace defaults','--resume',saved_id,workspace=other)
            assert not (work/'profile-effect').exists() and not (other/'profile-effect').exists()
            # Relevant preference mutation after one response refuses a second canonical turn.
            Provider.on_final=lambda:js('policy','defaults','clear','--project','--expected-revision','1')
            before=len(Provider.requests)
            invoke('chat','--plain',input='first defaults turn\nrefused stale defaults second turn\n/exit\n')
            assert len(Provider.requests)==before+2
            assert all('refused stale defaults second turn' not in p.read_text() for p in (root/'data/helm/sessions').glob('*.json'))
            # Even equal rules at a new candidate revision cannot reuse a different
            # candidate's escalation receipt above the actual Config authority.
            readonly_config=root/'readonly-base.toml'
            readonly_config.write_text(enabled.read_text().replace('access="unrestricted"','access="read-only"'))
            set_default('autonomous',5)
            grant=js('policy','defaults','preview',config=readonly_config)
            js('policy','defaults','activate','--expected-revision','1','--confirm',grant['preview']['transition_digest'],config=readonly_config)
            invoke('run','exact candidate activated',config=readonly_config)
            assert (work/'profile-effect').exists();(work/'profile-effect').unlink()
            set_default('autonomous',6)
            before=len(Provider.requests)
            invoke('run','equal candidate cannot reuse grant',config=readonly_config,ok=False)
            assert len(Provider.requests)==before and not (work/'profile-effect').exists()
            fresh=js('policy','defaults','preview',config=readonly_config)
            assert fresh['preview']['requires_confirmation'] and not fresh['preview']['activation_current']
            js('policy','defaults','activate','--expected-revision','2','--confirm',fresh['preview']['transition_digest'],config=readonly_config)
            invoke('run','fresh candidate activation',config=readonly_config)
            assert (work/'profile-effect').exists();(work/'profile-effect').unlink()
            # Existing real nested-child assertions run unchanged through an anchored root config.
            set_default('restricted',7)
            wrapper=root/'defaults-helm'
            wrapper.write_text('#!'+sys.executable+'\nimport os,sys,tempfile,subprocess\nhelm='+repr(str(HELM))+'\nanchor='+repr(anchor)+'\nroot='+repr(str(root))+'\nargs=sys.argv[1:]\ni=args.index("--config")+1\nfd,output=tempfile.mkstemp(prefix="anchored-",suffix=".toml",dir=root);os.close(fd);os.unlink(output)\nresult=subprocess.run([helm,"--config",args[i],"policy","defaults","enable","--directory",anchor["directory"],"--store-id",anchor["store_id"],"--output",output],capture_output=True,text=True)\nif result.returncode:raise SystemExit(result.stderr)\nargs[i]=output\nos.execv(helm,[helm,*args])\n')
            wrapper.chmod(0o700)
            import subagent_resources
            subagent_resources.run_resources(wrapper)
            assert not Provider.failures,Provider.failures
            assert not any('defaults-config-secret-canary' in p.read_text() for p in (root/'data/helm/sessions').glob('*.json'))
            print('policy defaults: create-only anchor, restart, real denial, scoped clear activation, explicit override and TUI relaunch refusal passed')
    finally:server.shutdown();server.server_close()
if __name__=='__main__':main()
