#!/usr/bin/env python3
"""Focused #273 X02/X03/X09/X10 synthetic Linux PTY journeys; no builds/providers.
Uses the surviving #272 fixture, not retired broad fixture modules.
Run with --bin-dir, --build-manifest and unique --output (same as ux272_journeys).
"""
import base64
import importlib.util
import json
import os
from pathlib import Path
import time
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('ux272', ROOT/'helm/tests/ux272_journeys.py')
u = importlib.util.module_from_spec(spec)
spec.loader.exec_module(u)
sys.modules['delivery_recovery'] = u.recovery

# Native Responses checks model metadata before image dispatch; implement its
# real GET API rather than relying on the old fixture's 501 HTML response.
def models_get(handler):
    body=json.dumps({'data':[{'id':'gpt-4o','input_modalities':['text','image']}]}).encode()
    handler.send_response(200); handler.send_header('Content-Type','application/json')
    handler.send_header('Content-Length',str(len(body))); handler.end_headers(); handler.wfile.write(body)
u.Provider.do_GET=models_get

F9 = b'\x1b[20~'
DOWN = b'\x1b[B'
ESC = b'\x1b'
PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg=='


def contains(ui, text):
    try:
        ui.until(lambda s: text in s.text(),text)
    except Exception:
        raise AssertionError(f'Expected {text!r}: {ui.pump().text()}') from None


def capture(j, ui, name):
    u.settle(ui)
    (j.output/(name+'.txt')).write_text(ui.pump().text())


def select(j, ui, sid):
    # F2 searches displayed titles/routes, not UUIDs. Use visible order, then
    # verify canonical identity via exact unique content in each caller.
    ui.send(b'\x1bOQ')
    contains(ui,'Find voyages')
    if hasattr(j,'picker_order'):
        contains(ui,'same-title' if not hasattr(j,'route_case') else 'identical-title')
        ui.send(b'\x1b[H'+DOWN*j.picker_order.index(sid))
    else:
        u.paste(ui,sid[:8])
    u.settle(ui)
    ui.send(b'\r')
    contains(ui,'Model:')
    u.settle(ui)
    # Escape leaves sidebar focus without changing voyage.
    ui.send(ESC); u.settle(ui)


def menu(j, ui, index, expected):
    ui.send(F9)
    contains(ui, 'Voyage actions')
    ui.send(DOWN*index+b'\r')
    contains(ui, expected)


def clipboard(j):
    """Real helper subprocess, deterministic local clipboard seam, no desktop data."""
    root = j.f.root/'clip'; root.mkdir()
    mode = root/'mode'; mode.write_text('image')
    data = root/'image.png'; data.write_bytes(base64.b64decode(PNG))
    helper = root/'wl-paste'
    helper.write_text('#!/usr/bin/env python3\nimport pathlib,sys,time,os\nr=pathlib.Path(__file__).parent\nm=(r/"mode").read_text()\nif "--list-types" in sys.argv: print("image/png"); sys.exit(0)\n(r/"started").write_text(str(os.getpid()))\nif m=="hold": time.sleep(20)\nif m=="fail": sys.exit(1)\nsys.stdout.buffer.write((r/"image.png").read_bytes())\n')
    helper.chmod(0o700)
    j.f.env.update(PATH=str(root)+os.pathsep+j.f.env['PATH'], WAYLAND_DISPLAY='synthetic-only')
    j.f.env.pop('DISPLAY',None)
    return mode


def command_receipts(f):
    return [json.loads(p.read_text()) for p in f.root.rglob('helm-command-receipts/*.json')]


