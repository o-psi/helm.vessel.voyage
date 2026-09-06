#!/usr/bin/env python3
"""Real TUI voyage navigation retains native PTY identity and workspace authority."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import response
from policy_ceiling import rendered_screen

HELM=Path(os.environ.get('HELM_BIN','target/release/helm')).resolve()

class Provider(BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def do_POST(self):
        state=self.server.state
        body=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        prompt=next(m['content'] for m in reversed(body['messages']) if m['role']=='user')
        try:
            assert self.headers['Authorization']=='Bearer terminal-navigation-fixture'
            with state['lock']:
                state['requests'].append(body)
                step=state['steps'].get(prompt,0)+1;state['steps'][prompt]=step
            if prompt.startswith('seed-'):
                value='seeded '+prompt
            elif step==1:
                if prompt.startswith('start-'):
                    label=prompt[-1]
                    value=('process',{'action':'start','name':'persistent-'+label,'command':f'export NAV_STATE=state-{label}; printf "%s" "$$" > terminal.pid; exec /bin/sh'})
                elif prompt=='foreign-C':
                    value=('process',{'action':'write','id':state['ids']['A'],'data':'echo must-not-run > foreign-effect\n'})
                elif prompt.startswith('list-'):
                    value=('process',{'action':'list'})
                elif prompt.startswith('verify-'):
                    value=('process',{'action':'write','id':state['ids'][prompt[-1]],'data':'printf "%s|%s|%s" "$$" "$NAV_STATE" "$PWD" > terminal-state.txt\n'})
                else:raise AssertionError('unexpected prompt '+prompt)
            else:
                result=next(m['content'] for m in reversed(body['messages']) if m['role']=='tool')
                if prompt.startswith('start-'):
                    assert 'started PTY process ' in result,result
                    state['ids'][prompt[-1]]=result.rsplit(' ',1)[1]
                elif prompt=='foreign-C':assert 'unknown process' in result,result
                elif prompt=='list-C':assert state['ids']['A'] not in result,result
                elif prompt=='list-B':assert state['ids']['A'] in result,result
                elif prompt.startswith('verify-'):assert 'wrote ' in result,result
                value=prompt+'-done'
            data=response('openai-chat',value,len(state['requests']))
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        except Exception as error:state['failures'].append(repr(error));self.send_error(500)

class Tui:
    def __init__(self,args,env):
        self.output=bytearray();self.reaped=False
        self.pid,self.master=pty.fork()
        if self.pid==0:os.execve(str(HELM),[str(HELM),*args],env)
        fcntl.ioctl(self.master,termios.TIOCSWINSZ,struct.pack('HHHH',40,140,0,0))
    def wait(self,predicate,label,timeout=15):
        deadline=time.monotonic()+timeout
        while not predicate():
            assert time.monotonic()<deadline,(label,rendered_screen(bytes(self.output),40,140))
            if select.select([self.master],[],[],.05)[0]:
                try:self.output.extend(os.read(self.master,65536))
                except OSError:pass
    def text(self,value):self.wait(lambda:value in rendered_screen(bytes(self.output),40,140),value)
    def active(self,label):
        # Recent lists every name before navigation; only the main header proves
        # the target runtime and its terminal input mode are ready.
        self.text('HELM  access: unrestricted · NAVIGATION-'+label)
    def send(self,value):os.write(self.master,value if isinstance(value,bytes) else value.encode())
    def turn(self,prompt):
        self.send(prompt+'\r');self.text(prompt+'-done');self.text('Completed ·')
    def finish(self):
        self.send(b'\x11')
        def exited():
            done,status=os.waitpid(self.pid,os.WNOHANG)
            if done:self.reaped=True;assert os.waitstatus_to_exitcode(status)==0,status;return True
            return False
        self.wait(exited,'Helm exit and owned terminal cleanup')
    def close(self):
        if not self.reaped:os.kill(self.pid,signal.SIGKILL);os.waitpid(self.pid,0)
        os.close(self.master)

def identity(pid):
    value=Path(f'/proc/{pid}/stat').read_text()
    return value[value.rfind(')')+2:].split()[19]

def main():
    state={'lock':threading.Lock(),'steps':{},'ids':{},'requests':[],'failures':[]}
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider);server.state=state
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-terminal-navigation-') as tmp:
            root=Path(tmp);a=root/'workspace-a';c=root/'workspace-c';a.mkdir();c.mkdir()
            env=dict(os.environ,TERM='xterm-256color',HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),NAVIGATION_KEY='terminal-navigation-fixture')
            config=root/'provider.toml';config.write_text(f'provider="openai-chat"\nmodel="navigation-model"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="NAVIGATION_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\ncommand_timeout_secs=2\n')
            # A real MCP child can fail during target construction after its
            # persistent subagent runtime has acquired the workspace lease.
            gate=root/'fail-mcp';trace=root/'mcp-pids';mcp=root/'mcp.py'
            mcp.write_text("""import json,os,sys,time
from pathlib import Path
with open(sys.argv[2], 'a') as trace: trace.write(str(os.getpid())+'\\n')
for line in sys.stdin:
    request=json.loads(line)
    if 'id' not in request: continue
    if request['method']=='initialize':
        if Path(sys.argv[1]).exists() and Path(sys.argv[1]).read_text()=='initialize': time.sleep(30)
        result={}
    elif Path(sys.argv[1]).exists(): result={}
    else: result={'tools':[{'name':'probe','description':'fixture','inputSchema':{'type':'object'}}]}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
