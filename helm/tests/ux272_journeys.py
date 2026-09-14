#!/usr/bin/env python3
"""Offline #272 integrated-binary journeys. No build or live provider authorization.

Each case uses a disposable real Vessel/Voyage and synthetic Helm PTY. A failed
case remains failed; unimplemented matrix portions are explicitly not verified.
"""
import argparse
import base64
import subprocess
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import platform
import socket
import struct
import threading
import traceback
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


recovery = load('ux272_delivery_recovery', ROOT / 'voyage/tests/delivery_recovery.py')
private = load('ux272_private_terminal', ROOT / 'helm/tests/private_terminal.py')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        try:
            self.connection.settimeout(15)
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            self.server.bodies.append(body)
            inputs = body.get('input', [])
            text = json.dumps(inputs, ensure_ascii=False)
            outputs = [i for i in inputs if i.get('type') == 'function_call_output']
            tool = None
            if not outputs:
                if 'UXQUESTION' in text:
                    tool = ('questions', {'question': 'Synthetic choice — no secrets',
                                           'options': ['Alpha', 'Beta']})
                elif 'UXAPPROVAL' in text:
                    tool = ('write_file', {'path': 'approval-output.txt', 'content': 'synthetic-approved'})
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            if 'UXHOLD' in text:
                self.event({'type': 'response.output_text.delta', 'delta': 'Synthetic streaming started\n'})
                assert self.server.release.wait(60), 'provider hold exceeded bound'
            output = ([{'type': 'function_call', 'call_id': 'synthetic-tool', 'name': tool[0],
                        'arguments': json.dumps(tool[1])}] if tool else
                      [{'type': 'message', 'role': 'assistant', 'content': [
                          {'type': 'output_text', 'text': 'Fixture finished. café 中文\n' +
                           '\n'.join(f'anchor-line-{i:03}' for i in range(80))}]}])
            self.event({'type': 'response.completed', 'response': {
                'id': 'synthetic-response', 'status': 'completed', 'output': output,
                'usage': {'input_tokens': 1, 'output_tokens': 1}}})
        except (BrokenPipeError, ConnectionResetError):
            # Expected only when the held run is explicitly cancelled/detached.
            self.server.disconnects += 1
        except Exception as error:
            self.server.errors.append(repr(error))

    def event(self, body):
        self.wfile.write(('data: ' + json.dumps(body) + '\n\n').encode())
        self.wfile.flush()


class Fixture(recovery.Fixture):
    def __init__(self, binaries):
        # Existing fixture constructs its provider in __init__, not start.
        recovery.Provider = Provider
        super().__init__(binaries)
        self.env['PROVIDER_FIXTURE_KEY'] = 'synthetic-offline-key'
        self.provider.release = threading.Event()
        self.provider.errors = []
        self.provider.disconnects = 0

    def seed_account(self):
        self.env['PROVIDER_FIXTURE_KEY'] = 'synthetic-offline-key'
        def cli(*args):
            run = subprocess.run([str(self.binaries/'vessel'),'auth','accounts',*args],
                env=self.env,cwd=self.workspace,capture_output=True,text=True,timeout=15)
            assert run.returncode == 0, run.stderr
            return json.loads(run.stdout)
        c = cli('connect','--label','Synthetic loopback','--endpoint',
            f'http://127.0.0.1:{self.provider.server_port}/v1','--transports','openai-responses')
        a = cli('add','--connection',c['id'],'--account','synthetic','--env','PROVIDER_FIXTURE_KEY')
        self.binding = {'account_id':a['id'],'connection_id':c['id'],
            'identity_generation':a['identity_generation'],'connection_revision':c['revision'],
            'transport':'openai_responses'}
        self.request({'op':'account_set_default','command_id':str(uuid.uuid4()),
            'workspace':str(self.workspace),'account':self.binding,'expected_revision':0})

    def command(self, session, command, envelope=False):
        return super().command(session, command, allow_error=envelope)

    def session(self, approval=False):
        sid = str(uuid.uuid4())
        config = self.root/(sid+'.json')
        access = 'approval' if approval else 'unrestricted' if getattr(self,'unrestricted',False) else 'read-only'
        config.write_text(json.dumps({'version':1,'workspace':str(self.workspace),
            'config':{'provider':'openai-responses','model':'fixture-model',
                'base_url':f'http://127.0.0.1:{self.provider.server_port}/v1',
                'account':self.binding,'access':access,'max_tokens':1024,
                'provider_retry_attempts':1,'context_window':0,
                'command_timeout_secs':getattr(self,'decision_timeout',12)},
            'explicit':{'access':access},'selection':None,'confirmation':None}))
        config.chmod(0o600)
        self.request({'op':'start_settings','session_id':sid,'command_id':str(uuid.uuid4()),
            'workspace':str(self.workspace),'config_path':str(config),'binding':self.binding,'settings':{}})
        self.sessions.append(sid)
        return sid

    def snapshot(self, session):
        return self.command(session, {'op': 'snapshot'})

    def mutation(self, session, op, **fields):
        return {'op': op, 'command_id': str(uuid.uuid4()),
                'expected_revision': self.snapshot(session)['revision'],
                'expires_at_ms': int(time.time()*1000)+60000, **fields}

    def close(self):
        if hasattr(self, 'provider') and hasattr(self.provider, 'release'):
            self.provider.release.set()
        super().close()
        assert not self.provider.errors, self.provider.errors
        assert self.supervisor is None or self.supervisor.poll() is not None
        assert not self.owned_processes(), 'fixture child processes remain'


