"""Scoped #273 synthetic end-to-end evidence; actual Helm/Vessel/Voyage/local Chromium.
No paid provider, personal profile, general test recreation or installed-service mutation.
"""
import socket
import os,sys,json,time,uuid,threading,subprocess,signal,urllib.request,urllib.error,http.server,pathlib,tempfile,re,html,pty,fcntl,termios,struct,sqlite3
urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
ROOT=pathlib.Path(__file__).resolve().parents[2]
os.umask(0o077)
(ROOT/'target/ux273-browser').mkdir(parents=True,exist_ok=True)
E=pathlib.Path(tempfile.mkdtemp(prefix='browser-integration-',dir=ROOT/'target/ux273-browser'));E.chmod(0o700)
print('Evidence:',E,flush=True)
BIN=pathlib.Path(os.environ['BROWSER_PROBE_BIN']).resolve()
# Verify the parent's serialized build attestation before running product binaries.
import hashlib,shutil
manifest=json.loads(pathlib.Path(os.environ['BROWSER_PROBE_MANIFEST']).read_text())
for name,digest in manifest['binary_sha256'].items():
 assert hashlib.file_digest((BIN/name).open('rb'),'sha256').hexdigest()==digest, name+' build identity mismatch'
(E/'build-manifest.json').write_text(json.dumps(manifest,indent=2))
node=subprocess.check_output(['node','-p','process.execPath'],text=True).strip();assert pathlib.Path(node).is_file()
env={'PATH':str(pathlib.Path(node).parent)+':/usr/bin:/bin','LANG':'C.UTF-8'}
(E/'tmp').mkdir(mode=0o700);env['TMPDIR']=str(E/'tmp')
for k in ['HOME','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME']:
 p=E/k.lower();p.mkdir(mode=0o700);env[k]=str(p)
stub=E/'bin';stub.mkdir();(stub/'xdg-open').write_text('#!/bin/sh\nexit 0\n');(stub/'xdg-open').chmod(0o700);env['PATH']=str(stub)+':'+env['PATH']
work=E/'workspace';work.mkdir();directory=pathlib.Path(tempfile.mkdtemp(prefix='b-',dir=os.environ['BROWSER_PROBE_RUNTIME_ROOT']));(E/'vessel-path.txt').write_text(str(directory));(directory/'tmp').mkdir(mode=0o700);env['TMPDIR']=str(directory/'tmp');bodies=[];stage=0;mode=None;target=None;element=None;errors=[];children=[]
def wait(f,seconds=30):
 end=time.monotonic()+seconds;last=None
 while time.monotonic()<end:
  try:
   v=f()
   if v:return v
  except Exception as ex:last=ex
  time.sleep(.1)
 raise AssertionError('wait expired: '+str(last))
