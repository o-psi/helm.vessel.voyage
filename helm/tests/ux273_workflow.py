#!/usr/bin/env python3
"""Focused offline Linux #273 workflow/private-setup acceptance; no build/live account."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time
import traceback
import uuid
import ux272_journeys as base

SECRET = 'synthetic-explicit-ux273-secret-87'

def check_secret(root):
    for path in root.rglob('*'):
        if path.is_file():
            assert SECRET.encode() not in path.read_bytes(), f'synthetic secret persisted: {path}'

class Provider(base.Provider):
    def do_POST(self):
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            self.server.bodies.append(body)
            inputs = body.get('input', [])
            outputs = [x for x in inputs if x.get('type') == 'function_call_output']
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            if not outputs:
                # The provider knows only the public reference, never the value.
                command = "python3 -c 'import os,sys; v=os.environ[\"HELM_WORKFLOW_TOKEN\"]; print(v); print(v,file=sys.stderr); assert len(v)==34'"
                self.event({'type':'response.output_item.done','item':{'type':'function_call','call_id':'private-shell','name':'shell','arguments':json.dumps({'command':command,'workflow_secrets':['token']})}})
            else:
                self.event({'type':'response.output_text.delta','delta':'OFFLINE WORKFLOW COMPLETE'})
            self.event({'type':'response.completed','response':{'usage':{'input_tokens':1,'output_tokens':1}}})
            self.wfile.flush()
        except Exception as error:
            self.server.errors.append(repr(error))

def flow(j):
    f = j.f
    sid = f.session()
    # A separate explicitly unrestricted synthetic owner: execution policy still local.
    f.command(sid, {'op':'controls','run_id':None,'section':'policy'})
    j.note('Initial synthetic owner policy read')
    folder = f.workspace/'.helm/workflows'
    folder.mkdir(parents=True)
    definition = 'schema_version = 1\nid = "private-check"\nversion = "1"\ndescription = "Offline private check"\nprompt = "Consume {{word}} and {{token}} using explicit shell binding"\n[parameters.token]\ntype = "string"\nsecret = true\nrequired = true\n[parameters.word]\ntype = "integer"\nrequired = true\n'
    path = folder/'private-check.toml'; path.write_text(definition)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    ui = j.connect()
    base.command_text(ui, '/workflows')
    ui.until(lambda s:'private-check' in s.text(), 'workflow discovery')
    ui.send(b'\r'); ui.until(lambda s:'Definition (untrusted content)' in s.text(), 'trust preview')
    ui.send(b'\x1b'); base.settle(ui)
    assert not f.snapshot(sid)['messages']
    j.note('Repository trust preview Escape cancelled without command/history/provider effect')
    base.command_text(ui, '/workflows'); ui.until(lambda s:'private-check' in s.text(),'discovery again')
    ui.send(b'\r'); ui.until(lambda s:'Definition (untrusted content)' in s.text(),'trust again')
    ui.send(b'\x1b[6~'*20); base.settle(ui); ui.send(b't')
    ui.until(lambda s:'PRIVATE' in s.text(),'private input')
    base.paste(ui, SECRET); base.settle(ui)
    assert SECRET not in ui.screen.text()
    ui.send(b'\r'); ui.until(lambda s:'word' in s.text(),'required public input')
    ui.send(b'\r'); base.settle(ui)
    assert not f.snapshot(sid)['messages']
    ui.send(b'\x1b'); base.settle(ui)
    j.note('Missing required integer input did not submit; secret masked; Escape discarded form')
    assert not f.provider.bodies
    # A fresh private editor is dropped on terminal focus loss, not merely masked.
    base.command_text(ui,'/workflows');ui.until(lambda s:'private-check' in s.text(),'focus-loss workflow')
    ui.send(b'\r');ui.until(lambda s:'Definition (untrusted content)' in s.text(),'focus-loss trust')
    ui.send(b'\x1b[6~'*20);base.settle(ui);ui.send(b't')
    ui.until(lambda s:'PRIVATE' in s.text(),'focus-loss private input')
    base.paste(ui,SECRET);base.settle(ui);ui.send(b'\x1b[O');base.settle(ui)
    assert 'Input 1/2' not in ui.screen.text(),ui.screen.text()
    ui.send(b'\x1b[I');base.settle(ui)
    assert not f.snapshot(sid)['messages']
    j.note('Workflow focus loss discarded private editor without submission')
    preview = {'op':'workflow_preview','id':'private-check','scope':'repository','inputs':[['word','7']],'trust_digest':digest}
    # Unknown schema and lack of trust are fail-closed on current runtime, not mock validation.
    response=f.command(sid,dict(preview,trust_digest=None),envelope=True)
    assert response.get('error'),response
    f.record('untrusted-preview',response)
    bad = folder/'unknown.toml'; bad.write_text('schema_version = 999\nid = "unknown"\nversion = "1"\ndescription = "Unknown schema"\nprompt = "never"\n')
    for command in [dict(preview, trust_digest=None),dict(preview,id='unknown')]:
        response = f.command(sid,command,envelope=True)
        assert response.get('error'), response
        f.record('preview-refusal-'+command['id'],response)
    j.note('Runtime rejected untrusted repository preview and unknown workflow schema')
    bad.unlink()  # Invalid discovered definition blocks collection until corrected.
    rendered = f.command(sid,preview)
    assert SECRET not in json.dumps(rendered)
    handle = str(uuid.uuid4())
    stored = f.command(sid,{'op':'workflow_inputs','input_id':handle,'values':[['token',SECRET]]})
    assert stored['storage']=='volatile'
    command = dict(preview,op='workflow_submit',command_id=str(uuid.uuid4()),expected_revision=f.snapshot(sid)["revision"]+100,expires_at_ms=int(time.time()*1000)+60000,private_inputs_id=handle)
    response = f.command(sid,command,envelope=True)
    f.record('stale-workflow-receipt',response)
    assert response.get('result',response).get('status') == 'rejected', response
    assert not f.snapshot(sid)['messages']
    retry = dict(command,command_id=str(uuid.uuid4()),expected_revision=f.snapshot(sid)['revision'])
    response = f.command(sid,retry,envelope=True)
    assert response.get('error'), response
    j.note('Stale dispatch rejected; consumed private input handle could not be replayed')
    # New input is explicitly supplied after refusal, never auto-replayed.
    handle = str(uuid.uuid4())
    f.command(sid,{'op':'workflow_inputs','input_id':handle,'values':[['token',SECRET]]})
    retry.update(command_id=str(uuid.uuid4()),private_inputs_id=handle,expected_revision=f.snapshot(sid)['revision'])
    receipt = f.command(sid,retry)
    assert receipt['state']=='accepted', receipt
    snap = j.finish(sid)
    f.record('workflow-complete',snap)
    assert any('OFFLINE WORKFLOW COMPLETE' in m.get('content','') for m in snap['messages']), snap
    text = json.dumps(snap)
    assert SECRET not in text
    outputs = [m for m in snap['messages'] if m.get('role')=='tool']
    assert outputs and any(json.loads(m.get('content','{}')) == {'status':'exited','code':0} for m in outputs), outputs
    assert not f.provider.errors,f.provider.errors
    assert SECRET not in json.dumps(f.provider.bodies)
    j.note('Private transport accepted once; real shell explicitly bound token; stdout/stderr suppressed before provider/history; cleanup complete')
    check_secret(f.root)


def forms(j):
    f=j.f;sid=f.session();ui=j.connect()
    base.command_text(ui,'/vessels')
    ui.until(lambda s:'Add HTTPS' in s.text(),'Vessel connection manager')
    ui.send(b'a');ui.until(lambda s:'Owner pairing invitation (masked)' in s.text(),'pairing setup')
    ui.send(b'\t');base.paste(ui,SECRET);base.settle(ui)
    assert SECRET not in ui.screen.text()
    assert any('Owner pairing invitation' in line and '••' in line for line in ui.screen.text().splitlines()),ui.screen.text()
    ui.send(b'\x1b');base.settle(ui)
    ui.send(b'a');ui.until(lambda s:'Owner pairing invitation (masked)' in s.text(),'fresh pairing setup')
    assert not any('Owner pairing invitation' in line and '••' in line for line in ui.screen.text().splitlines()),'private field survived closing form'
    ui.send(b'\x1b');base.settle(ui);ui.send(b'\x1b');base.settle(ui)
    assert not f.snapshot(sid)['messages']
    j.note('Connection pairing private field masked; Escape clears form and reopened field; no connection dispatched')
    base.command_text(ui,'/actions');ui.until(lambda s:'Search' in s.text(),'Actions');ui.send(b'Run an operator tool');base.settle(ui);ui.send(b'\r')
    ui.until(lambda s:'Search actions:' in s.text(),'live operator registry')
    ui.send(b'shell');base.settle(ui);ui.send(b'\r')
    ui.until(lambda s:'Field 1 of 2' in s.text(),'live shell form')
    ui.send(b'\t');base.settle(ui)
    ui.until(lambda s:'workflow_secrets' in s.text(),'private reference unavailable')
    assert 'unsupported/private' in ui.screen.text(),ui.screen.text()
    ui.send(b'\x1b[6~'*20);base.settle(ui)
    ui.send(b'\t'*20+b'\r');base.settle(ui)
    assert not f.snapshot(sid)['messages']
    ui.send(b'\x1b');base.settle(ui);ui.send(b'\x1b');base.settle(ui)
    j.note('Live shell operator form labels private references unavailable; blank command review/cancel dispatched nothing')
    check_secret(f.root)
    check_secret(j.output)

def setup(j):
    f=j.f
    for key, label in [(b'\x1b','Escape'),(b'\x03','Ctrl-C')]:
        ui=base.private.OuterPTY([str(f.binaries/'vessel'),'auth','accounts','setup','--provider','openai','--account','cancel-only'],f.env,f.workspace,j.output/f'setup-{label}.pty')
        j.clients.append(ui)
        ui.until(lambda s:'API key (hidden' in s.text(),'private API key prompt')
        ui.send(SECRET.encode()); base.settle(ui)
        assert SECRET not in ui.screen.text()
        ui.send(key); ui.exited_restored()
        j.note(f'Execution-host account setup {label}: synthetic key hidden, exit 0, termios restored')
    result=subprocess.run([str(f.binaries/'vessel'),'auth','accounts','list'],env=f.env,cwd=f.workspace,capture_output=True,text=True,timeout=15)
    assert result.returncode==0,result.stderr
    assert 'cancel-only' not in result.stdout
    assert not f.provider.bodies
    check_secret(f.root)
    j.note('Cancelled account metadata absent; no provider request; no synthetic secret persisted')

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bin-dir',type=Path,required=True);p.add_argument('--build-manifest',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
    a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
    manifest=json.loads(a.build_manifest.read_text()); hashes={n:base.sha(a.bin_dir/n) for n in ('helm','vessel','voyage')}
    assert hashes==manifest['binary_sha256']
    # Keep fixture account initialization and real start_settings; only synthetic provider differs.
    base.Provider=Provider
    results=[]
    for name,case in [('setup',setup),('forms',forms),('workflow',flow)]:
        out=a.output/name;out.mkdir(parents=True,exist_ok=True)
        f=base.Fixture(a.bin_dir.resolve());f.unrestricted=True
        j=base.Journey(f,a.bin_dir.resolve(),out)
        result={'case':name,'status':'failed','checks':j.observations,'fixture':str(j.f.root)}
        try:
            f.start();f.seed_account();case(j); result['status']='passed'
        except Exception: result['error']=traceback.format_exc()
        finally:
            result['cleanup_errors']=j.close()
            if result['cleanup_errors']:result['status']='failed'
            try: check_secret(f.root);check_secret(out)
            except Exception:
                result['status']='failed';result['scan_error']=traceback.format_exc()
        results.append(result); print(json.dumps(result,indent=2),flush=True)
    assert hashes=={n:base.sha(a.bin_dir/n) for n in hashes},'binaries changed'
    (a.output/'result.json').write_text(json.dumps({'manifest':manifest,'binary_sha256':hashes,'results':results},indent=2))
    return int(any(x['status']!='passed' for x in results))

if __name__=='__main__': raise SystemExit(main())