def paste(ui, text):
    ui.send(b'\x1b[200~' + text.encode() + b'\x1b[201~')


def command_text(ui, text):
    # Slash actions preserve authored drafts; replace explicitly, never append.
    ui.send(b'\x01')  # actual composer Ctrl+A selection
    paste(ui, text)
    ui.send(b'\r')


def settle(ui):
    deadline = time.monotonic() + .4
    while time.monotonic() < deadline:
        ui.pump()
        time.sleep(.02)


class Journey:
    def __init__(self, f, binaries, output):
        self.f, self.binaries, self.output = f, binaries, output
        self.clients = []
        self.observations = []

    def connect(self):
        ui = private.OuterPTY([str(self.binaries/'helm'), 'connect', '--directory',
                              str(self.f.directory), '--no-start'],
                             {**self.f.env, 'TERM': 'xterm-256color'}, self.f.workspace,
                             self.output / f'ui-{len(self.clients)}.pty')
        self.clients.append(ui)
        ui.resize(120, 40)
        ui.until(lambda s: 'Helm' in s.text() or 'Your voyages' in s.text(), 'connected Helm')
        if self.f.sessions:
            ui.until(lambda s: 'Model:' in s.text(), 'selected voyage composer')
            settle(ui)
        return ui

    def note(self, text):
        self.observations.append(text)

    def finish(self, session):
        snapshot = self.f.finished(session)
        assert snapshot.get('pending_cleanup_run') is None, snapshot.get('pending_cleanup_run')
        return snapshot

    def close(self):
        errors = []
        for ui in self.clients:
            try:
                if ui.process.poll() is None:
                    ui.send(b'\x03')
                    ui.exited_restored()
                ui.close()
            except Exception as error:
                errors.append(repr(error))
                try: ui.close()
                except Exception: pass
        try: self.f.close()
        except Exception as error: errors.append(repr(error))
        return errors


def composer(j):
    s = j.f.session()
    ui = j.connect()
    prompt = 'UXUNICODE café e\u0301 中文 👩‍💻\nsecond line'
    paste(ui, prompt)
    settle(ui)
    assert not j.f.snapshot(s)['messages'], 'paste implicitly submitted'
    ui.send(b'\x01\x18'); settle(ui)  # select all / cut to internal yank
    ui.send(b'\x1by'); settle(ui)  # internal yank, no system clipboard
    ui.send(b'\x1a'); settle(ui)  # undo yank
    ui.send(b'\x19'); settle(ui)  # redo yank
    ui.send(b'\r')
    snap = j.finish(s)
    assert any(m.get('role') == 'user' and m.get('content') == prompt for m in snap['messages']), snap['messages']
    assert len(j.f.provider.bodies) == 1
    j.note('X01/X02: bracketed multiline Unicode paste + select/cut/yank/undo/redo authored and submitted exactly once through Helm')
    ui.until(lambda screen: 'anchor-line-' in screen.text(), 'long canonical answer')
    ui.send(b'\x1b[5~'); settle(ui)
    before = ui.pump().text()
    ui.resize(80, 24); settle(ui)
    ui.resize(120, 40); settle(ui)
    assert 'anchor-line-' in ui.pump().text()
    (j.output/'anchor-before.txt').write_text(before)
    (j.output/'anchor-after.txt').write_text(ui.pump().text())
    ui.send(b'\x1b[F'); settle(ui)
    assert len(j.f.snapshot(s)['messages']) == len(snap['messages'])
    j.note('X08/X16 partial: paged canonical answer survives resize without history duplication; exact visual anchor requires retained frame review')


