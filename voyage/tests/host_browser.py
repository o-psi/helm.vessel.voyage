"""Offline #333 Linux process/browser journey. No Cargo build or personal state.

Runs two real Vessel-supervised voyages, scripted provider, loopback website and
Chromium DOM viewers, then real automatic suspension and fresh native/Web
owner preparation, client file/tab/scroll controls and an isolated adverse phase.
The latter kills only the fixture's identity-checked browser worker and briefly
withholds its actual guardian evidence before explicit cleanup. Evidence stays in a private temporary directory. Requires
existing playwright-core and ws; never installs dependencies. Nonzero on any
journey/cleanup failure; pre-teardown status is retained separately from cleanup.
"""
import argparse
import http.server
import html
import shutil
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import stat
import subprocess
import tempfile
import threading
import time
import traceback
import urllib.request
import urllib.parse
import uuid

from ui_journeys import launch as launch_tui, screen, send, pty_helpers


def uid():
    return str(uuid.uuid4())


def wait(fn, timeout=40):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = fn()
        if value:
            return value
        time.sleep(.1)
    raise AssertionError('timeout waiting for fixture state')


class Fixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        self.server.visits.append(self.path)
        route = urllib.parse.urlsplit(self.path)
        if route.path == '/fixture-counter':
            label = urllib.parse.parse_qs(route.query, strict_parsing=True).get('label', [''])[0]
            assert label in ('adverse-native', 'adverse-other')
            data = json.dumps({'count': self.server.counter_updates.get(label, 0)}).encode()
            self.send_response(200); self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
        if route.path.startswith('/ui/'):
            label = route.path.removeprefix('/ui/')
            assert label in ('local-owner', 'access-file', 'web-access-file', 'adverse-native', 'adverse-other')
            data = ('''<!doctype html><title>Synthetic UI ''' + label + '''</title>
<style>body{font:16px sans-serif;margin:16px;min-height:2400px}#scroll-result{position:fixed;right:12px;top:8px;background:white}</style>
<h1>Synthetic voyage browser</h1><p id="scroll-anchor">Scroll the task page</p>
<button id="counter" onclick="document.querySelector('#count').textContent=String(++window.fixtureCount);if(window.fixtureCounterLabel)fetch('/fixture-counter',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({label:window.fixtureCounterLabel,count:window.fixtureCount})})">Increment task counter</button>
<output id="count">0</output><script>window.fixtureCount=0</script>
<label>Upload fixture file <input id="upload" type="file" onchange="const f=this.files[0];if(f)f.text().then(text=>document.querySelector('#upload-result').textContent=f.name+'|'+text)"></label>
<output id="upload-result"></output><a href="/ui-download?label=''' + label + '''">Download fixture bytes</a>
<output id="scroll-result">Scrolled:0</output>
<script>window.fixtureCounterLabel=''' + json.dumps(label if label.startswith('adverse-') else None) + ''';addEventListener('scroll',()=>document.querySelector('#scroll-result').textContent='Scrolled:'+Math.round(scrollY))</script>''').encode()
            self.send_response(200); self.send_header('Content-Type', 'text/html'); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
        if route.path == '/ui-download':
            label = urllib.parse.parse_qs(route.query, strict_parsing=True).get('label', [''])[0]
            assert label in ('local-owner', 'access-file', 'web-access-file', 'adverse-native', 'adverse-other')
            data = ('SYNTHETIC_BROWSER_FILE_333:'+label+'\n').encode()
            self.send_response(200); self.send_header('Content-Type', 'application/octet-stream')
            self.send_header('Content-Disposition', 'attachment; filename="fixture-'+label+'.txt"')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
        if self.path == '/site-classes':
            data = ('''<!doctype html><title>Site classes</title>
<link rel="stylesheet" href="/site.css"><h1>Synthetic voyage browser</h1>
<button id="dynamic" onclick="document.querySelector('#result').textContent='Changed in task browser'">Change page</button>
<p id="result">Waiting for task action</p><div id="shadow"></div>
<iframe title="Cross-origin fixture" src="''' + self.server.child_site + '''/frame"></iframe>
<script>let root=document.querySelector('#shadow').attachShadow({mode:'open'});
root.innerHTML='<style>span{color:rgb(12,34,56)}</style><span>Open shadow content</span>';
document.cookie='fixture_asset=allowed; SameSite=Lax';</script>
<img id="authenticated-asset" src="/authenticated.svg"></img>''').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Set-Cookie', 'fixture_asset=allowed; SameSite=Lax; Path=/')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers(); self.wfile.write(data); return
        if self.path in ('/frame', '/site.css', '/authenticated.svg'):
            if self.path == '/frame':
                data, kind = b'<!doctype html><h1>Cross-origin child content</h1><button onclick="this.textContent=\'Child action observed\'">Child action</button>', 'text/html'
            elif self.path == '/site.css':
                data, kind = b'body{background:rgb(17,51,85)} #result{color:rgb(90,80,70)} iframe{width:500px;height:160px}', 'text/css'
            else:
                if 'fixture_asset=allowed' not in self.headers.get('Cookie', ''):
                    self.send_error(403); return
                data, kind = b'<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="green"/></svg>', 'image/svg+xml'
            self.send_response(200)
            self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers(); self.wfile.write(data); return
        if self.path in self.server.modules or self.path == '/receiver':
            data = (self.server.modules[self.path].read_bytes() if self.path in self.server.modules else
                    b'<!doctype html><meta name=viewport content="width=device-width, initial-scale=1"><link rel=stylesheet href=/viewer.css><script src=/rrweb-vendor.mjs></script><style>html,body{margin:0;height:100%;overflow:hidden}main{box-sizing:border-box;height:100dvh!important}</style><main id=viewer></main>')
            self.send_response(200)
            self.send_header('Content-Type', 'text/javascript' if self.path.endswith(('.mjs', '.js')) else 'text/css' if self.path.endswith('.css') else 'text/html')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers(); self.wfile.write(data); return
        if self.path == '/agent-frame':
            data = b'<body><input aria-label="Frame input"><button>Frame apply</button></body>'
            self.send_response(200); self.send_header('Content-Type', 'text/html'); self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data); return
        extras = ('<select aria-label="Agent choice"><option value="a">Alpha</option><option value="b">Beta</option></select><input type="checkbox" aria-label="Agent agree"><button ondblclick="this.textContent=\'Agent doubled\'">Agent double</button><iframe src="/agent-frame"></iframe>' if self.server.agent_interactions else '')
        data = ('<!doctype html><title>Synthetic ' + html.escape(self.path) + '</title><body style="background:#1b6579;color:white;font:48px sans-serif">'
                '<h1 style="font-size:24px">Synthetic voyage browser</h1><p>' + html.escape(self.path) + '</p><input autofocus>' +
                ('<script>setTimeout(()=>{document.querySelector("input").value=prompt("Synthetic modal","")||""},400)</script>' if self.path == '/modal' else '') +
                extras + '<canvas id="c" width="300" height="80"></canvas><script>let n=0;setInterval(()=>{'
                'let x=c.getContext("2d");x.fillStyle=n++%2?"orange":"blue";x.fillRect(0,0,300,80);},100)</script>').encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        if not 0 < length <= 4*1024*1024:
            self.send_error(413); return
        if self.path == '/fixture-counter':
            assert length <= 256
            value = json.loads(self.rfile.read(length))
            assert set(value) == {'label', 'count'} and value['label'] in ('adverse-native', 'adverse-other')
            assert isinstance(value['count'], int) and not isinstance(value['count'], bool) and 0 <= value['count'] <= 8
            self.server.counter_updates[value['label']] = max(self.server.counter_updates.get(value['label'], 0), value['count'])
            self.send_response(204); self.send_header('Content-Length', '0'); self.end_headers(); return
        if self.path == '/fixture-control':
            if length > 4096 or self.headers.get('Authorization') != 'Bearer '+getattr(self.server, 'control_token', ''):
                self.send_error(403); return
            try:
                result = self.server.fixture_control(json.loads(self.rfile.read(length)))
                data = json.dumps(result).encode()
                self.send_response(200)
            except Exception:
                # The private root report retains the failure. Never return
                # process paths, tokens, page content or a Python traceback.
                self.server.control_failed = True
                data = b'{"error":"fixture control refused"}'
                self.send_response(500)
            self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(data)))
            self.end_headers(); self.wfile.write(data); return
        try:
            body = json.loads(self.rfile.read(length))
            label = next(m['content'] for m in body['messages'] if m['role'] == 'user')
            assert label in ('orchard', 'harbor'), label
            self.server.requests.append(body)
            tools = [m for m in body['messages'] if m['role'] == 'tool']
            if not tools:
                assert any(t['function']['name'] == 'host_browser' for t in body['tools'])
                delta = {'tool_calls': [{'index': 0, 'id': 'navigate-'+label, 'type': 'function',
                         'function': {'name': 'host_browser', 'arguments': json.dumps({
                             'action': 'navigate', 'url': self.server.site+'/'+label})}}]}
            elif self.server.agent_interactions and len(tools) <= 17:
                step = len(tools)
                def observed():
                    values = [json.loads(m['content']) for m in reversed(tools) if m['content'].startswith('{')]
                    return next(v for v in values if isinstance(v, dict) and 'elements' in v)
                def reference(label):
                    return next(e['ref'] for e in observed()['elements'] if e['text'] == label)
                actions = {
                    1: lambda: {'action': 'inspect', 'limit': 16},
                    2: lambda: {'action': 'fill', 'reference': next(e['ref'] for e in observed()['elements'] if e['tag'] == 'input'), 'text': 'Agent grounded input'},
                    3: lambda: {'action': 'inspect'},
                    4: lambda: {'action': 'select', 'reference': reference('Agent choice'), 'value': 'b'},
                    5: lambda: {'action': 'inspect'},
                    6: lambda: {'action': 'check', 'reference': reference('Agent agree'), 'checked': True},
                    7: lambda: {'action': 'inspect'},
                    8: lambda: {'action': 'key', 'reference': reference('Agent agree'), 'key': 'Escape'},
                    9: lambda: {'action': 'inspect'},
                    10: lambda: {'action': 'double_click', 'reference': reference('Agent double')},
                    11: lambda: {'action': 'inspect'},
                    12: lambda: {'action': 'inspect', 'frame': next(f['frame'] for f in observed()['frames'] if f['url'].endswith('/agent-frame'))},
                    13: lambda: {'action': 'fill', 'reference': reference('Frame input'), 'text': 'Agent child input'},
                    14: lambda: {'action': 'diagnostics'},
                    15: lambda: {'action': 'history', 'direction': 'reload'},
                    16: lambda: {'action': 'inspect'},
                    17: lambda: {'action': 'read', 'reference': reference('Agent double')},
                }
                # Native state, observed by the runtime, must agree before reload.
                if step == 12:
                    state = observed()['elements']
                    assert next(e for e in state if e['text'] == 'Agent choice')['selected'] == ['Beta']
                    assert next(e for e in state if e['text'] == 'Agent agree')['checked'] is True
                    assert any(e['text'] == 'Agent doubled' for e in state)
                action = actions[step]()
                delta = {'tool_calls': [{'index': 0, 'id': f'agent-{label}-{step}', 'type': 'function',
                    'function': {'name': 'host_browser', 'arguments': json.dumps(action)}}]}
            else:
                if self.server.agent_interactions:
                    assert len(tools) == 18
                    assert json.loads(tools[-1]['content'])['text'] == 'Agent double'
                self.server.arrived[label] = tools[-1]
                assert self.server.release.wait(300), 'human journey barrier timeout'
                delta = {'content': 'Synthetic browser journey finished.'}
            events = [{'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]},
                      {'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls' if 'tool_calls' in delta else 'stop'}]}]
            data = (''.join('data: '+json.dumps(e)+'\n\n' for e in events)+'data: [DONE]\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
        except Exception as exc:
            self.server.errors.append(repr(exc))
            data = b'{"error":{"message":"synthetic fixture failed"}}'
            self.send_response(500)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--agent-interactions', action='store_true', help='Exercise #379 agent controls before both Helm viewers')
    parser.add_argument('--node', type=Path, default=Path('/usr/bin/node'))
    parser.add_argument('--evidence-dir', type=Path, help='Existing ignored target directory for pre-private screenshots (runtime evidence uses short /tmp path)')
    parser.add_argument('--binaries', type=Path, required=True)
    parser.add_argument('--web-resources', type=Path, required=True,
                        help='resources/js directory from the matching o-psi/webhelm checkout')
    parser.add_argument('--ws', type=Path, help='Existing ws package; defaults to the matching Web checkout node_modules/ws')
    parser.add_argument('--chromium', type=Path, default=Path('/usr/bin/chromium'))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    binaries = args.binaries.resolve()
    web_resources = args.web_resources.resolve()
    if args.ws is None:
        args.ws = web_resources.parents[1]/'node_modules/ws'
    for name in ('host-browser.js', 'vessel-client.js', 'connection-diagnostics.js'):
        if not (web_resources/name).is_file():
            parser.error(f'--web-resources is missing {name}')
    web_shared = web_resources.parents[1]/'shared'
    for path in ('voyage/browser/rrweb-vendor.mjs', 'helm/browser-view/viewer.mjs'):
        if not (web_shared/path).is_file():
            parser.error(f'--web-resources checkout is missing shared/{path}')
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix='host-browser333-'))
    screenshots = args.evidence_dir.resolve() if args.evidence_dir else root
    print('evidence:', root, flush=True)
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'PROVIDER_FIXTURE_KEY': 'synthetic-only'}
    for key in ('HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME', 'TMPDIR'):
        path = root/key.lower(); path.mkdir(mode=0o700); env[key] = str(path)
    (Path(env['XDG_STATE_HOME'])/'voyage').mkdir(mode=0o700)
    workspace = root/'workspace'; workspace.mkdir()
    directory = root/'vessel'
    report = {'sessions': [], 'pre_teardown': [], 'cleanup': {},
              'binary_sha256': {name: hashlib.file_digest((binaries/name).open('rb'), 'sha256').hexdigest()
                                for name in ('helm', 'vessel', 'voyage')},
              'source_sha256': {name: hashlib.sha256((repo/name).read_bytes()).hexdigest() for name in
                               ('voyage/tests/host_browser.py', 'voyage/tests/host_browser_viewer.mjs',
                                'voyage/browser/worker.mjs', 'helm/browser-view/viewer.mjs')},
              'web_source_sha256': {name: hashlib.sha256((web_resources/name).read_bytes()).hexdigest()
                                   for name in ('host-browser.js', 'vessel-client.js', 'connection-diagnostics.js')}}
    fixture_principals = {}
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    server.modules = {'/viewer.css': repo/'helm/browser-view/viewer.css', '/viewer.mjs': repo/'helm/browser-view/viewer.mjs',
                      '/helm/browser-view/viewer.mjs': repo/'helm/browser-view/viewer.mjs'}
    server.modules['/rrweb-vendor.mjs'] = repo/'voyage/browser/rrweb-vendor.mjs'
    server.modules['/voyage/browser/rrweb-vendor.mjs'] = repo/'voyage/browser/rrweb-vendor.mjs'
    server.modules['/shared/voyage/browser/rrweb-vendor.mjs'] = web_shared/'voyage/browser/rrweb-vendor.mjs'
    server.modules['/shared/helm/browser-view/viewer.mjs'] = web_shared/'helm/browser-view/viewer.mjs'
    server.modules['/web/shared/voyage/browser/rrweb-vendor.mjs'] = web_shared/'voyage/browser/rrweb-vendor.mjs'
    server.modules['/web/shared/helm/browser-view/viewer.mjs'] = web_shared/'helm/browser-view/viewer.mjs'
    for name in ('host-browser.js', 'vessel-client.js', 'connection-diagnostics.js'):
        server.modules['/web/resources/js/'+name] = web_resources/name
    server.site = f'http://127.0.0.1:{server.server_port}'
    child_server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    child_server.modules, child_server.visits = {}, []
    child_server.site = f'http://127.0.0.1:{child_server.server_port}'
    server.child_site = child_server.site
    child_thread = threading.Thread(target=child_server.serve_forever, daemon=True); child_thread.start()
    server.agent_interactions = args.agent_interactions
    server.requests, server.errors, server.visits, server.arrived = [], [], [], {}
    server.control_token, server.control_failed = uid(), False
    server.counter_updates = {}
    server.release = threading.Event()
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    log = (root/'vessel.log').open('wb')
    supervisor = None
    gateway = None
    launchers = []
    tui_clients = []
    opener = root/'bin'; opener.mkdir()
    (opener/'xdg-open').write_text('#!/usr/bin/python3\nimport os,pathlib,sys\np=pathlib.Path(sys.argv[1]).resolve()\nassert p.is_relative_to(pathlib.Path(os.environ["HOME"]).parent) and p.name == "open.html"\npathlib.Path(os.environ["FIXTURE_LAUNCHER"]).write_text(str(p))\n')
    (opener/'xdg-open').chmod(0o700)

    def cli(*parts):
        p = subprocess.run([str(binaries/'vessel'), *parts], env=env, cwd=workspace,
                           capture_output=True, text=True, timeout=25)
        assert p.returncode == 0, p.stderr
        return p.stdout

    def request(command, allow_error=False):
        cred = json.loads((directory/'process-http.json').read_text())
        req = urllib.request.Request(cred['endpoint']+'/v1/vessel/command',
            data=json.dumps({'protocol': 1, 'command': command}).encode(),
            headers={'Authorization': 'Bearer '+cred['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=35) as response:
            result = json.load(response)
        if allow_error:
            return result
        assert result.get('error') is None, result
        return result['result']

    def voyage(session, **command):
        reply = request({'session_id': session, **command})
        assert reply.get('error') is None, reply
        return reply['result']

    def owned():
        found = []
        marker = ('HOME='+env['HOME']).encode()
        browser_home = ('HOME='+str(directory)).encode()
        for p in Path('/proc').iterdir():
            if p.name.isdigit():
                try:
                    values = (p/'environ').read_bytes().split(b'\0')
                    if marker in values or any(v.startswith(browser_home+b'/sessions/') for v in values):
                        found.append(int(p.name))
                except (OSError, ProcessLookupError):
                    pass
        return found

    # Fixture-only adverse control. Requests have no arbitrary path, PID,
    # signal or script parameter. A pidfd and proc starttime bind the one
    # worker effect to this private fixture's selected Voyage.
    adverse = {}

    def private_bytes(path, limit=4096):
        metadata = path.lstat()
        assert stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.getuid()
        assert metadata.st_nlink == 1 and not metadata.st_mode & 0o077 and metadata.st_size <= limit
        assert path.resolve() == path and path.is_relative_to(root)
        return path.read_bytes()

    def process_identity(pid):
        try:
            fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
            return fields[19], fields[0]
        except (FileNotFoundError, ProcessLookupError):
            return None

    def fixture_control(command):
        action = command.get('action')
        expected_keys = {'action', 'session', 'command_id'} if action in ('pin-saved-receipt', 'check-saved-receipt') else {'action', 'session'}
        assert set(command) == expected_keys
        session = command['session']
        assert session in [item['session'] for item in report['sessions']]
        worker_root = directory/'sessions'/session/'journal'/'host-browser'
        marker, held = worker_root/'guardian-cleanup.json', worker_root/'guardian-cleanup.fixture-held.json'
        if action == 'crash-worker':
            assert session not in adverse, 'fixture crash is one-shot'
            lock_bytes = private_bytes(worker_root/'worker.lock')
            lock = json.loads(lock_bytes)
            pid = lock['pid']; assert isinstance(pid, int) and 1 < pid < 2**31
            identity = process_identity(pid); assert identity and identity[1] != 'Z'
            fd = os.pidfd_open(pid)
            try:
                now = process_identity(pid)
                assert now and now[0] == identity[0] and now[1] != 'Z'
                args_now = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
                assert args_now[:2] == [os.fsencode(args.node.resolve()), os.fsencode(repo/'voyage/browser/worker.mjs')]
                environment = Path(f'/proc/{pid}/environ').read_bytes().split(b'\0')
                assert b'HOME='+os.fsencode(worker_root) in environment
                scratch = Path(os.fsdecode(next(value.removeprefix(b'TMPDIR=') for value in environment if value.startswith(b'TMPDIR='))))
                scratch_meta = scratch.lstat()
                assert scratch.parent == Path('/tmp') and scratch.name.startswith('vhb-')
                assert stat.S_ISDIR(scratch_meta.st_mode) and scratch_meta.st_uid == os.getuid() and not scratch_meta.st_mode & 0o077
                tracked = {}
                for candidate in Path('/proc').iterdir():
                    if candidate.name.isdecimal():
                        try:
                            if b'HOME='+os.fsencode(worker_root) in (candidate/'environ').read_bytes().split(b'\0'):
                                seen = process_identity(int(candidate.name))
                                if seen: tracked[int(candidate.name)] = seen[0]
                        except OSError:
                            pass
                adverse[session] = {'worker': pid, 'start': identity[0], 'scratch': scratch, 'tracked': tracked,
                    'lock_sha256': hashlib.sha256(lock_bytes).hexdigest()}
                now = process_identity(pid)
                assert now and now[0] == identity[0] and now[1] != 'Z'
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            finally:
                os.close(fd)
            wait(lambda: marker.exists(), 12)
            return {'worker_signal_sent': True, 'guardian_marker_written': True}
        assert session in adverse
        if action in ('pin-saved-receipt', 'check-saved-receipt'):
            command_id = str(uuid.UUID(command['command_id']))
            assert command_id == command['command_id'] and command_id != str(uuid.UUID(int=0))
            database = worker_root/'receipts.sqlite3'
            # This is direct fixture storage observation, not a public saved-
            # receipt API or permission to prepare a replacement owner.
            private_bytes(database, 16*1024*1024)
            connection = sqlite3.connect(database.as_uri()+'?mode=ro', uri=True, timeout=1.0)
            try:
                connection.execute('PRAGMA query_only=ON')
                row = connection.execute('SELECT principal,digest,state FROM receipts WHERE id=?', (command_id,)).fetchone()
            finally:
                connection.close()
            assert row is not None and row[0] == fixture_principals[session]
            assert len(row[1]) == 64 and all(char in '0123456789abcdef' for char in row[1])
            assert row[2] == 'unknown'
            saved = (command_id, *row)
            if action == 'pin-saved-receipt':
                assert 'saved_receipt' not in adverse[session]
                adverse[session]['saved_receipt'] = saved
            else:
                assert saved == adverse[session]['saved_receipt'], 'original exact receipt changed'
            return {'state': 'unknown', 'original_principal_matches': True,
                    'exact_receipt_unchanged': action == 'check-saved-receipt', 'read_only_storage': True}
        if action == 'withhold-cleanup':
            assert not held.exists()
            record = private_bytes(marker)
            value = json.loads(record)
            assert all(value.get(key) is True for key in ('observed', 'cleanup_complete', 'descendants_terminated', 'descendants_reaped', 'temporary_cleaned'))
            adverse[session]['marker_sha256'] = hashlib.sha256(record).hexdigest()
            adverse[session]['marker_identity'] = (marker.stat().st_dev, marker.stat().st_ino)
            marker.rename(held)
            return {'cleanup_evidence_withheld': True}
        if action == 'restore-cleanup':
            record = private_bytes(held)
            assert not marker.exists()
            assert hashlib.sha256(record).hexdigest() == adverse[session]['marker_sha256']
            assert (held.stat().st_dev, held.stat().st_ino) == adverse[session]['marker_identity']
            held.rename(marker)
            return {'same_cleanup_evidence_restored': True}
        assert action == 'cleanup-state'
        live = zombies = 0
        for pid, start in adverse[session]['tracked'].items():
            now = process_identity(pid)
            if now and now[0] == start:
                if now[1] == 'Z': zombies += 1
                else: live += 1
        capacity = Path(env['XDG_DATA_HOME'])/'helm'/'host-browser-capacity'
        slots = sum(private_bytes(path, 128).decode() == session for path in capacity.glob('browser-slot-*'))
        record = json.loads(private_bytes(marker)) if marker.exists() else None
        lock = worker_root/'worker.lock'
        return {'live_owned_browser_processes': live, 'owned_browser_zombies': zombies,
                'scratch_removed': not adverse[session]['scratch'].exists(),
                'worker_lock_retained': lock.exists(),
                'worker_lock_unchanged': lock.exists() and hashlib.sha256(private_bytes(lock)).hexdigest() == adverse[session]['lock_sha256'],
                'cleanup_evidence_available': record is not None, 'capacity_slots_retained': slots,
                'guardian_observed': record.get('observed') if record else False,
                'external_actions_reconciled': record.get('external_actions_reconciled') if record else False}

    server.fixture_control = fixture_control

    try:
        for name in ('helm', 'vessel', 'voyage'):
            assert (binaries/name).is_file(), name
        assert args.ws.is_dir() and (repo/'voyage/browser/node_modules/playwright-core').is_dir()
        supervisor = subprocess.Popen([str(binaries/'vessel'), 'local-serve', '--directory', str(directory), '--voyage-binary', str(binaries/'voyage')],
            env=env, cwd=workspace, stdout=log, stderr=log)
        wait(lambda: (directory/'process-http.json').exists())
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
        endpoint = f'http://127.0.0.1:{port}'
        gateway = subprocess.Popen([str(binaries/'vessel'), '--bind', f'127.0.0.1:{port}',
            '--database', str(root/'gateway.db'), '--process-directory', str(directory),
            '--allow-insecure-loopback', '--public-origin', endpoint],
            env=env, cwd=workspace, stdout=log, stderr=log)
        time.sleep(1)
        assert gateway.poll() is None, 'gateway exited'
        connection = json.loads(cli('auth', 'accounts', 'connect', '--label', 'synthetic', '--endpoint',
                                    server.site+'/v1', '--transports', 'openai-chat'))
        account = json.loads(cli('auth', 'accounts', 'add', '--connection', connection['id'],
                                 '--account', 'fixture', '--env', 'PROVIDER_FIXTURE_KEY'))
        binding = {k: account[k] for k in ('identity_generation',)}
        binding.update(account_id=account['id'], connection_id=connection['id'],
                       connection_revision=connection['revision'], transport='openai_chat')
        request({'op': 'account_set_default', 'command_id': uid(), 'workspace': str(workspace),
                 'account': binding, 'expected_revision': 0})
        config = {'provider': 'openai-chat', 'model': 'fixture-model', 'api_key_required': False,
                  'base_url': server.site+'/v1', 'provider_retry_attempts': 1,
                  # The two-client human barrier is bounded at 300s; the normal
                  # 60s response deadline must not retire its browser mid-journey.
                  'provider_response_timeout_ms': 300000,
                  'access': 'unrestricted', 'context_window': 0, 'account': binding,
                  'host_browser_launch': {'node': str(args.node.resolve()), 'worker': str(repo/'voyage/browser/worker.mjs'),
                      'chromium': str(args.chromium), 'config': {'public_web': True, 'origins': [{'origin': origin, 'private_network': True} for origin in (server.site, child_server.site)],
                       'width': 1280, 'height': 720}}}
        config_path = root/'config.json'
        config_path.write_text(json.dumps({'version': 1, 'workspace': str(workspace), 'config': config,
            'explicit': {'access': 'unrestricted'}, 'selection': None, 'confirmation': None}))
        for label in ('orchard', 'harbor'):
            session = uid()
            request({'op': 'start_settings', 'session_id': session, 'command_id': uid(),
                     'workspace': str(workspace), 'config_path': str(config_path), 'binding': binding, 'settings': {}})
            item = {'session': session, 'label': label}; report['sessions'].append(item)
            snap = voyage(session, op='snapshot')
            voyage(session, op='submit', command_id=uid(), expected_revision=snap['revision'], expires_at_ms=int(time.time()*1000)+60000, prompt=label)
        def agent_arrived():
            assert not server.errors, server.errors
            return len(server.arrived) == 2
        wait(agent_arrived, 70)
        (root/'provider-before-viewer.json').write_text(json.dumps({'arrived': server.arrived, 'visits': server.visits}, indent=2))
        for item in report['sessions']:
            inspection = request({'op': 'inspect', 'session_id': item['session']})
            (root/(item['label']+'-inspect.json')).write_text(json.dumps(inspection, indent=2))
            item['incarnation'] = inspection['incarnation']
            access = root/(item['label']+'-access.json')
            fixture_principals[item['session']] = uid()
            cli('process-grant', '--directory', str(directory), '--output', str(access),
                '--session', item['session'], '--principal', fixture_principals[item['session']], '--workspace', str(workspace),
                '--endpoint', endpoint, '--rights', 'observe,execute,history')
            item['access'] = str(access)
        # Native CLI inspects the snapshot, hence the explicit fixture history right above.
        for index, item in enumerate(report['sessions']):
            captured = root/(item['label']+'-launcher-path')
            launch_env = {**env, 'PATH': str(opener)+':'+env['PATH'], 'FIXTURE_LAUNCHER': str(captured)}
            route = ['--directory', str(directory)] if index == 0 else ['--access-file', item['access']]
            if index == 1:
                # The scoped route contains exactly this voyage. Exercise the
                # real TUI key path, not just the native viewer CLI entry point.
                tui = launch_tui([str(binaries/'helm'), 'connect', *route, '--no-start'],
                    launch_env, workspace, root/'browser-tui.pty', 120, 36)
                tui_clients.append(tui)
                wait(lambda: item['label'] in screen(tui) or tui['process'].poll() is not None)
                assert tui['process'].poll() is None, 'TUI exited before F6'
                send(tui, '\x1b[17~')
                launch = tui['process']
                item['native_entry'] = 'tui-f6'
            else:
                launch = subprocess.Popen([str(binaries/'helm'), 'connect', *route, 'browser', item['session']],
                    env=launch_env, cwd=workspace, stdout=subprocess.DEVNULL, stderr=(root/(item['label']+'-native-error.log')).open('wb'))
                launchers.append(launch)
                item['native_entry'] = 'cli'
            wait(lambda: captured.exists() or launch.poll() is not None)
            assert captured.exists(), 'native launcher exited before private fixture capture'
            item['launcher'] = captured.read_text()
            item['native_route'] = 'local-owner' if index == 0 else 'access-file'
        report['gaps'] = ['Linux headless Chromium; no native macOS/Windows or personal desktop browser evidence.']
        viewer = root/'viewer.json'
        viewer.write_text(json.dumps({'sessions': report['sessions'], 'ws': str(args.ws.resolve()),
            'playwright': str(repo/'voyage/browser/node_modules/playwright-core'), 'chromium': str(args.chromium),
            'site': server.site, 'screenshots': str(screenshots), 'evidence': str(root/'viewer-evidence.json'),
            'fixture_control_token': server.control_token}))
        with (root/'viewer.log').open('wb') as output:
            result = subprocess.run([str(args.node.resolve()), str(Path(__file__).with_name('host_browser_viewer.mjs')), str(viewer)],
                env=env, cwd=workspace, stdout=output, stderr=output, timeout=260)
        assert result.returncode == 0, f'viewer failed ({result.returncode}); see viewer-evidence.json/viewer.log'
        assert all(path in server.visits for path in ('/history-one', '/history-two', '/modal', '/private', '/native-local-owner', '/native-access-file')), server.visits
        assert all(path in server.visits for path in ('/site-classes', '/site.css', '/authenticated.svg')), server.visits
        assert '/frame' in child_server.visits, child_server.visits
        for launch in launchers:
            launch.send_signal(signal.SIGINT)
            assert launch.wait(timeout=15) == 0, 'native viewer cleanup failed'
        for tui in tui_clients:
            pty_helpers.stop_pty(tui, wait)
            assert tui['process'].returncode == 0, 'TUI detach failed'
        server.release.set()
        for item in report['sessions']:
            def completed():
                reply = request({'session_id': item['session'], 'op': 'snapshot'}, allow_error=True)
                if reply.get('error') == 'suspended observation unavailable' and not reply.get('outcome_unknown'):
                    return None
                assert reply.get('error') is None, reply
                saved = reply['result']['result']
                return saved if saved.get('run', {}).get('state') == 'completed' and saved.get('pending_cleanup_run') is None else None
            snap = wait(completed)
            assert snap.get('pending_cleanup_run') is None, snap
            assert '/private' not in json.dumps(snap['messages']), 'private human URL leaked into conversation'
            assert 'SYNTHETIC_PRIVATE_INPUT_333' not in json.dumps(snap['messages']), 'private input leaked into conversation'
            assert 'SYNTHETIC_DUAL_PRIVATE_333' not in json.dumps(snap['messages']), 'dual-viewer private input leaked'
            assert 'SYNTHETIC_BROWSER_FILE_333' not in json.dumps(snap['messages']), 'private file bytes leaked into conversation'
            assert all(m.get('tool_outcome', {}).get('execution') == 'succeeded' for m in snap['messages'] if m['role'] == 'tool')
        # No mock stopped marker or explicit shutdown: observe automatic owner suspension.
        # Each receiver is fresh; no prior browser attachment or socket can wake the owner.
        for phase in ('suspended-native', 'suspended-web', 'adverse'):
            for index, item in enumerate(report['sessions']):
                inspection = wait(lambda: (info if (info := request({
                    'op': 'inspect', 'session_id': item['session']}))['state'] == 'suspended' else None), 70)
                item['incarnation'] = inspection['incarnation']
                report.setdefault('suspended', []).append({'phase': phase, 'session': item['session'],
                    'state': inspection['state'], 'incarnation': inspection['incarnation']})
                if phase in ('suspended-native', 'adverse'):
                    item['native_entry'] = 'cli'
                    captured = root/(item['label']+'-'+phase+'-launcher-path')
                    launch_env = {**env, 'PATH': str(opener)+':'+env['PATH'], 'FIXTURE_LAUNCHER': str(captured)}
                    route = ['--directory', str(directory)] if index == 0 else ['--access-file', item['access']]
                    launch = subprocess.Popen([str(binaries/'helm'), 'connect', *route, 'browser', item['session']],
                        env=launch_env, cwd=workspace, stdout=subprocess.DEVNULL,
                        stderr=(root/(item['label']+'-suspended-native-error.log')).open('wb'))
                    launchers.append(launch)
                    wait(lambda: captured.exists() or launch.poll() is not None)
                    assert captured.exists(), 'suspended native launcher exited before capture'
                    item['launcher'] = captured.read_text()
                    assert request({'op': 'inspect', 'session_id': item['session']})['state'] == 'suspended', 'launcher woke owner before Status'
            phase_config = root/(phase+'.json')
            phase_config.write_text(json.dumps({**json.loads(viewer.read_text()), 'sessions': report['sessions'],
                'mode': phase, 'evidence': str(root/(phase+'-evidence.json'))}))
            with (root/(phase+'.log')).open('wb') as output:
                result = subprocess.run([str(args.node.resolve()), str(Path(__file__).with_name('host_browser_viewer.mjs')), str(phase_config)],
                    env=env, cwd=workspace, stdout=output, stderr=output, timeout=200)
            assert result.returncode == 0, f'{phase} failed ({result.returncode}); inspect sanitized phase evidence'
            for launch in launchers:
                if launch.poll() is None:
                    launch.send_signal(signal.SIGINT)
                    assert launch.wait(timeout=15) == 0, 'suspended native viewer cleanup failed'
            for item in report['sessions']:
                inspection = wait(lambda: (info if (info := request({
                    'op': 'inspect', 'session_id': item['session']}))['state'] == 'suspended' else None), 70)
                assert inspection['incarnation'] != item['incarnation'], 'no actual owner preparation observed'
        assert len(server.requests) == (38 if args.agent_interactions else 4), 'viewer preparation must not invoke provider or replay the run'
        assert not server.errors, server.errors
        assert not server.control_failed, 'adverse fixture control failed'
        report['actions'] = 'passed'
    except Exception:
        report['failure'] = traceback.format_exc()
        print(report['failure'], flush=True)
    finally:
        for tui in tui_clients:
            try:
                pty_helpers.stop_pty(tui, wait)
            except Exception as exc:
                report['cleanup'].setdefault('tui_errors', []).append(type(exc).__name__)
        for item in report['sessions']:
            try:
                report['pre_teardown'].append({'session': item['session'], 'snapshot': voyage(item['session'], op='snapshot'),
                    'inspect': request({'op': 'inspect', 'session_id': item['session']})})
            except Exception as exc:
                report['pre_teardown'].append({'error': str(exc)})
        (root/'report.json').write_text(json.dumps(report, indent=2))
        server.release.set()
        # Restore only this fixture's exact held evidence on a failed assertion
        # before stopping its Voyage. The failed journey stays failed.
        for session in adverse:
            held = directory/'sessions'/session/'journal'/'host-browser'/'guardian-cleanup.fixture-held.json'
            if held.exists():
                try: fixture_control({'action': 'restore-cleanup', 'session': session})
                except Exception: report['cleanup']['held_evidence_unresolved'] = True
        for pid in owned():
            try:
                fd = os.pidfd_open(pid)
                try:
                    if pid in owned(): signal.pidfd_send_signal(fd, signal.SIGTERM)
                finally: os.close(fd)
            except ProcessLookupError: pass
        try:
            wait(lambda: not owned(), 15)
        except AssertionError:
            report['cleanup']['forced_pids'] = owned()
            for pid in owned():
                try:
                    fd = os.pidfd_open(pid)
                    try:
                        if pid in owned(): signal.pidfd_send_signal(fd, signal.SIGKILL)
                    finally: os.close(fd)
                except ProcessLookupError: pass
        if supervisor: supervisor.wait(timeout=10)
        if gateway: gateway.wait(timeout=10)
        server.shutdown(); server.server_close(); thread.join(timeout=5); log.close()
        child_server.shutdown(); child_server.server_close(); child_thread.join(timeout=5)
        report['cleanup'].update(remaining_owned_pids=owned(), server_thread_alive=thread.is_alive())
        report['cleanup']['child_server_thread_alive'] = child_thread.is_alive()
        (root/'provider.json').write_text(json.dumps({'requests': server.requests, 'errors': server.errors, 'arrived': server.arrived, 'visits': server.visits}, indent=2))
        if report.get('actions') == 'passed' and not report['cleanup'].get('forced_pids') and not owned() and not thread.is_alive() and not child_thread.is_alive() and not report['cleanup'].get('tui_errors') and not report['cleanup'].get('held_evidence_unresolved'):
            report['journey'] = 'passed'
        (root/'report.json').write_text(json.dumps(report, indent=2))
    assert report.get('journey') == 'passed' and not report['cleanup'].get('forced_pids') and not owned(), str(root)
    print('PASS: two real voyages, actual TUI/native and Web adapter, private handoff, exact file/tab/scroll controls, bounded stalled viewer, worker crash/retained cleanup uncertainty, suspended preparation and observed cleanup')


if __name__ == '__main__':
    main()