def images(j):
    mode = clipboard(j)
    sid = j.f.session()
    j.f.command(sid,j.f.mutation(sid,'set_model',model='gpt-4o'))
    ui = j.connect(); select(j,ui,sid)
    u.paste(ui,'four-image-draft')
    for n in range(1,5):
        ui.send(b'\x16')
        contains(ui, f'[Image {n}]')
    capture(j,ui,'four-images-acquired')
    j.note('X02: four images acquired by explicit Ctrl+V via bounded local helper and retained in memory')
    ui.send(b'\x16'); contains(ui,'At most four images')
    contains(ui, '[Image 4]')
    mode.write_text('fail'); ui.send(b'\x16')
    contains(ui,'Clipboard has no image or text'); capture(j,ui,'clipboard-failure')
    contains(ui, '[Image 4]'); contains(ui, 'four-image-draft')
    mode.write_text('hold'); (mode.parent/'started').unlink(missing_ok=True)
    ui.send(b'\x16')
    u.recovery.wait_for(lambda:(mode.parent/'started').exists())
    helper_pid=int((mode.parent/'started').read_text())
    ui.send(ESC); time.sleep(.2); u.settle(ui)
    u.recovery.wait_for(lambda:not Path(f'/proc/{helper_pid}').exists())
    contains(ui, '[Image 4]'); contains(ui, 'four-image-draft')
    capture(j,ui,'clipboard-cancel')
    j.note('X02: fifth-image rejection, helper failure and Escape acquisition cancellation retain authored text/four images; held helper reaped')
    mode.write_text('image')
    ui.send(b'\r'); image_snapshot=j.finish(sid)
    (j.output/'image-final.json').write_text(json.dumps(image_snapshot,indent=2))
    assert image_snapshot['run']['state']=='completed', image_snapshot.get('run')
    body=j.f.provider.bodies[-1]
    assert json.dumps(body).count('data:image/png;base64,')==4, body
    assert len(j.f.provider.bodies)==1
    (j.output/'image-provider.json').write_text(json.dumps(body,indent=2))
    snap=j.f.snapshot(sid); user=[m for m in snap['messages'] if m['role']=='user']
    assert len(user)==1 and json.dumps(user[0]).count('image/png')==4, user
    j.note('X02: all four acquired images and text submitted once to synthetic provider and canonical user content')


def steering(j):
    clipboard(j)
    sid=j.f.session(); ui=j.connect(); select(j,ui,sid)
    u.paste(ui,'UXHOLD busy run'); ui.send(b'\r')
    u.recovery.wait_for(lambda:bool(j.f.provider.bodies))
    contains(ui,'Synthetic streaming started')
    u.paste(ui,'retained-image-steering'); ui.send(b'\x16')
    contains(ui, '[Image 1]')
    ui.send(b'\r'); contains(ui,'draft preserved')
    contains(ui, 'retained-image-steering'); contains(ui, '[Image 1]')
    assert len(j.f.provider.bodies)==1
    capture(j,ui,'busy-image-draft-retained')
    j.note('X03: image-bearing steering refused while busy without dropping text/image or admitting another provider call')
    ui.send(b'\x01'); u.paste(ui,'unique-text-steer'); ui.send(b'\r')
    def admitted_draft():
        return next((d for d in command_receipts(j.f) if d.get('pending') and 'unique-text-steer' in json.dumps(d['pending'])),None)
    # Admission receipt exists independently of canonical messages until applied.
    contains(ui,'Steering admitted')
    capture(j,ui,'steering-admitted')
    pending=admitted_draft()
    if pending:
        command_id=pending['pending']['command_id']
        admitted=j.f.command(sid,{'op':'receipt','command_id':command_id})
        (j.output/'steering-admitted.json').write_text(json.dumps(admitted,indent=2))
    j.f.provider.release.set(); final_snapshot=j.finish(sid)
    final=next(m for m in final_snapshot['messages'] if m.get('steering') and 'unique-text-steer' in json.dumps(m))
    (j.output/'steering-delivered.json').write_text(json.dumps(final,indent=2))
    assert final['steering']['status']=='applied', final
    assert sum('unique-text-steer' in json.dumps(m) for m in final_snapshot['messages'] if m['role']=='user')==1
    capture(j,ui,'steering-delivered')
    j.note('X03: text steering admitted once during held run and canonical receipt transitions to delivered after released safe boundary')