def readiness(j):
    ui = j.connect()
    ui.send(b'\x0e')
    ui.until(lambda s: any(w in s.text().lower() for w in ('account', 'first message', 'sign in')), 'readiness')
    assert not j.f.request({'op': 'catalogue'})
    ui.resize(30, 10)
    ui.until(lambda s: 'small' in s.text().lower() or '40' in s.text(), 'small-size guard')
    ui.send(b'\x1b'); settle(ui)
    ui.resize(120, 40)
    ui.until(lambda s: any(w in s.text().lower() for w in ('account', 'first message', 'sign in')), 'readiness preserved')
    assert not j.f.request({'op': 'catalogue'})
    j.note('X01/X16 partial: empty readiness and tiny-layout Esc do not create a voyage')


def decision(j, kind, action):
    j.f.decision_timeout = 5 if action == 'expiry' else 12
    s = j.f.session(approval=kind == 'approval')
    ui = j.connect()
    prompt = 'UXQUESTION' if kind == 'question' else 'UXAPPROVAL'
    # Runtime submits the trigger; the actual pending decision is rendered/responded by Helm.
    j.f.command(s, j.f.submit(s, prompt))
    d = recovery.wait_for(lambda: j.f.command(s, {'op': 'decisions'}))[0]
    ui.until(lambda screen: ('Synthetic choice' if kind == 'question' else 'approval-output') in screen.text(), 'real decision modal')
    if action == 'custom':
        ui.send(b'\x1b[B\x1b[B\r'); settle(ui)
        paste(ui, 'custom café\n中文')
        ui.send(b'\r')
    elif action == 'choice':
        ui.send(b'\x1b[B\r')
    elif action == 'back':
        ui.send(b'\x1b[B\x1b[B\r'); settle(ui)
        paste(ui, 'discarded custom')
        ui.send(b'\x1b'); settle(ui)
        assert j.f.command(s, {'op': 'decisions'}), 'back unexpectedly answered'
        ui.send(b'\x1b')
    elif action == 'approve':
        ui.send(b'\x1b[B\r')
    elif action == 'escape':
        ui.send(b'\x1b')
    elif action == 'expiry':
        # Fixture command_timeout_secs=5: real decision deadline, not an expired submit.
        pass
    else:
        raise AssertionError(action)
    snap = j.finish(s)
    text = json.dumps(snap, ensure_ascii=False)
    if kind == 'approval':
        path = j.f.workspace/'approval-output.txt'
        if action == 'approve':
            assert path.read_text() == 'synthetic-approved'
        else:
            assert not path.exists()
            expected = 'approval_expired' if action == 'expiry' else 'approval_denied'
            outcomes = [m.get('tool_outcome', {}).get('execution') for m in snap['messages'] if m.get('tool_outcome')]
            assert expected in outcomes, outcomes
    elif action == 'custom':
        assert 'custom café中文' in text, text
    elif action == 'choice':
        assert 'Beta' in text and 'selected' in text, text
    elif action in ('escape', 'back'):
        assert 'cancelled' in text, text
        assert 'discarded custom' not in text
    else:
        assert 'custom café' not in text
        assert any(word in text.lower() for word in ('timed out', 'expired', 'timeout')), text
    stale = j.f.mutation(s, 'respond', run_id=d['run_id'], decision_id=d['decision_id'],
                         response='approved' if kind == 'approval' else {'status':'selected', 'index':0, 'answer':'Alpha'})
    response = j.f.command(s, stale, envelope=True)
    assert response.get('error') or response.get('result', {}).get('status') != 'accepted', response
    assert j.f.snapshot(s)['messages'] == snap['messages']
    assert [m['content'] for m in snap['messages'] if m.get('role') == 'user'] == [prompt]
    j.note(f'X06/X07 {kind}/{action}: real runtime decision + Helm focus, stale response refused; no answer leaked into composer submission')