""")
            with config.open('a') as output:
                output.write('[mcp_servers.navigation]\ncommand='+json.dumps(sys.executable)+'\nargs='+json.dumps([str(mcp),str(gate),str(trace)])+'\n')
            profiles=root/'profiles'
            common=['--config',str(config),'--policy-directory',str(profiles)]
            def policy(*args):
                result=subprocess.run([str(HELM),*common,'policy',*args],env=env,capture_output=True,text=True,timeout=10)
                assert result.returncode==0,(result.stdout,result.stderr)
                return json.loads(result.stdout)
            anchor=policy('defaults','init','--directory',str(root/'defaults'))
            enabled=root/'enabled.toml'
            policy('defaults','enable','--directory',str(root/'defaults'),'--store-id',anchor['store_id'],'--output',str(enabled))
            common[1]=str(enabled)
            policy('list')
            selection=policy('inspect','restricted')
            sessions=root/'data/helm/sessions';ids={}
            for label,workspace in [('A',a),('B',a),('C',c)]:
                before=set(sessions.glob('*.json'))
                seed=subprocess.run([str(HELM),*common,'--workspace',str(workspace),'run','seed-'+label],env=env,capture_output=True,text=True,timeout=20)
                assert seed.returncode==0,(seed.stdout,seed.stderr)
                path,=set(sessions.glob('*.json'))-before
                record=json.loads(path.read_text());record['name']='NAVIGATION-'+label;path.write_text(json.dumps(record));ids[label]=record['id']
            ui=Tui([*common,'chat','--resume',ids['A']],env)
            pids={}
            try:
                ui.active('A');ui.turn('start-A')
                ui.wait(lambda:(a/'terminal.pid').exists(),'native terminal started')
                pids['A']=int((a/'terminal.pid').read_text());original=identity(pids['A'])
                ui.send('/resume '+ids['B']+'\r');ui.active('B')
                assert identity(pids['A'])==original,'same-workspace navigation terminated/replaced PTY'
                ui.turn('list-B')
                # A competing target owner cannot release the current voyage or
                # tear down its runtime while navigation waits for the lease.
                with open(sessions/('.'+ids['C']+'.lock'),'a') as lock:
                    fcntl.flock(lock,fcntl.LOCK_EX)
                    ui.send('/resume '+ids['C']+'\r');ui.text('Cannot switch voyages')
                    assert identity(pids['A'])==original
                for failure,notice in [('discover','failed to discover tools'),('initialize','initialization timed out')]:
                    before_mcp=len(trace.read_text().splitlines())
                    gate.write_text(failure)
                    ui.send('/resume '+ids['C']+'\r');ui.text(notice)
                    assert identity(pids['A'])==original
                    assert len(trace.read_text().splitlines())==before_mcp+1
                    failed_pid=int(trace.read_text().splitlines()[-1])
                    ui.wait(lambda:not Path(f'/proc/{failed_pid}').exists(),'failed candidate MCP cleanup')
                    gate.unlink()
                # Retry also proves the failed runtime released its workspace
                # writer lease, and the current voyage kept its execution fence.
                ui.send('/resume '+ids['C']+'\r');ui.active('C');ui.turn('list-C');ui.turn('foreign-C')
                assert not (a/'foreign-effect').exists(),'foreign workspace controlled retained terminal'
                ui.turn('start-C');ui.wait(lambda:(c/'terminal.pid').exists(),'second workspace native terminal')
                pids['C']=int((c/'terminal.pid').read_text());other=identity(pids['C'])
                assert identity(pids['A'])==original,'cross-workspace navigation destroyed inactive PTY'
                ui.send('/resume '+ids['A']+'\r');ui.active('A');ui.turn('verify-A')
                ui.wait(lambda:(a/'terminal-state.txt').exists(),'retained shell state')
                assert (a/'terminal-state.txt').read_text()==f"{pids['A']}|state-A|{a}"
                assert identity(pids['A'])==original and identity(pids['C'])==other
                ui.send('/resume '+ids['C']+'\r');ui.active('C')
                policy('defaults','set','--global','--profile-directory',str(profiles),'restricted','--revision','1','--digest',selection['digest'],'--expected-revision','0')
                ui.send('/resume '+ids['A']+'\r');ui.active('A')
                before_requests=len(state['requests'])
                ui.send('policy-must-not-dispatch\r');ui.text('Policy changed; restart or rebuild')
                assert len(state['requests'])==before_requests,'inactive cached policy dispatched after revision changed'
                assert identity(pids['A'])==original and identity(pids['C'])==other
                ui.finish()
                assert all(not Path(f'/proc/{pid}').exists() for pid in pids.values()),'Helm exit did not reap every retained PTY'
                assert not state['failures'],state['failures']
            finally:
                ui.close()
                for label,pid in pids.items():
                    try:
                        if identity(pid)==(original if label=='A' else other):os.killpg(pid,signal.SIGKILL)
                    except (FileNotFoundError,ProcessLookupError):pass
    finally:server.shutdown();server.server_close();thread.join(5)
    print('terminal navigation: same/different workspace PTY identity, shell state, foreign-write denial, busy target ownership, failed MCP discovery/initialization cleanup and retry, inactive policy freshness and complete exit cleanup passed')

if __name__=='__main__':main()
