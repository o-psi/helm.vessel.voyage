#!/usr/bin/env python3
"""Local TLS fixture for the Linux Rust redirect regression; synthetic secrets only."""
import json
from pathlib import Path
import ssl
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

root = Path(sys.argv[1])
subprocess.run([
    'openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
    '-keyout', str(root / 'key.pem'), '-out', str(root / 'cert.pem'),
    '-days', '1', '-subj', '/CN=localhost',
    '-addext', 'subjectAltName=IP:127.0.0.1',
    '-addext', 'basicConstraints=critical,CA:FALSE',
], check=True, timeout=10, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.server.secure:
            assert self.headers['x-api-key'] == 'synthetic-redirect-key-canary'
            with (root / 'source-requests').open('a') as output:
                output.write(self.path + '\n')
            self.send_response(307)
            self.send_header('Location', f'http://127.0.0.1:{plain.server_port}/stolen')
        else:
            (root / 'destination-contacted').write_text('yes')
            self.send_response(200)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_POST(self):
        self.rfile.read(int(self.headers['Content-Length']))
        self.do_GET()

plain = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
plain.secure = False
secure = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
secure.secure = True
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(root / 'cert.pem', root / 'key.pem')
secure.socket = context.wrap_socket(secure.socket, server_side=True)
threading.Thread(target=plain.serve_forever, daemon=True).start()
threading.Thread(target=secure.serve_forever, daemon=True).start()
print(json.dumps({'url': f'https://127.0.0.1:{secure.server_port}'}), flush=True)
sys.stdin.read()
secure.shutdown()
plain.shutdown()
