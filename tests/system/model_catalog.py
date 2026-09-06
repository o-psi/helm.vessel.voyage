#!/usr/bin/env python3
"""Real native catalog CLI/plain-output regression with synthetic loopback metadata."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BIN = str(Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve())
KEY = 'synthetic-model-catalog-credential'
CONFIG_SECRETS = ['environment-秘密-token', 'mcp-credential-canary', 'redact-\"quoted\"-value']

class Handler(BaseHTTPRequestHandler):
    value = {}
    code = 200
    requests = []
    def log_message(self, *_):
        pass
    def do_GET(self):
        self.requests.append(self.path)
        assert self.headers.get('x-api-key') == KEY
        value = self.value
        if value == "timeout":
            time.sleep(3)
            value = {"data": []}
        if value == 'pages':
            value = ({'data': [{'id': 'model-second', 'display_name': '日本語 🚢'}], 'has_more': False}
                     if 'after_id=' in self.path else
                     {'data': [{'id': 'model-first', 'display_name': 'Français'}], 'has_more': True, 'last_id': 'cursor-first'})
        body = value if isinstance(value, bytes) else json.dumps(value).encode()
        self.send_response(self.code)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

def main():
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root/'config'),
                   XDG_DATA_HOME=str(root/'data'), ANTHROPIC_API_KEY=KEY)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        config = root/'helm.toml'
        def configure(model='manual-current'):
            config.write_text(f'provider = "anthropic"\nmodel = {json.dumps(model)}\nbase_url = "http://127.0.0.1:{server.server_port}/v1"\ncommand_timeout_secs = 2\n'
                              f'redact_values = [{json.dumps(CONFIG_SECRETS[2])}]\n'
                              f'[env]\nSERVICE_TOKEN = {json.dumps(CONFIG_SECRETS[0])}\n'
                              f'[mcp_servers.fixture]\ncommand = "not-a-real-mcp-server"\n'
                              f'[mcp_servers.fixture.env]\nTOKEN = {json.dumps(CONFIG_SECRETS[1])}\n')
        configure()
        original = config.read_bytes()
        def run(*args, ok=True, input=None):
            process = subprocess.run([BIN, '--config', str(config), *args], cwd=root,
                                     env=env, input=input, capture_output=True, text=True, timeout=10)
            assert (process.returncode == 0) == ok, (args, process.returncode)
            for secret in [KEY, *CONFIG_SECRETS]:
                assert secret not in process.stdout+process.stderr, 'credential leaked from catalog'
                assert json.dumps(secret)[1:-1] not in process.stdout+process.stderr, 'escaped credential leaked from catalog'
            assert '\x1b' not in process.stdout+process.stderr, 'escape leaked from catalog'
            return process
        try:
            Handler.value = {'data': [{'id': KEY}]}
            for args in [('models',), ('models', '--json')]:
                run(*args, ok=False)
            for bad in ['bad\x1b]52;copy\x07', 'bad\x00', 'bad\x85', 'bad\u202e', 'bad\u2066', KEY]:
                for field in ['id', 'display_name']:
                    entry = {'id': 'valid', 'display_name': 'Valid'}
                    entry[field] = bad
                    Handler.value = {'data': [entry]}
                    for args in [('models',), ('models', '--json')]:
                        result = run(*args, ok=False)
                        assert bad not in result.stdout+result.stderr
                result = run('chat', '--plain', input='/models\n/exit\n')
                assert bad not in result.stdout+result.stderr
            for secret in CONFIG_SECRETS:
                Handler.value={'data':[{'id':'good', 'display_name':secret}]}
                run('models',ok=False)
                run('models','--json',ok=False)
                run('chat','--plain',input='/models\n/exit\n')
            Handler.value={'data':[]}
            for secret in [KEY,*CONFIG_SECRETS]:
                configure(secret)
                run('models',ok=False)
                run('models','--json',ok=False)
            configure()
            Handler.value={'data':[{'id':f'm{i}'} for i in range(1024)]}
            run('models','--json',ok=False)
            Handler.value={'data':[{'id':f'm{i}'} for i in range(1023)]}
            assert len(json.loads(run('models','--json').stdout))==1024
            for value in [b'invalid-'+KEY.encode(), {'data':[{'id':'x'*513}]},
                          {'data':[{'id':'good','display_name':'x'*513}]},
                          {'data':[{'id':f'm{i}'} for i in range(1025)]}, b'x'*(1024*1024+1)]:
                Handler.value=value
                run('models','--json',ok=False)
            for code in [401,403,429,500]:
                Handler.code=code
                Handler.value={'error':KEY}
                run('models',ok=False)
            Handler.code=200
            Handler.value='timeout'
            result=run('models',ok=False)
            assert 'timed out' in result.stderr
            Handler.value='pages'
            Handler.requests=[]
            models=json.loads(run('models','--json').stdout)
            assert {model['id'] for model in models}=={'model-first','model-second','manual-current'}
            assert next(m for m in models if m['id']=='model-second')['display_name']=='日本語 🚢'
            assert len(Handler.requests)==2 and 'after_id=cursor-first' in Handler.requests[1]
            result=run('models')
            assert '日本語 🚢' in result.stdout and 'manual-current' in result.stdout
            Handler.value={'data':[]}
            assert json.loads(run('models','--json').stdout)[0]['id']=='manual-current'
            assert config.read_bytes()==original
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)
    print('model catalog CLI/plain safety, pagination, Unicode and fallback: passed')

if __name__ == '__main__':
    main()