def lifecycle(j):
    sid=j.f.session(); peer=j.f.session()
    for s in (sid,peer):
        j.f.command(s,j.f.mutation(s,'rename',name='same-title'))
        j.f.command(s,j.f.mutation(s,'submit',prompt='lifecycle-evidence-'+s[:8])); j.finish(s)
    before=j.f.snapshot(peer)['messages']
    j.picker_order=[peer,sid]
    ui=j.connect(); select(j,ui,sid)
    contains(ui,'lifecycle-evidence-'+sid[:8])
    u.paste(ui,'unsent-lifecycle-draft'); u.settle(ui)
    contains(ui, 'unsent-lifecycle-draft')
    assert all('unsent-lifecycle-draft' not in json.dumps(d) for d in command_receipts(j.f))
    (j.output/'receipts-before-archive.json').write_text(json.dumps(command_receipts(j.f),indent=2))
    menu(j,ui,1,'Archived')
    u.recovery.wait_for(lambda:j.f.snapshot(sid)['lifecycle']['archived'])
    u.recovery.wait_for(lambda:bool(j.f.request({'op':'inspect','session_id':sid}).get('archive')))
    time.sleep(1); u.settle(ui)
    assert not j.f.snapshot(peer)['lifecycle']['archived']
    capture(j,ui,'archived-exact-target')
    ui.send(b'\x1b[15~'); time.sleep(1); u.settle(ui)
    contains(ui,'same-title')
    j.picker_order=[sid]; select(j,ui,sid); menu(j,ui,1,'Update confirmed')
    j.picker_order=[peer,sid]
    u.recovery.wait_for(lambda:not j.f.snapshot(sid)['lifecycle']['archived'])
    capture(j,ui,'restored-exact-target')
    (j.output/'receipts-after-restore.json').write_text(json.dumps(command_receipts(j.f),indent=2))
    contains(ui,'unsent-lifecycle-draft')
    contains(ui,'Enter Submit new turn'); time.sleep(.5); u.settle(ui)
    j.note('X10: F9 archives/restores only exact selected same-title voyage')
    for index,word in [(6,'CLEAR'),(7,'DELETE')]:
        original=j.f.snapshot(sid)['messages']
        menu(j,ui,index,'Type '+word)
        ui.send(ESC); u.settle(ui); ui.send(ESC); u.settle(ui)
        assert j.f.snapshot(sid)['messages']==original
        contains(ui,'unsent-lifecycle-draft')
        menu(j,ui,index,'Type '+word)
        j.f.command(sid,j.f.mutation(sid,'rename',name='same-title-revised-'+word))
        time.sleep(.6); u.settle(ui)
        u.paste(ui,word); ui.send(b'\r')
        ui.until(lambda screen:any(t in screen.text() for t in ('Voyage changed','Voyage restarted; reopen Actions')),'stale review refusal'); capture(j,ui,word.lower()+'-stale-refused')
        assert j.f.snapshot(sid)['messages']==original
        ui.send(ESC); u.settle(ui); ui.send(ESC); u.settle(ui)
        contains(ui,'Enter Submit new turn'); time.sleep(1); u.settle(ui)
        menu(j,ui,index,'Type '+word)
        u.paste(ui,word); ui.send(b'\r')
        if word=='CLEAR':
            u.recovery.wait_for(lambda:not j.f.snapshot(sid)['messages'])
            contains(ui,'unsent-lifecycle-draft'); capture(j,ui,'clear-confirmed')
            j.f.command(sid,j.f.mutation(sid,'submit',prompt='new-history-before-delete')); j.finish(sid)
            time.sleep(.5); u.settle(ui)
        else:
            def deleted():
                snapshot=j.f.snapshot(sid)
                return snapshot['lifecycle']['deleted']
            try:
                u.recovery.wait_for(deleted)
            except Exception:
                capture(j,ui,'delete-failed')
                (j.output/'delete-failed-snapshot.json').write_text(json.dumps(j.f.snapshot(sid),indent=2))
                raise
            capture(j,ui,'delete-confirmed')
        assert j.f.snapshot(peer)['messages']==before
        j.note(f'X10: {word} cancel leaves history/draft; stale frozen review refuses; reopened confirmation affects exact selected history only')