def recover(j):
    s = j.f.session()
    original = j.f.submit(s, 'UXRECOVER')
    registration = json.loads((j.f.directory/'sessions'/s/'registration.json').read_text())
    message = {'protocol':1, 'session_id':s, 'incarnation':registration['incarnation'],
               'token':registration['token'], 'command':original}
    with socket.socket(socket.AF_UNIX) as conn:
        conn.connect(str(j.f.directory/'sessions'/s/'runtime.sock'))
        encoded = json.dumps(message).encode()
        conn.sendall(struct.pack('>I', len(encoded))+encoded)
        snap = j.finish(s)  # Never read the submit response.
    receipt = j.f.command(s, {'op':'resolve', 'command_id':original['command_id'], 'original':original})
    assert receipt['status'] == 'accepted', receipt
    assert j.f.snapshot(s)['messages'] == snap['messages']
    assert len(j.f.provider.bodies) == 1
    (j.output/'receipt.json').write_text(json.dumps({'original': original, 'receipt':receipt}, indent=2))
    j.note('X12: genuinely unread submit response resolved using ORIGINAL command ID and payload; no submit replay')


def exact_replay(j):
    s = j.f.session()
    original = j.f.submit(s, 'UXDEDUP')
    accepted = j.f.command(s, original)
    snap = j.finish(s)
    duplicate = j.f.command(s, original)
    assert duplicate['duplicate'] is True
    assert duplicate['run_id'] == accepted['run_id']
    assert len(j.f.provider.bodies) == 1
    assert j.f.snapshot(s)['messages'] == snap['messages']
    j.note('X12 separate adversarial EXACT REPLAY/dedup probe; NOT ordinary lost-response recovery')


def stop(j):
    s = j.f.session()
    ui = j.connect()
    j.f.command(s, j.f.submit(s, 'UXHOLD'))
    recovery.wait_for(lambda: j.f.provider.bodies)
    paste(ui, 'unsent retained')
    ui.send(b'\x03'); ui.exited_restored()
    assert j.f.snapshot(s)['run']['state'] == 'running', 'detach cancelled voyage'
    ui = j.connect()
    command_text(ui, '/stop')
    ui.until(lambda screen: 'Request Stop' in screen.text(), 'Stop exact-run review')
    assert s in ui.pump().text(), 'Stop review does not display exact voyage'
    ui.send(b'\x1b'); settle(ui)
    assert j.f.snapshot(s)['run']['state'] == 'running', 'Esc back cancelled run'
    command_text(ui, '/stop')
    ui.until(lambda screen: 'Request Stop' in screen.text(), 'Stop review reopened')
    ui.send(b'\r')
    snap = j.finish(s)
    assert snap['run']['state'] == 'cancelled', snap['run']
    assert not any(m.get('content') == '/stop' for m in snap['messages'])
    j.f.provider.release.set()
    j.note('X03/X04 partial: real held stream, Ctrl+C detaches; reconnect /stop Esc back preserves run, then visible confirmation stops exact run and observed cleanup')


def sessions(j):
    first = j.f.session()
    second = j.f.session()
    for session, title in ((first, 'UX session Alpha'), (second, 'UX session Beta')):
        response = j.f.command(session, j.f.mutation(session, 'rename', name=title))
        assert response['status'] == 'applied', response
    ui = j.connect()
    def select(title):
        ui.until(lambda screen: screen.position(title) is not None, 'sidebar ' + title)
        x, y = ui.pump().position(title)
        # Synthetic SGR pointer event at observed current geometry, not guessed row.
        ui.send(f'\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m'.encode())
        settle(ui)
    select('UX session Alpha')
    paste(ui, 'alpha draft café')
    select('UX session Beta')
    paste(ui, 'beta draft 中文')
    select('UX session Alpha')
    ui.send(b'\x1b'); settle(ui)  # sidebar focus -> composer, not submission
    ui.send(b'\r')
    one = j.finish(first)
    assert any(m.get('content') == 'alpha draft café' for m in one['messages'])
    assert not j.f.snapshot(second)['messages']
    select('UX session Beta')
    ui.send(b'\x1b'); settle(ui)
    ui.send(b'\r')
    two = j.finish(second)
    assert any(m.get('content') == 'beta draft 中文' for m in two['messages'])
    assert len(j.f.provider.bodies) == 2
    j.note('X09 partial: two exact local identities, runtime rename and observed sidebar switch preserve each unsent Unicode draft; no cross-target send')


