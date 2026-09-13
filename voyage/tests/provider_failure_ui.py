"""Offline Helm failure-status regression; synthetic provider, no live inference."""
import argparse,sys,time,os,json
from pathlib import Path
os.umask(0o077)

from provider_attempts import Fixture, Handler, session, wait_for
from ui_journeys import launch,screen,send
from images_composer import stop_pty
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--bin-dir',type=Path,required=True)
args=parser.parse_args()
f=Fixture(args.bin_dir.resolve())
f.provider.RequestHandlerClass=Handler
f.provider.errors=[]
f.provider.times=[]
f.provider.scenario=('responses','text_idle')
ui=None
try:
 f.start()
 sid=session(f,'responses','text_idle')
 ui=launch([str(f.binaries/'helm'),'connect','--directory',str(f.directory),'--no-start'],{**f.env,'TERM':'xterm-256color'},f.workspace,f.root/'failure-status.pty',140,45)
 wait_for(lambda:'F2' in screen(ui),timeout=15)
 send(ui,'/use '+sid+'\r')
 time.sleep(.4)
 send(ui,'Check this synthetic partial response.\r')
 wait_for(lambda:len(f.provider.bodies)==1,timeout=15)
 saved=f.finished(sid)
 f.suspended(sid)
 wait_for(lambda:'Provider request timed out.' in screen(ui),timeout=15)
 # Let inventory pass its old freshness window after the owner suspends.
 time.sleep(6)
 shown=screen(ui)
 assert 'Request received.' in shown,shown
 assert 'Waiting for the result' not in shown,shown
 assert 'Checking programs' not in shown,shown
 assert 'Program status not current' in shown,shown
 assert 'Ready to continue' in shown,shown
 assert len(f.provider.bodies)==1
 assert saved['run']['provider_attempts'][-1]['decision']=='partial_response'
 f.record('failure-status',{'screen':shown,'requests':len(f.provider.bodies),'state':saved['run']['state']})
 print('PASS: actual Helm PTY submit, partial timeout, suspended inventory, truthful receipt and no retry',flush=True)
finally:
 if ui:stop_pty(ui,wait_for)
 f.close()
