#!/usr/bin/env python3
"""Actual configured plain-chat/provider/PTY/canonical-history boundary (offline)."""
import fcntl,json,os,pty,re,select,signal,struct,subprocess,tempfile,termios,threading,time,uuid
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from completion_gate import response
HELM=Path(os.environ.get('HELM_BIN','target/release/helm')).resolve()
CSI=re.compile(rb'\x1b\[[0-?]*[ -/]*[@-~]')
CANARY='PRIVATE_CANARY_秘密'
class Provider(BaseHTTPRequestHandler):
    def log_message(self,*_):pass
    def do_POST(self):
        state=self.server.state
        try:
            assert self.headers['Authorization']=='Bearer plain-attachment-fixture'
            size=int(self.headers['Content-Length']);assert size<2*1024*1024
            body=json.loads(self.rfile.read(size));state['requests'].append(body)
            assert CANARY not in json.dumps(body,ensure_ascii=False),'private input reached provider'
            prompt=next(m['content'] for m in reversed(body['messages']) if m['role']=='user')
            step=state['steps'].get(prompt,0)+1;state['steps'][prompt]=step
            if step==1:
                if prompt=='start':value=('process',{'action':'start','name':'live','command':'stty -echo; printf "%s" "$$" > terminal.pid; exec /bin/sh'})
                elif prompt=='inspect🧭':value=('process',{'action':'read','id':state['id']})
                elif prompt=='other':value=('process',{'action':'start','name':'model-owned','command':'printf PUBLIC_VISIBLE; exec /bin/sh'})
                elif prompt=='read-other':value=('process',{'action':'read','id':state['other']})
                else:raise AssertionError('unexpected prompt '+repr(prompt))
            else:
                result=next(m['content'] for m in reversed(body['messages']) if m['role']=='tool')
                if prompt in ['start','other']:
                    assert 'started PTY process ' in result,result
                    state['id' if prompt=='start' else 'other']=result.rsplit(' ',1)[1]
                elif prompt=='inspect🧭':assert 'model capture unavailable' in result and 'private' in result,result
                elif prompt=='read-other':assert 'PUBLIC_VISIBLE' in result,result
                value=prompt+'-done'
            data=response('openai-chat',value,len(state['requests']))
            self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
        except Exception as error:state['failures'].append(repr(error));self.send_error(500)
class Plain:
    def __init__(self,args,env):
        self.master,self.slave=pty.openpty();self.initial=termios.tcgetattr(self.slave);self.output=bytearray();self.deadline=time.monotonic()+60
        fcntl.ioctl(self.slave,termios.TIOCSWINSZ,struct.pack('HHHH',25,100,0,0))
        def setup():os.setsid();fcntl.ioctl(self.slave,termios.TIOCSCTTY,0)
        self.process=subprocess.Popen([str(HELM),*args],stdin=self.slave,stdout=self.slave,stderr=self.slave,env=env,preexec_fn=setup)
    def wait(self,text,after=0):
        expected=text.encode()
        while expected not in CSI.sub(b'',self.output[after:]):
            assert time.monotonic()<self.deadline,(text,bytes(self.output[-5000:]))
            if select.select([self.master],[],[],.03)[0]:
                try:self.output.extend(os.read(self.master,65536))
                except OSError:pass
            assert len(self.output)<8*1024*1024
            assert self.process.poll() is None or expected in CSI.sub(b'',self.output[after:]),bytes(self.output[-5000:])
    def send(self,text):os.write(self.master,text.encode() if isinstance(text,str) else text)
    def command(self,text,result):
        offset=len(self.output);self.send(text+'\n');self.wait(result,offset);return offset
    def turn(self,text):self.command(text,text+'-done')
    def close(self):
        if self.process.poll() is None:os.killpg(self.process.pid,signal.SIGKILL);self.process.wait(timeout=5)
        os.close(self.master);os.close(self.slave)
