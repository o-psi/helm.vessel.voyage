#!/usr/bin/env python3
"""Real CLI + EnrollmentApi fixture; synthetic secrets, no external network."""
import http.server
import json
import os
import pathlib
import signal
import socket
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
SUFFIX = '.exe' if os.name == 'nt' else ''
HELM = pathlib.Path(os.environ.get('HELM_BIN', ROOT / ('target/release/helm' + SUFFIX))).resolve()
VESSEL = pathlib.Path(os.environ.get('VESSEL_BIN', ROOT / ('target/release/vessel' + SUFFIX))).resolve()
TOKEN = 'fixture-only-owner-token-at-least-32-bytes'
CANARY = 'FixtureInvitationSecretNeverEchoed_________'  # 43 base64url bytes
assert len(CANARY) == 43

class Proxy(http.server.BaseHTTPRequestHandler):
    backend = ''
    drop_next = False
    redirect_next = False
    stall_next = False
    stalled = threading.Event()
    release = threading.Event()
    def log_message(self, *args):
        pass
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('content-length', '0')))
        if self.path.endswith('/challenge') and Proxy.redirect_next:
            Proxy.redirect_next = False
            self.send_response(302)
            self.send_header('Location', 'http://credential:' + CANARY + '@127.0.0.1:1/forbidden')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        headers = {'Content-Type': 'application/json', 'x-voyage-request': '2'}
        if 'Authorization' in self.headers:
            headers['Authorization'] = self.headers['Authorization']
        req = urllib.request.Request(Proxy.backend + self.path, data=body, headers=headers)
        try:
            response = urllib.request.urlopen(req, timeout=5)
        except urllib.error.HTTPError as error:
            response = error
        payload = response.read()
        if self.path.endswith('/complete') and response.status == 200:
            if Proxy.drop_next:
                Proxy.drop_next = False
                self.close_connection = True
                self.connection.shutdown(socket.SHUT_RDWR)
                return
            if Proxy.stall_next:
                Proxy.stall_next = False
                Proxy.stalled.set()
                Proxy.release.wait(5)
        try:
            self.send_response(response.status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass

def post(origin, path, body):
    request = urllib.request.Request(origin + path, data=json.dumps(body).encode(), headers={
        'Content-Type': 'application/json', 'x-voyage-request': '2', 'Authorization': 'Bearer ' + TOKEN})
    return json.load(urllib.request.urlopen(request, timeout=5))

def main():
    with tempfile.TemporaryDirectory(prefix='helm-enrollment-cli-') as temp:
        root = pathlib.Path(temp)
        env = dict(os.environ, RUST_LOG='trace', XDG_DATA_HOME=str(root / 'data'), HOME=str(root / 'home'))
        directory = root / 'identity'
        known_secrets = [TOKEN, CANARY]
        def cli(*args, key=None, expected=0, target=directory, extra=()):
            command = [str(HELM), 'attachment', '--directory', str(target), *extra, *args]
            result = subprocess.run(command, input=key, text=True, capture_output=True, env=env, timeout=20)
            for secret in known_secrets:
                assert secret not in result.stdout + result.stderr, 'secret escaped CLI output'
            assert result.returncode == expected, (args, result.returncode, result.stdout, result.stderr)
            return json.loads(result.stdout) if result.stdout.strip() else None
        assert cli('status') == {'status': 'unenrolled'}
        assert not directory.exists()
        assert not (root / 'data').exists()
        unrelated = subprocess.run([str(HELM), '--config', str(root/'missing-provider-config'), '--set', 'provider=not-a-provider',
            'attachment', '--directory', str(directory), 'status'], capture_output=True, text=True, env=env, timeout=3)
        assert unrelated.returncode == 0 and json.loads(unrelated.stdout) == {'status': 'unenrolled'}
        # Invalid arguments and invalid input must not echo even an accidental argv secret.
        cli('enroll', CANARY, expected=2)
        cli('enroll', '--invitation-id', str(uuid.uuid4()), '--invitation-key-stdin', key=CANARY + 'x', expected=1,
            extra=('--origin', 'https://example.com'))
        assert not directory.exists()
        for bad in ['', CANARY+'\n\n', 'x'*100000]:
            cli('enroll', '--invitation-id', str(uuid.uuid4()), '--invitation-key-stdin', key=bad, expected=1,
                extra=('--origin', 'https://example.com'))
            assert not directory.exists()
        if os.name != 'nt':
            import pty
            master, slave = pty.openpty()
            try:
                terminal = subprocess.run([str(HELM), 'attachment', '--directory', str(directory), '--origin', 'https://example.com',
                    'enroll', '--invitation-id', str(uuid.uuid4()), '--invitation-key-stdin'], stdin=slave,
                    capture_output=True, text=True, env=env, timeout=3)
                assert terminal.returncode == 1 and not directory.exists()
            finally:
                os.close(master); os.close(slave)
            waiting = subprocess.Popen([str(HELM), 'attachment', '--directory', str(directory), '--origin', 'https://example.com',
                'enroll', '--invitation-id', str(uuid.uuid4()), '--invitation-key-stdin'], stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True)
            time.sleep(.15)
            waiting.send_signal(signal.SIGINT)
            waiting.wait(timeout=3)  # Keep stdin open until cancellation wins; communicate closes it.
            out, err = waiting.communicate(timeout=3)
            assert waiting.returncode == 130 and CANARY not in out+err and not directory.exists()
        help_result = subprocess.run([str(HELM), 'attachment', '--help'], capture_output=True, text=True)
        for command in ['enroll', 'status', 'resume', 'rotate', 'revoke', 'detach']:
            assert command in help_result.stdout
        proxy = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        origin = 'http://127.0.0.1:' + str(proxy.server_port)
        reserve = socket.socket(); reserve.bind(('127.0.0.1', 0)); port = reserve.getsockname()[1]; reserve.close()
        Proxy.backend = 'http://127.0.0.1:' + str(port)
        server = subprocess.Popen([str(VESSEL), '--bind', '127.0.0.1:' + str(port), '--public-origin', origin,
            '--allow-insecure-loopback', '--attachment-directory', str(root / 'authority'), '--database', str(root / 'vessel.db')],
            env=dict(env, VESSEL_OPERATOR_TOKEN=TOKEN, RUST_LOG='error'), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        thread = threading.Thread(target=proxy.serve_forever, daemon=True); thread.start()
        try:
            deadline = time.monotonic() + 10
            while True:
                assert server.poll() is None, 'Vessel exited before readiness'
                try:
                    urllib.request.urlopen(Proxy.backend + '/ready', timeout=.2).close()
                    break
                except (OSError, urllib.error.URLError):
                    assert time.monotonic() < deadline, 'Vessel readiness timed out'
                    time.sleep(.03)
            def invitation():
                value = post(origin, '/v2/enrollment/invitations', {'ttl_ms': 60000})
                known_secrets.append(value['key'])
                return value
            flags = ('--origin', origin, '--allow-insecure-loopback')
            def enroll(inv, target=directory, expected=0):
                return cli('enroll', '--invitation-id', inv['id'], '--invitation-key-stdin', key=inv['key']+'\n', extra=flags, target=target, expected=expected)
            inv = invitation()
            Proxy.drop_next = True
            enroll(inv, expected=1)
            state_path = directory / 'client.json'
            pending_bytes = state_path.read_bytes()
            pending = json.loads(pending_bytes)
            assert pending['status'] == 'enrolling'
            before_stat = state_path.stat()
            info = cli('status')
            after_stat = state_path.stat()
            assert (before_stat.st_mtime_ns, before_stat.st_ino) == (after_stat.st_mtime_ns, after_stat.st_ino)
            assert info['transaction_id'] == pending['pending']['operation']['transaction_id']
            assert state_path.read_bytes() == pending_bytes
            assert 'private_key' not in info and 'invitation_key' not in info
            cli('resume', expected=1, extra=flags)
            active = cli('resume', '--invitation-key-stdin', key=inv['key'], extra=flags)
            assert active['status'] == 'active' and active['epoch'] == 1
            before = state_path.read_bytes()
            cli('status', expected=1, extra=('--origin', 'https://different.example'))
            cli('status', expected=1, extra=('--origin', 'https://user:' + CANARY + '@example.com'))
            assert state_path.read_bytes() == before
            Proxy.drop_next = True
            cli('rotate', expected=1, extra=flags)
            rotating = json.loads(state_path.read_bytes())
            assert rotating['status'] == 'rotating' and rotating['pending']['new_private_key']
            transaction = rotating['pending']['operation']['transaction_id']
            assert cli('status')['transaction_id'] == transaction
            rotated = cli('resume', extra=flags)
            assert rotated['status'] == 'active' and rotated['epoch'] == 2
            assert json.loads(state_path.read_bytes())['private_key'] == rotating['pending']['new_private_key']
            Proxy.drop_next = True
            cli('revoke', expected=1, extra=flags)
            assert cli('status')['status'] == 'revoking'
            assert cli('resume', extra=flags)['status'] == 'revoked'
            revoked_bytes = state_path.read_bytes()
            revoked_stat = state_path.stat()
            redundant = cli('detach')
            assert redundant['status'] == 'revoked' and 'already confirmed' in redundant['notice']
            assert state_path.read_bytes() == revoked_bytes
            assert (state_path.stat().st_mtime_ns, state_path.stat().st_ino) == (revoked_stat.st_mtime_ns, revoked_stat.st_ino)
            assert cli('status')['status'] == 'revoked'
            # Offline disable preserves every uncertain transaction across real
            # server-side commit/lost response and explicit recovery.
            for operation in ['enroll', 'rotate', 'revoke']:
                disabled_dir = root / ('disabled-' + operation)
                disabled_inv = invitation()
                if operation == 'enroll':
                    Proxy.drop_next = True
                    enroll(disabled_inv, target=disabled_dir, expected=1)
                else:
                    enroll(disabled_inv, target=disabled_dir)
                    Proxy.drop_next = True
                    cli(operation, target=disabled_dir, extra=flags, expected=1)
                disabled_path = disabled_dir / 'client.json'
                original = json.loads(disabled_path.read_bytes())
                info = cli('detach', target=disabled_dir)
                assert info['locally_disabled'] and info['pending'] == operation
                retained = json.loads(disabled_path.read_bytes())
                assert retained['pending'] == original['pending']
                assert retained['private_key'] == original['private_key']
                saved = disabled_path.read_bytes()
                cli('detach', target=disabled_dir)
                assert disabled_path.read_bytes() == saved
                if operation == 'enroll':
                    recovered = cli('resume', '--invitation-key-stdin', key=disabled_inv['key'], target=disabled_dir, extra=flags)
                else:
                    recovered = cli('resume', target=disabled_dir, extra=flags)
                assert recovered['locally_disabled']
                assert recovered['status'] == ('revoked' if operation == 'revoke' else 'detached')
                if operation != 'revoke':
                    cli('rotate', target=disabled_dir, extra=flags, expected=1)
                    assert cli('revoke', target=disabled_dir, extra=flags)['status'] == 'revoked'
            # Denials and redirects preserve the original pending identity.
            denied_dir = root / 'denied'
            wrong = invitation()
            cli('enroll', '--invitation-id', wrong['id'], '--invitation-key-stdin', key=CANARY, target=denied_dir, extra=flags, expected=1)
            denied = (denied_dir/'client.json').read_bytes()
            cli('resume', '--invitation-key-stdin', key=CANARY, target=denied_dir, extra=flags, expected=1)
            assert (denied_dir/'client.json').read_bytes() == denied
            redirect_dir = root / 'redirect'
            redirected = invitation(); Proxy.redirect_next = True
            enroll(redirected, target=redirect_dir, expected=1)
            assert cli('resume', '--invitation-key-stdin', key=redirected['key'], target=redirect_dir, extra=flags)['status'] == 'active'
            # Interrupt after server commit, preserving original pending operation.
            if os.name != 'nt':
                Proxy.stall_next = True
                proc = subprocess.Popen([str(HELM), 'attachment', '--directory', str(redirect_dir), *flags, 'rotate'], env=env,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                assert Proxy.stalled.wait(5)
                proc.send_signal(signal.SIGINT)
                out, err = proc.communicate(timeout=3)
                assert proc.returncode == 130 and all(secret not in out+err for secret in known_secrets)
                Proxy.release.set()
                assert cli('status', target=redirect_dir)['status'] == 'rotating'
                assert cli('resume', target=redirect_dir, extra=flags)['status'] == 'active'
            # Offline detach has no network dependency and preserves all sessions.
            sentinel = root/'sessions'; sentinel.mkdir(); (sentinel/'keep.json').write_text('user-session')
            server.terminate(); server.wait(timeout=5)
            detached = cli('detach', target=redirect_dir)
            assert detached['status'] == 'detached' and 'not confirmed' in detached['notice']
            assert (sentinel/'keep.json').read_text() == 'user-session'
            cli('status', target=redirect_dir)
            assert not (root/'data').exists(), 'explicit enrollment polluted runtime data'
        finally:
            Proxy.release.set()
            if server.poll() is None:
                server.terminate(); server.wait(timeout=5)
            proxy.shutdown(); proxy.server_close(); thread.join(timeout=2)
    print('attachment enrollment CLI: lifecycle, recovery, privacy, bounds and offline detach passed')

if __name__ == '__main__':
    main()
