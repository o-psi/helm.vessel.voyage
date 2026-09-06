#!/usr/bin/env python3
"""Actual policy keys, native tool authority, private profile drift and MCP cleanup."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from policy_ceiling import rendered_screen

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()

class Provider(BaseHTTPRequestHandler):
    failures = []
    requests = []
    def log_message(self, *_): pass
    def do_GET(self):
        payload = b'{"data":[{"id":"policy-fixture"}]}'
        self.send_response(200); self.send_header('Content-Length', str(len(payload))); self.end_headers(); self.wfile.write(payload)
    def do_POST(self):
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            self.requests.append(body)
            assert self.headers['Authorization'] == 'Bearer policy-key-canary'
            messages = body['messages']
            last = max(i for i, item in enumerate(messages) if item['role'] == 'user')
            prompt = messages[last]['content']
            tool = next((item for item in reversed(messages[last + 1:]) if item['role'] == 'tool'), None)
            if tool is not None or prompt not in ['effect denied', 'effect allowed', 'terminal start', 'terminal close']:
                delta = {'content': 'policy-fixture-finished ' + prompt}
            else:
                if prompt.startswith('effect '): name, args = 'shell', {'command': 'touch ' + prompt.replace(' ', '-')}
                elif prompt == 'terminal start': name, args = 'process', {'action': 'start', 'name': 'handoff-terminal', 'command': 'sleep 30'}
                else: name, args = 'process', {'action': 'terminate', 'current_name': 'handoff-terminal'}
                delta = {'tool_calls': [{'index': 0, 'id': 'policy-tool', 'type': 'function', 'function': {'name': name, 'arguments': json.dumps(args)}}]}
            payload = ('data: ' + json.dumps({'choices': [{'delta': delta, 'finish_reason': 'stop'}]}) + '\n\ndata: [DONE]\n\n').encode()
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.send_header('Content-Length', str(len(payload))); self.end_headers(); self.wfile.write(payload)
        except Exception as error:
            self.failures.append(repr(error)); self.send_error(500)

class Case:
    def __init__(self, root, port, access='unrestricted', mcp=False):
        self.root = root; root.mkdir()
        self.rows, self.columns = 35, 140
        self.env = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'), POLICY_FIXTURE_KEY='policy-key-canary', TERM='xterm-256color')
        self.profiles = root / 'config/helm/profiles'
        self.config = root / 'config.toml'
        text = f'provider="openai-chat"\nmodel="policy-fixture"\nbase_url="http://127.0.0.1:{port}/v1"\napi_key_env="POLICY_FIXTURE_KEY"\naccess="{access}"\nprovider_retry_attempts=1\n[env]\nSECRET="policy-secret-canary"\n'
        if mcp:
            script = root / 'mcp.py'
            script.write_text('''import json,subprocess,sys
child=subprocess.Popen(['sleep','30'])
open(sys.argv[1],'w').write(str(child.pid))
for line in sys.stdin:
 request=json.loads(line)
 if 'id' not in request: continue
 result={'tools':[]} if request['method']=='tools/list' else {'protocolVersion':'2025-06-18','capabilities':{},'serverInfo':{'name':'fixture','version':'1'}}
 print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
 if request['method']=='tools/list': break
''')
            import sys
            text += '\n[mcp_servers.fixture]\ncommand=' + json.dumps(sys.executable) + '\nargs=' + json.dumps([str(script), str(root / 'mcp-pid')]) + '\n'
        self.config.write_text(text)
        self.common = [str(HELM), '--config', str(self.config), '--workspace', str(root)]
        self.admin('policy', 'list')
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', self.rows, self.columns, 0, 0))
        self.previous = termios.tcgetattr(self.slave)
        self.output = bytearray()
        self.process = subprocess.Popen([*self.common, 'chat'], stdin=self.slave, stdout=self.slave, stderr=self.slave, env=self.env, cwd=root, start_new_session=True)
        self.text('HELM')
    def admin(self, *args):
        result = subprocess.run([*self.common, *args], env=self.env, cwd=self.root, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, (args, result.stdout, result.stderr)
        assert 'policy-key-canary' not in result.stdout + result.stderr
        assert 'policy-secret-canary' not in result.stdout + result.stderr
        return json.loads(result.stdout)
    def screen(self): return rendered_screen(bytes(self.output), self.rows, self.columns)
    def drain(self):
        if select.select([self.master], [], [], .05)[0]:
            try: self.output.extend(os.read(self.master, 65536))
            except OSError: pass
    def wait(self, condition, label, seconds=15):
        deadline = time.monotonic() + seconds
        while not condition():
            assert time.monotonic() < deadline, (label, self.process.poll(), self.screen(), Provider.failures)
            self.drain()
    def text(self, text): self.wait(lambda: text in self.screen(), text)
    def send(self, data): os.write(self.master, data.encode() if isinstance(data, str) else data)
    def pick(self, end=True):
        self.send(b'\x10'); self.text('Use launch defaults'); self.text('restricted · revision')
        self.send(b'\x1b[F' if end else b'\x1b[H\x1b[B')
        self.send(b'\r'); self.text('PROPOSED EFFECTIVE POLICY')
    def apply(self, key=b'\r'):
        self.send(key); self.text('Policy applied to this workspace runtime')
    def prompt(self, prompt):
        self.send(prompt + '\r'); self.text('policy-fixture-finished ' + prompt); self.text('Completed ·')
        self.wait(lambda: any(item.get('phase') == 'completed' for item in self.saved().get('run_summaries', [])), 'canonical completion')
    def saved(self):
        files = list((self.root / 'data/helm/sessions').glob('*.json'))
        return json.loads(files[0].read_text()) if files else {}
    def finish(self):
        self.send(b'\x11'); self.wait(lambda: self.process.poll() is not None, 'exit')
        assert self.process.returncode == 0, self.screen()
        assert termios.tcgetattr(self.slave) == self.previous, 'complete prior terminal mode was not restored'
        assert b'policy-key-canary' not in self.output and b'policy-secret-canary' not in self.output
    def close(self):
        if self.process.poll() is None: os.killpg(self.process.pid, signal.SIGKILL); self.process.wait(5)
        os.close(self.master); os.close(self.slave)

def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-tui-policy-') as directory:
            root = Path(directory)
            case = Case(root / 'switch', server.server_port)
            try:
                # Known configured secrets remain redacted even if untrusted
                # profile metadata deliberately repeats one in its display name.
                created = subprocess.run([*case.common, 'policy', 'create', 'policy-secret-canary', '--preset', 'restricted'], env=case.env, cwd=case.root, capture_output=True, timeout=15)
                assert created.returncode == 0
                case.send('effect denied'); case.pick(); case.apply()
                assert case.saved()['draft'] == 'effect denied'
                case.send(b'\r'); case.text('policy-fixture-finished effect denied'); case.text('Completed ·')
                assert not (case.root / 'effect-denied').exists()
                case.pick(end=False); case.text('AUTHORITY INCREASE')
                before = case.saved()['messages']
                case.send(b'\r\x1b[200~y\x1b[201~'); case.drain()
                assert case.saved()['messages'] == before and 'AUTHORITY INCREASE' in case.screen()
                case.send(b'\x1b'); case.text('Use launch defaults')
                case.send(b'\r'); case.text('AUTHORITY INCREASE'); case.apply(b'y')
                case.prompt('effect allowed'); assert (case.root / 'effect-allowed').exists()
                case.send(b'\x10'); case.text('restricted · revision')
                case.send(b'\x1b[H\r'); case.text('Selected: launch defaults for this workspace')
                case.text('PROPOSED EFFECTIVE POLICY')
                case.apply(b'y' if 'AUTHORITY INCREASE' in case.screen() else b'\r')
                assert not any('policy_profile' in key for key in case.saved())
                case.finish()
                # Reopen saved voyage with ordinary launch policy; selection is not authority on disk.
                saved = case.saved()
                result = subprocess.run([*case.common, 'run', 'ordinary restart', '--resume', saved['id']], env=case.env, cwd=case.root, capture_output=True, text=True, timeout=15)
                assert result.returncode == 0, result.stderr
            finally: case.close()
            case = Case(root / 'stale', server.server_port, access='read-only')
            try:
                case.admin('policy', 'create', 'z-custom', '--preset', 'autonomous')
                case.pick(); case.text('AUTHORITY INCREASE')
                case.admin('policy', 'delete', 'z-custom', '--expected-revision', '1')
                case.send('y'); case.text('Policy changed or confirmation invalid')
                assert not case.saved()['messages']
                case.finish()
            finally: case.close()
            case = Case(root / 'terminal', server.server_port)
            try:
                case.prompt('terminal start')
                case.pick(); case.send(b'\r'); case.text('close terminals and finish background shell work first')
                case.prompt('terminal close')
                case.pick(); case.apply(); case.finish()
            finally: case.close()
            case = Case(root / 'mcp', server.server_port, mcp=True)
            try:
                pid = int((case.root / 'mcp-pid').read_text())
                case.pick(); case.apply()
                stat = Path(f'/proc/{pid}/stat')
                assert not stat.exists() or stat.read_text().rsplit(') ', 1)[1].startswith('Z'), 'forked MCP descendant survived authority handoff'
                case.finish()
            finally: case.close()
        assert not Provider.failures, Provider.failures
        print('TUI policy switching: keys, consent, denial, drift, terminal refusal/recovery, MCP descendants and restart passed')
    finally: server.shutdown(); server.server_close()

if __name__ == '__main__': main()