def main():
    state={'requests':[],'steps':{},'failures':[]}
    server=ThreadingHTTPServer(('127.0.0.1',0),Provider);server.state=state
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-plain-chat-') as tmp:
            root=Path(tmp);workspace=root/'workspace';workspace.mkdir()
            config=root/'config.toml';config.write_text(f'provider="openai-chat"\nmodel="plain-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="PLAIN_FIXTURE_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\n')
            env=dict(os.environ,HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),PLAIN_FIXTURE_KEY='plain-attachment-fixture',TERM='xterm-256color')
            plain=Plain(['--config',str(config),'--workspace',str(workspace),'chat','--plain'],env)
            try:
                plain.wait('helm>');plain.command('/name PLAIN-A','session renamed')
                plain.command('/terminals','No live terminals');assert not state['requests']
                offset=len(plain.output);plain.send('/terminal missing\n'+CANARY+'\n')
                plain.wait('No queued prompt was submitted',offset);assert not state['requests']
                plain.turn('start');pid=int((workspace/'terminal.pid').read_text());assert Path(f'/proc/{pid}').exists()
                count=len(state['requests']);offset=len(plain.output)
                plain.send('/terminal '+str(uuid.uuid4())+'\n'+CANARY+'\n')
                plain.wait('No queued prompt was submitted',offset);assert len(state['requests'])==count
                plain.command('/terminal live','Ctrl+T/Ctrl+] detach')
                plain.command(f"printf '%s' '{CANARY}' > human.txt; printf SCREEN_READY",'SCREEN_READY')
                offset=len(plain.output);plain.send('\x14inspect🧭\n');plain.wait('inspect🧭-done',offset)
                assert (workspace/'human.txt').read_text()==CANARY
                assert Path(f'/proc/{pid}').exists(),'detach killed inner process'
                plain.turn('other');plain.turn('read-other')
                count=len(state['requests']);plain.command('/new PLAIN-B','new session: PLAIN-B')
                plain.command('/terminals',state['id']);assert Path(f'/proc/{pid}').exists();assert len(state['requests'])==count
                plain.command('/terminal '+state['id'],'Ctrl+T/Ctrl+] detach')
                plain.command('printf SWITCH_SURVIVED','SWITCH_SURVIVED')
                plain.send(b'\x1d/exit\n');plain.process.wait(timeout=10)
                assert plain.process.returncode==0
                assert termios.tcgetattr(plain.slave)==plain.initial,'outer terminal not restored'
                assert not Path(f'/proc/{pid}').exists(),'Helm exit did not reap terminal'
                records=list((root/'data').glob('helm/sessions/*.json'));assert records,'no canonical session records'
                for record in records:
                    assert record.stat().st_size<4*1024*1024
                    assert CANARY not in record.read_text(),'private input persisted canonically'
                assert not state['failures'],state['failures']
                assert len(state['requests'])==8,state['requests']
                print('PASS actual plain chat ownership, privacy, exact suffix, unrelated capture, voyage switch, and cleanup',flush=True)
            except BaseException:
                destination=os.environ.get('HELM_PLAIN_TERMINAL_EVIDENCE')
                if destination:
                    evidence=Path(destination)/str(uuid.uuid4());evidence.mkdir(parents=True)
                    data=json.dumps(state,ensure_ascii=False,indent=2).encode();assert len(data)<=8*1024*1024
                    (evidence/'provider.json').write_bytes(data)
                    (evidence/'terminal.bin').write_bytes(plain.output)
                    saved=evidence/'sessions';saved.mkdir();total=0
                    for record in (root/'data').glob('helm/sessions/*.json'):
                        size=record.stat().st_size;total+=size;assert total<=8*1024*1024
                        (saved/record.name).write_bytes(record.read_bytes())
                    print('Synthetic plain fixture failure retained at '+str(evidence),flush=True)
                raise
            finally:plain.close()
    finally:server.shutdown();server.server_close()
if __name__=='__main__':main()