class Website(http.server.BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  text='PRIVATE_BROWSER_MARKER_234' if self.path=='/private' else '<button onclick="this.textContent=\'Clicked once\'">Browser test button</button>'
  data=('<!doctype html><title>Fixture browser</title><h1>Shared local fixture</h1>'+text).encode();self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
class Provider(http.server.BaseHTTPRequestHandler):
 def log_message(self,*a):pass
 def do_GET(self):
  data=json.dumps({'data':[{'id':'gpt-4o','input_modalities':['text','image']}]}).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def do_POST(self):
  global stage,target,element,mode
  body=json.loads(self.rfile.read(int(self.headers['Content-Length'])));bodies.append(body)
  if not body.get('tools'):
   output=[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Fixture Browser Task'}]}]
   payload=('data: '+json.dumps({'type':'response.completed','response':{'id':'title-fixture-'+str(len(bodies)),'status':'completed','output':output,'usage':{'input_tokens':1,'output_tokens':1}}})+'\n\n').encode();self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload);return
  try:
   encoded=json.dumps(body.get('input',[])); users=[json.dumps(x.get('content','')) for x in body.get('input',[]) if x.get('role')=='user'];
   if users:mode='private' if 'private-browser-check' in users[-1] else 'main' if 'main-browser-check' in users[-1] else 'other'
   outputs=[x for x in body.get('input',[]) if x.get('type')=='function_call_output']
   if mode=='main':
    if stage==0: action={'action':'inspect','page_id':None}
    elif stage==1:
     meta=json.loads(outputs[-1]['output']);obs=json.loads(meta['text']);target={'page_id':obs['page_id'],'observation_id':obs['observation_id']};element=next(x['ref'] for x in obs['refs'] if x['label']=='Browser test button');action={'action':'screenshot','target':target}
    elif stage==2:
     blocks=outputs[-1]['output'];assert isinstance(blocks,list),blocks;assert any(x.get('type')=='input_image' and x['image_url'].startswith('data:image/jpeg;base64,') for x in blocks);action={'action':'click','target':target,'element':element}
    elif stage==3: action={'action':'inspect','page_id':target['page_id']}
    else:
     meta=json.loads(outputs[-1]['output']);assert 'Clicked once' in meta['text'];action=None
    stage+=1
    output=[{'type':'function_call','call_id':'browser-fixture-'+str(stage),'name':'browser','arguments':json.dumps(action)}] if action else [{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Shared browser verified.'}]}]
   elif mode=='private' and not any(x.get('call_id')=='private-fixture' for x in outputs):
    output=[{'type':'function_call','call_id':'private-fixture','name':'browser','arguments':json.dumps({'action':'inspect','page_id':None})}]
   else: output=[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Fixture done.'}]}]
   assert 'PRIVATE_BROWSER_MARKER_234' not in json.dumps(body)
  except Exception as ex:
   errors.append(repr(ex));output=[{'type':'message','role':'assistant','content':[{'type':'output_text','text':'Fixture assertion failed.'}]}]
  payload=('data: '+json.dumps({'type':'response.completed','response':{'id':'fixture-'+str(len(bodies)),'status':'completed','output':output,'usage':{'input_tokens':1,'output_tokens':1}}})+'\n\n').encode();self.send_response(200);self.send_header('Content-Type','text/event-stream');self.send_header('Content-Length',str(len(payload)));self.end_headers();self.wfile.write(payload)
provider=http.server.ThreadingHTTPServer(('127.0.0.1',0),Provider);site=http.server.ThreadingHTTPServer(('127.0.0.1',0),Website)
for server in [provider,site]:threading.Thread(target=server.serve_forever,daemon=True).start()
site_url='http://127.0.0.1:'+str(site.server_port)
def cli(*args,timeout=25):
 p=subprocess.run([str(BIN/'helm'),'connect','--directory',str(directory),'--no-start',*args],env=env,cwd=work,text=True,capture_output=True,timeout=timeout)
 assert p.returncode==0,(args,p.stdout,p.stderr)
 return json.loads(p.stdout)
def request(command):
 for attempt in range(20):
  c=json.loads((directory/'process-http.json').read_text());req=urllib.request.Request(c['endpoint']+'/v1/vessel/command',data=json.dumps({'protocol':1,'command':command}).encode(),headers={'Authorization':'Bearer '+c['token'],'Content-Type':'application/json'});v=json.load(urllib.request.urlopen(req,timeout=20))
  if v.get('error') and 'locked' in v['error'] and command['op'] in ['snapshot','inspect','catalogue'] and attempt<19:
   time.sleep(.025);continue
  assert v.get('error') is None,v
  return v['result']

def session_command(s,op):return request({'session_id':s,**op})['result']
def submit(s,prompt):
 snap=session_command(s,{'op':'snapshot'});return session_command(s,{'op':'submit','command_id':str(uuid.uuid4()),'expected_revision':snap['revision'],'expires_at_ms':int(time.time()*1000)+60000,'prompt':prompt})
def terminal(s,run_id=None):
 snap=session_command(s,{'op':'snapshot'});return snap if (run_id is None or snap.get('run',{}).get('run_id')==run_id) and snap.get('run',{}).get('state') in ['completed','failed','cancelled','incomplete'] and snap.get('pending_cleanup_run') is None else None
def remote_sockets(pid):
 inodes=set()
 for fd in pathlib.Path('/proc',str(pid),'fd').iterdir():
  try:
   link=os.readlink(fd)
   if link.startswith('socket:['):inodes.add(link[8:-1])
  except FileNotFoundError:pass
 port=int(json.loads((directory/'process-http.json').read_text())['endpoint'].rsplit(':',1)[1])
 return sorted(row.split()[9] for row in pathlib.Path('/proc/net/tcp').read_text().splitlines()[1:] if row.split()[9] in inodes and int(row.split()[2].split(':')[1],16)==port)
env['UX273_FIXTURE_KEY']='synthetic-not-a-real-key'
connection=json.loads(subprocess.check_output([str(BIN/'vessel'),'auth','accounts','connect','--label','X14 loopback fixture','--endpoint',f'http://127.0.0.1:{provider.server_port}/v1','--transports','openai-responses'],env=env,cwd=work,timeout=15))
account=json.loads(subprocess.check_output([str(BIN/'vessel'),'auth','accounts','add','--connection',connection['id'],'--account','ux273-fixture','--env','UX273_FIXTURE_KEY'],env=env,cwd=work,timeout=15))
base=cookie=csrf=None;epoch=1;cid=str(uuid.uuid4());master=None;tui=os.environ.get('BROWSER_PROBE_TUI')=='1'
def local(data,endpoint='/api'):
 global epoch
 req=urllib.request.Request(base+endpoint,data=json.dumps({'controller_id':cid,'epoch':epoch,**data}).encode(),headers={'Origin':base,'Content-Type':'application/json',**({'Cookie':cookie} if cookie else {}),**({'X-Helm-CSRF':csrf} if csrf else {})});r=urllib.request.urlopen(req,timeout=15);v=json.load(r);epoch=v.get('epoch',epoch);return v,r.headers
try:
 with (E/'setup.log').open('w') as log:
  p=subprocess.run([str(BIN/'helm'),'browser','setup'],env=env,cwd=work,stdout=log,stderr=log,timeout=150);assert p.returncode==0
 log=(E/'vessel.log').open('w');vessel=subprocess.Popen([str(BIN/'vessel'),'local-serve','--directory',str(directory),'--voyage-binary',str(BIN/'voyage')],env=env,cwd=work,stdout=log,stderr=log);children.append(vessel);wait(lambda:(directory/'process-http.json').exists());wait(lambda:request({'op':'catalogue'}) is not None)
 binding={'account_id':account['id'],'connection_id':connection['id'],'identity_generation':account['identity_generation'],'connection_revision':connection['revision'],'transport':'openai_responses'}
 request({'op':'account_set_default','command_id':str(uuid.uuid4()),'workspace':str(work),'account':binding,'expected_revision':0})
 s=str(uuid.uuid4());request({'op':'start_settings','command_id':str(uuid.uuid4()),'session_id':s,'workspace':str(work),'settings':{'model':'gpt-4o','context_window':0,'command_timeout_secs':60,'access_mode':'unrestricted'},'binding':binding});submit(s,'bootstrap fixture');assert wait(lambda:terminal(s))['run']['state']=='completed';wait(lambda:request({'op':'inspect','session_id':s})['state']=='suspended');print('PASS suspended starting point',flush=True)
 blog=(E/'browser.log').open('wb')
 args=[str(BIN/'helm'),'connect','--directory',str(directory),'--no-start']
 if tui:
  master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,140,0,0));env['TERM']='xterm-256color';browser=subprocess.Popen(args,env=env,cwd=work,stdin=slave,stdout=slave,stderr=slave);os.close(slave)
  def drain():
   try:
    while True:
     b=os.read(master,65536)
     if not b:break
     blog.write(b);blog.flush()
   except OSError:pass
  threading.Thread(target=drain,daemon=True).start();time.sleep(3);os.write(master,b'unsent draft marker');os.write(master,b'\x1b[17~')
 else:browser=subprocess.Popen(args+['browser',s],env=env,cwd=work,stdout=blog,stderr=blog)
 children.append(browser)
 launch=wait(lambda:next(pathlib.Path(env['XDG_STATE_HOME']).glob('voyage/helm-browser/session-*/open.html'),None),40);launch_text=launch.read_text();url=html.unescape(re.search(r'url=([^\"]+)',launch_text)[1]);base=url.split('/#')[0];secret=url.split('#')[1]
 auth,headers=local({'secret':secret},'/session');cookie=headers['Set-Cookie'].split(';')[0];csrf=auth['csrf'];local({'op':'claim'});local({'op':'origin','url':site_url,'private_network':True});local({'op':'navigate','url':site_url});local({'op':'mode','mode':'agent','confirm_share':True});
 wait(lambda:json.loads(sqlite3.connect(directory/'sessions'/s/'journal/journal.sqlite3').execute('select state from browser_state where session_id=?',(s,)).fetchone()[0])['offer']['control']=='shared',30) if tui else wait(lambda:'Agent sharing enabled' in (E/'browser.log').read_text(),30);print('PASS explicit share resumes suspended voyage without replacing session',flush=True)
 socket_identity=remote_sockets(browser.pid);assert len(socket_identity)==1,socket_identity
 [(time.sleep(.5),local({'op':'state'})) for _ in range(6)]
 main_run=submit(s,'main-browser-check')['run_id']
 end=time.monotonic()+90;done=None
 while time.monotonic()<end:
  state,_=local({'op':'state'})
  for prompt in state['prompts']:local({'op':'confirm','prompt_id':prompt['id'],'allow':True})
  done=terminal(s,main_run)
  if done:break
  time.sleep(.15)
 assert done and done['run']['state']=='completed',(done,errors,(E/'browser.log').read_text());assert stage==5,(stage,errors);assert not errors,errors
 assert any(m.get('tool_output') and any(x.get('type')=='image' for x in m['tool_output']['content']) for m in done['messages']);print('PASS remote tool inspect/screenshot/click and real provider image pixels with canonical artifact',flush=True)
 local({'op':'mode','mode':'private'});local({'op':'navigate','url':site_url+'/private'});[(time.sleep(.5),local({'op':'state'})) for _ in range(14)];assert browser.poll() is None,(E/'browser.log').read_text();local({'op':'state'});private_run=submit(s,'private-browser-check')['run_id'];private=wait(lambda:terminal(s,private_run),30);assert private['run']['state']=='completed';assert any(x.get('call_id')=='private-fixture' for b in bodies for x in b.get('input',[]));assert 'PRIVATE_BROWSER_MARKER_234' not in json.dumps(bodies);print('PASS private-mode lease remains usable and model receives no private page content',flush=True)
 assert remote_sockets(browser.pid)==socket_identity;print('PASS commands, lease control and delayed browser-work notifications use one unchanged duplex socket',flush=True)
 os.write(master,b'\x11') if tui else browser.send_signal(signal.SIGINT);browser.wait(timeout=25);assert browser.returncode==0,(E/'browser.log').read_text();last_run=submit(s,'after browser disconnect')['run_id'];assert wait(lambda:terminal(s,last_run))['run']['state']=='completed';print('PASS closing browser does not cancel or destroy voyage',flush=True)
 if tui:
  assert 'unsent draft marker' not in json.dumps(bodies)
  assert any('unsent draft marker' in p.read_text(errors='ignore') for p in pathlib.Path(env['XDG_STATE_HOME']).rglob('*.json') if 'node_modules' not in str(p))
  print('PASS actual F6 TUI launch, quit cleanup and preserved unsent draft',flush=True)
 wait(lambda:request({'op':'inspect','session_id':s})['state']=='suspended');before=len(bodies)
 reconciled=subprocess.run([str(BIN/'helm'),'connect','--directory',str(directory),'--no-start','browser',s,'--reconcile'],env=env,cwd=work,text=True,capture_output=True,timeout=30)
 (E/'reconcile.json').write_text(json.dumps({'code':reconciled.returncode,'stdout':reconciled.stdout,'stderr':reconciled.stderr}))
 assert reconciled.returncode==0,(reconciled.stdout,reconciled.stderr)
 assert 'Reconciled ' in reconciled.stdout and len(bodies)==before
 print('PASS cleanup reconciliation follows suspended owner with old inner bindings and no model/effect replay',flush=True)
 (E/'provider-bodies.json').write_text(json.dumps(bodies,indent=2));(E/'snapshot.json').write_text(json.dumps(done,indent=2));(E/'result.json').write_text(json.dumps({'passed':True,'session':s,'stage':stage}));print('ALL PASS',flush=True)
finally:
 (E/'provider-bodies.json').write_text(json.dumps(bodies,indent=2))
 (E/'errors.json').write_text(json.dumps(errors))
 # Stop the local Helm browser first, while its supervising Vessel is reachable.
 for p in reversed(children[1:]):
  if p.poll() is None:
   if tui and master is not None:os.write(master,b'\x11')
   else:p.send_signal(signal.SIGINT)
  try:p.wait(timeout=20)
  except subprocess.TimeoutExpired:p.kill();p.wait()
 if children and children[0].poll() is None and (directory/'process-http.json').exists():
  try:
   for info in request({'op':'catalogue'}):
    try:request({'op':'stop','session_id':info['session_id'],'incarnation':info['incarnation']})
    except Exception:pass
  except Exception:pass
 for p in children[:1]:
  if p.poll() is None:p.terminate()
  try:p.wait(timeout=10)
  except subprocess.TimeoutExpired:p.kill();p.wait()
 provider.shutdown();site.shutdown();provider.server_close();site.server_close()
 # Observe exact owned fixtures; do not delete locks/receipts or claim process exit
 # resolved a remote cleanup obligation. Preserve private evidence for diagnosis.
 markers=[str(E).encode(),str(directory).encode()];remaining=[]
 for proc in pathlib.Path('/proc').glob('[0-9]*'):
  if int(proc.name)==os.getpid():continue
  try:
   cmd=(proc/'cmdline').read_bytes()
   if any(marker in cmd for marker in markers):remaining.append(int(proc.name))
  except (FileNotFoundError,ProcessLookupError,PermissionError):pass
 ports=[provider.server_port,site.server_port]
 if base:ports.append(urllib.parse.urlparse(base).port)
 listeners=[]
 for port in ports:
  with socket.socket() as sock:
   sock.settimeout(.3)
   if sock.connect_ex(('127.0.0.1',port))==0:listeners.append(port)
 cleanup={'child_exit_codes':[p.poll() for p in children],'matching_live_processes':remaining,'remaining_loopback_listeners':listeners,'executor_locks_remaining':len(list(pathlib.Path(env['XDG_STATE_HOME']).glob('voyage/helm-browser/session-*/executor.lock')))}
 (E/'cleanup-audit.json').write_text(json.dumps(cleanup,indent=2))
 assert not remaining and not listeners and all(p.poll() is not None for p in children),cleanup
 print('PASS exact fixture process/listener cleanup observed (receipts retained)',flush=True)