def routes(j):
    gateway_module=u.load('ux273_gateway',ROOT/'voyage/tests/approval_semantics.py')
    second=u.Fixture(j.binaries); gateway_a=gateway_b=None
    try:
        second.start(); second.seed_account()
        a=j.f.session(); b=second.session()
        for f,s,text in [(j.f,a,'route-alpha-canonical'),(second,b,'route-beta-canonical')]:
            f.command(s,f.mutation(s,'rename',name='identical-title'))
            f.command(s,f.mutation(s,'submit',prompt=text))
            u.recovery.wait_for(lambda:f.snapshot(s).get('pending_cleanup_run') is None and f.snapshot(s).get('run',{}).get('state')=='completed')
        gateway_a=gateway_module.Gateway(j.f); gateway_b=gateway_module.Gateway(second)
        paths=[]
        for i,(g,s) in enumerate([(gateway_a,a),(gateway_b,b)]):
            p=j.f.root/f'route-{i}.json'; p.write_text(json.dumps(g.grant(s,['observe','history','execute','steer'],300000))); p.chmod(0o600); paths.append(p)
        ui=u.private.OuterPTY([str(j.binaries/'helm'),'connect','--access-file',str(paths[0]),'--access-file',str(paths[1])],
            {**j.f.env,'TERM':'xterm-256color'},j.f.workspace,j.output/'two-routes.pty')
        j.clients.append(ui); ui.resize(140,45)
        contains(ui,'Model:'); u.settle(ui)
        j.picker_order=[b,a]; j.route_case=True
        for s,text in [(a,'route-alpha-canonical'),(b,'route-beta-canonical'),(a,'route-alpha-canonical')]:
            select(j,ui,s); contains(ui,text); capture(j,ui,'selected-'+s[:8])
            u.paste(ui,'draft-'+s[:8]); u.settle(ui)
        assert 'route-alpha-canonical' in json.dumps(j.f.snapshot(a)['messages'])
        assert 'route-beta-canonical' in json.dumps(second.snapshot(b)['messages'])
        j.note('X09: real two loopback Vessel gateways with same-title voyages; F2 exact session selection switches canonical source and returns to original route')
    finally:
        # Close clients before the gateways they poll.
        for ui in j.clients: ui.close()
        j.clients=[]
        for g in (gateway_a,gateway_b):
            if g: g.close()
        second.close()
        j.note('Second Vessel/provider/gateway shutdown observed')


if __name__=='__main__':
    u.CASES={'images':images,'steering':steering,'routes':routes,'lifecycle':lifecycle}
    import sys
    try:
        u.main()
    finally:
        if '--output' in sys.argv:
            out=Path(sys.argv[sys.argv.index('--output')+1])/'result.json'
            if out.exists():
                result=json.loads(out.read_text())
                result['harness_sha256'][str(Path(__file__).relative_to(ROOT))]=u.sha(Path(__file__))
                result['dimensions']=[[120,40],[140,45]]
                result['configuration']={'provider':'synthetic openai-responses loopback SSE', 'model':'fixture-model; gpt-4o for image metadata', 'access':'read-only', 'terminal':'xterm-256color', 'features':'parent manifest; no live auth/browser'}
                result['limitations']=['Linux synthetic PTY only; deterministic clipboard helper (not a real desktop clipboard/capture)',
                    'No live providers, credentials, native platform, public TLS or screenshot-selection claim',
                    'Cases report only observed assertions; X02/X03/X09/X10 are not broad matrix certification']
                result['binary_sha256_after']={name:u.sha(Path(sys.argv[sys.argv.index('--bin-dir')+1])/name) for name in ('helm','vessel','voyage')}
                out.write_text(json.dumps(result,indent=2)+'\n')
