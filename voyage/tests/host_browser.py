"""Offline #333 Linux process/browser journey. No Cargo build or personal state.

Runs two real Vessel-supervised voyages, scripted provider, loopback website and
Chromium DOM viewers, then real automatic suspension and fresh native/Web
owner preparation with replayed pages and explicit close. Evidence stays in a private temporary directory. Requires
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
import subprocess
import tempfile
import threading
import time
import traceback
import urllib.request
import uuid


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
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
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
                assert self.server.release.wait(240), 'human journey barrier timeout'
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
    parser.add_argument('--ws', type=Path, default=Path('/home/psi/voyage/web/gateway/node_modules/ws'))
    parser.add_argument('--chromium', type=Path, default=Path('/usr/bin/chromium'))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    binaries = args.binaries.resolve()
    web_resources = args.web_resources.resolve()
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
                                for name in ('helm', 'vessel', 'voyage')}}
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
    server.agent_interactions = args.agent_interactions
    server.requests, server.errors, server.visits, server.arrived = [], [], [], {}
    server.release = threading.Event()
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    log = (root/'vessel.log').open('wb')
    supervisor = None
    gateway = None
    launchers = []
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
            '--allow-insecure-loopback', endpoint], env=env, cwd=workspace, stdout=log, stderr=log)
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
                  'access': 'unrestricted', 'context_window': 0, 'account': binding,
                  'host_browser_launch': {'node': str(args.node.resolve()), 'worker': str(repo/'voyage/browser/worker.mjs'),
                      'chromium': str(args.chromium), 'config': {'public_web': True, 'origins': [{'origin': server.site, 'private_network': True}],
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
            cli('process-grant', '--directory', str(directory), '--output', str(access),
                '--session', item['session'], '--principal', uid(), '--workspace', str(workspace),
                '--endpoint', endpoint, '--rights', 'observe,execute,history')
            item['access'] = str(access)
        # Native CLI inspects the snapshot, hence the explicit fixture history right above.
        for index, item in enumerate(report['sessions']):
            captured = root/(item['label']+'-launcher-path')
            launch_env = {**env, 'PATH': str(opener)+':'+env['PATH'], 'FIXTURE_LAUNCHER': str(captured)}
            route = ['--directory', str(directory)] if index == 0 else ['--access-file', item['access']]
            launch = subprocess.Popen([str(binaries/'helm'), 'connect', *route, 'browser', item['session']],
                env=launch_env, cwd=workspace, stdout=subprocess.DEVNULL, stderr=(root/(item['label']+'-native-error.log')).open('wb'))
            launchers.append(launch)
            wait(lambda: captured.exists() or launch.poll() is not None)
            assert captured.exists(), 'native launcher exited before private fixture capture'
            item['launcher'] = captured.read_text()
            item['native_route'] = 'local-owner' if index == 0 else 'access-file'
        report['gaps'] = ['Linux headless Chromium; no native macOS/Windows or personal desktop browser evidence.']
        viewer = root/'viewer.json'
        viewer.write_text(json.dumps({'sessions': report['sessions'], 'ws': str(args.ws.resolve()),
            'playwright': str(repo/'voyage/browser/node_modules/playwright-core'), 'chromium': str(args.chromium),
            'site': server.site, 'screenshots': str(screenshots), 'evidence': str(root/'viewer-evidence.json')}))
        with (root/'viewer.log').open('wb') as output:
            result = subprocess.run([str(args.node.resolve()), str(Path(__file__).with_name('host_browser_viewer.mjs')), str(viewer)],
                env=env, cwd=workspace, stdout=output, stderr=output, timeout=200)
        assert result.returncode == 0, f'viewer failed ({result.returncode}); see viewer-evidence.json/viewer.log'
        assert all(path in server.visits for path in ('/history-one', '/history-two', '/modal', '/private', '/native-local-owner', '/native-access-file')), server.visits
        for launch in launchers:
            launch.send_signal(signal.SIGINT)
            assert launch.wait(timeout=15) == 0, 'native viewer cleanup failed'
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
            assert all(m.get('tool_outcome', {}).get('execution') == 'succeeded' for m in snap['messages'] if m['role'] == 'tool')
        # No mock stopped marker or explicit shutdown: observe automatic owner suspension.
        # Each receiver is fresh; no prior browser attachment or socket can wake the owner.
        for phase in ('suspended-native', 'suspended-web'):
            for index, item in enumerate(report['sessions']):
                inspection = wait(lambda: (info if (info := request({
                    'op': 'inspect', 'session_id': item['session']}))['state'] == 'suspended' else None), 70)
                item['incarnation'] = inspection['incarnation']
                report.setdefault('suspended', []).append({'phase': phase, 'session': item['session'],
                    'state': inspection['state'], 'incarnation': inspection['incarnation']})
                if phase == 'suspended-native':
                    captured = root/(item['label']+'-suspended-launcher-path')
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
        report['actions'] = 'passed'
    except Exception:
        report['failure'] = traceback.format_exc()
        print(report['failure'], flush=True)
    finally:
        for item in report['sessions']:
            try:
                report['pre_teardown'].append({'session': item['session'], 'snapshot': voyage(item['session'], op='snapshot'),
                    'inspect': request({'op': 'inspect', 'session_id': item['session']})})
            except Exception as exc:
                report['pre_teardown'].append({'error': str(exc)})
        (root/'report.json').write_text(json.dumps(report, indent=2))
        server.release.set()
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
        report['cleanup'].update(remaining_owned_pids=owned(), server_thread_alive=thread.is_alive())
        (root/'provider.json').write_text(json.dumps({'requests': server.requests, 'errors': server.errors, 'arrived': server.arrived, 'visits': server.visits}, indent=2))
        if report.get('actions') == 'passed' and not report['cleanup'].get('forced_pids') and not owned() and not thread.is_alive():
            report['journey'] = 'passed'
        (root/'report.json').write_text(json.dumps(report, indent=2))
    assert report.get('journey') == 'passed' and not report['cleanup'].get('forced_pids') and not owned(), str(root)
    print('PASS: two real voyages, shared mounted viewer, native local/access-file launchers, replayed DOM, private UI control, history/title, modal/IME, conversation fence, actual suspended native/Web preparation and cleanup')


if __name__ == '__main__':
    main()