def historical(j):
    source = j.f.session()
    for prompt in ('UXHISTORY first', 'UXHISTORY second'):
        j.f.command(source, j.f.submit(source, prompt))
        j.finish(source)
    original = j.f.snapshot(source)['messages']
    files = {p.name:sha(p) for p in j.f.workspace.iterdir() if p.is_file()}
    ui = j.connect()
    ui.until(lambda screen: 'anchor-line-' in screen.text(), 'saved history loaded')
    command_text(ui, '/branch')
    ui.until(lambda screen: 'New voyage:' in screen.text(), 'ordinary historical review identity')
    # Full-history is a distinct option; cycle only observed review choices until
    # the FIRST saved user-message boundary is explicitly visible.
    for _ in range(4):
        if 'Through saved user message #1' in ui.pump().text() and 'UXHISTORY first' in ui.pump().text():
            break
        ui.send(b'\x1b[1;5B'); settle(ui)
    text = ui.pump().text()
    assert 'UXHISTORY first' in text and 'Through saved user message #1' in text, text
    # The panel may visually truncate the UUID; do not reconstruct/guess it.
    existing = {p.name for p in (j.f.directory/'sessions').iterdir()}
    branch = None
    # Retain the expected identity before the create effect; register it with
    # fixture teardown once its real registration is observed.
    ui.send(b'\r')
    def registered():
        nonlocal branch
        found = [p.name for p in (j.f.directory/'sessions').iterdir()
                 if p.name not in existing and (p/'registration.json').exists()]
        assert len(found) <= 1, 'unexpected multiple branch admissions'
        if found:
            branch = found[0]
            if branch not in j.f.sessions: j.f.sessions.append(branch)
            return True
        return False
    recovery.wait_for(registered)
    assert branch != source
    child = j.f.snapshot(branch)
    users = [m['content'] for m in child['messages'] if m.get('role') == 'user']
    assert users == ['UXHISTORY first'], users
    assert j.f.snapshot(source)['messages'] == original
    assert len(j.f.provider.bodies) == 2, 'branch unexpectedly inferred'
    assert files == {p.name:sha(p) for p in j.f.workspace.iterdir() if p.is_file()}
    # A readable branch snapshot can precede the initialized runtime's settled
    # lifecycle. Wait for its actual clean idle suspension before fixture teardown;
    # do not signal during bootstrap and then expect a normal stopped checkpoint.
    def branch_suspended():
        path = j.f.directory/'sessions'/branch/'stopped.json'
        if not path.exists(): return False
        stopped = json.loads(path.read_text())
        registration = json.loads((path.parent/'registration.json').read_text())
        return stopped.get('cleanup_observed') is True and stopped.get('incarnation') == registration['incarnation']
    recovery.wait_for(branch_suspended)
    j.note('X09/X10 partial: rendered selected historical user boundary creates distinct identity/context, unchanged source/files, no inference')


def inspection(j):
    j.f.unrestricted = True
    w = j.f.workspace
    def git(*args):
        subprocess.run(['git', *args], cwd=w, check=True, capture_output=True)
    # This is a NEW disposable synthetic project, never checkout metadata.
    git('init', '-q')
    (w/'tracked.txt').write_text('baseline\n')
    (w/'binary.dat').write_bytes(b'\x00baseline')
    git('add', '.')
    git('-c', 'user.name=Synthetic', '-c', 'user.email=synthetic@example.invalid',
        'commit', '-qm', 'synthetic baseline')
    (w/'tracked.txt').write_text('staged-marker\n')
    git('add', 'tracked.txt')
    (w/'tracked.txt').write_text('unstaged-marker\n')
    (w/'untracked.txt').write_text('untracked-marker\n')
    (w/'binary.dat').write_bytes(b'\x00changed')
    before = {p.name:sha(p) for p in w.iterdir() if p.is_file()}
    s = j.f.session()
    ui = j.connect()
    j.f.command(s, j.f.submit(s, 'UXCOPY'))
    j.finish(s)
    ui.until(lambda screen: 'anchor-line-' in screen.text(), 'completed response')
    command_text(ui, '/copy')
    ui.until(lambda screen: 'clipboard' in screen.text().lower(), 'copy disclosure')
    ui.pump()
    assert b'\x1b]52;' not in Path(ui.log.name).read_bytes()
    ui.send(b'\x1b'); settle(ui)
    assert b'\x1b]52;' not in Path(ui.log.name).read_bytes()
    command_text(ui, '/copy')
    ui.until(lambda screen: 'clipboard' in screen.text().lower(), 'copy disclosure again')
    ui.send(b'\r')
    def copied():
        ui.pump()
        return b'\x1b]52;' in Path(ui.log.name).read_bytes()
    recovery.wait_for(copied)
    capture = Path(ui.log.name).read_bytes()
    payload = capture.split(b'\x1b]52;', 1)[1].split(b';', 1)[1]
    payload = payload.split(b'\x07', 1)[0].split(b'\x1b\\', 1)[0]
    canonical = base64.b64decode(payload).decode()
    assert canonical.startswith('Fixture finished. café 中文\n')
    assert 'anchor-line-079' in canonical
    command_text(ui, '/diff')
    ui.until(lambda screen: 'Inspect coding results' in screen.text(), 'inspection panel')
    settle(ui)
    ui.send(b'\x1b[B\r')  # Panel defaults to Status; explicitly select Unstaged.
    ui.until(lambda screen: 'unstaged-marker' in screen.text(), 'actual unstaged runtime diff', timeout=40)
    assert before == {p.name:sha(p) for p in w.iterdir() if p.is_file()}
    assert len(j.f.provider.bodies) == 1, 'operator inspection unexpectedly called provider'
    j.note('X11 partial: explicit copy cancel/confirm, OSC52 canonical bytes, runtime unstaged diff over dirty synthetic repository; no files changed')


CASES = {'readiness':readiness, 'composer':composer, 'recovery':recover, 'exact-replay':exact_replay, 'stop':stop, 'inspection':inspection, 'sessions':sessions, 'historical':historical}
for _kind in ('approval', 'question'):
    for _action in (('approve', 'escape', 'expiry') if _kind == 'approval' else ('choice', 'custom', 'back', 'escape', 'expiry')):
        CASES[f'{_kind}-{_action}'] = lambda j, k=_kind, a=_action: decision(j, k, a)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bin-dir', type=Path, required=True)
    p.add_argument('--build-manifest', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--case', action='append', choices=CASES)
    a = p.parse_args()
    binaries = a.bin_dir.resolve()
    manifest = json.loads(a.build_manifest.read_text())
    assert manifest.get('source_revision') and manifest.get('source_fingerprint'), 'source attestation required'
    hashes = {n:sha(binaries/n) for n in ('helm','vessel','voyage')}
    assert hashes == manifest['binary_sha256'], 'binary manifest mismatch'
    a.output.mkdir(parents=True, exist_ok=False)
    result = {'build':manifest, 'platform':platform.platform(), 'dimensions':[[120,40],[80,24],[30,10]],
              'configuration':{'provider':'synthetic openai-responses loopback SSE',
                'model':'fixture-model', 'access':'read-only; approval decision cases; unrestricted inspection only',
                'command_timeout_secs':{'normal':12,'decision_expiry':5}, 'provider_retry_attempts':1,
                'context_window':0, 'terminal':'xterm-256color', 'features':'default build; no live auth/browser'},
              'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'cases':{},
              'harness_sha256':{str(p.relative_to(ROOT)):sha(p) for p in [Path(__file__),
                ROOT/'voyage/tests/delivery_recovery.py', ROOT/'helm/tests/private_terminal.py']},
              'limitations':['Synthetic Linux PTY only; screen parser is not a full grapheme-width emulator',
                'X10 destructive actions, X13-X15 boundaries not verified; other rows partial; see ledger',
                'No native platform, image, live auth, browser, real clipboard or competitor journey claim']}
    for name in a.case or CASES:
        out = a.output/name; out.mkdir()
        f = Fixture(binaries)
        j = Journey(f,binaries,out)
        # Constructing the existing fixture already owns a provider thread.
        row = {'status':'running', 'private_fixture_root':str(f.root)}
        result['cases'][name] = row
        try:
            f.start()
            if name != 'readiness': f.seed_account()
            CASES[name](j)
            row['status'] = 'passed-partial'
        except Exception as error:
            row.update(status='failed', error=repr(error))
            (out/'failure.txt').write_text(traceback.format_exc())
        finally:
            row['observations'] = j.observations
            row['cleanup_errors'] = j.close()
            if row['cleanup_errors']: row['status'] = 'failed-cleanup'
            (a.output/'result.json').write_text(json.dumps(result, indent=2, ensure_ascii=False)+'\n')
        print(name + ': ' + row['status'], flush=True)
    assert hashes == {n:sha(binaries/n) for n in hashes}, 'binaries changed during measurement'
    assert all(r['status'] == 'passed-partial' for r in result['cases'].values()), 'journey failures; inspect result.json'


if __name__ == '__main__':
    main()
