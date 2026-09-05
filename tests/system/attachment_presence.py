#!/usr/bin/env python3
"""Actual foreground Helm/Vessel presence, isolated credentials, no provider/network service."""
import json
import os
import pathlib
import select
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[2]
HELM = pathlib.Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()
VESSEL = pathlib.Path(os.environ.get('VESSEL_BIN', ROOT / 'target/release/vessel')).resolve()
TOKEN = 'presence-fixture-operator-token-at-least-32-bytes'


def main():
    with tempfile.TemporaryDirectory(prefix='voyage-presence-') as temporary:
        root = pathlib.Path(temporary)
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_CONFIG_HOME=str(root / 'config'), VESSEL_OPERATOR_TOKEN=TOKEN, RUST_LOG='trace')
        invalid_config = root / 'invalid-provider.toml'
        invalid_config.write_text('not valid TOML [ deliberately')
        with socket.socket() as allocated:
            allocated.bind(('127.0.0.1', 0))
            address = '127.0.0.1:' + str(allocated.getsockname()[1])
        origin = 'http://' + address
        directory = root / 'identity'
        secrets = [TOKEN]
        children = []
        server_log = tempfile.TemporaryFile()
        server = None

        def request(path, body=None, authenticated=True, expected=200, headers=None):
            fields = dict(headers or {})
            if authenticated:
                fields['Authorization'] = 'Bearer ' + TOKEN
            if body is not None:
                fields.update({'Content-Type': 'application/json', 'x-voyage-request': '2'})
            req = urllib.request.Request(origin + path, headers=fields,
                data=None if body is None else json.dumps(body).encode())
            try:
                response = urllib.request.urlopen(req, timeout=2)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                payload = response.read().decode()
                assert response.status == expected, (path, response.status, expected)
            for secret in secrets:
                assert secret not in payload, 'secret escaped HTTP response'
            return payload

        def upgrade_status(protocol):
            # urllib forces Connection: close, so use a real HTTP upgrade here.
            with socket.create_connection(('127.0.0.1', int(address.rsplit(':', 1)[1])), timeout=2) as peer:
                peer.sendall((f'GET /v2/attachment HTTP/1.1\r\nHost: {address}\r\n'
                    f'Origin: {origin}\r\nx-voyage-request: 2\r\n'
                    'Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n'
                    'Sec-WebSocket-Key: MDEyMzQ1Njc4OWFiY2RlZg==\r\n'
                    f'Sec-WebSocket-Protocol: {protocol}\r\n\r\n').encode())
                response = b''
                while b'\r\n\r\n' not in response:
                    chunk = peer.recv(4096)
                    assert chunk and len(response) < 16384, 'invalid upgrade response'
                    response += chunk
                return int(response.split(b' ', 2)[1])

        def diagnostic():
            return json.loads(request('/v1/diagnostics'))

        def wait_count(count):
            deadline = time.monotonic() + 5
            while True:
                records = diagnostic()['connections']
                if len(records) == count:
                    return records
                assert time.monotonic() < deadline, 'connection observation timed out'
                time.sleep(.03)

        def start_server(enabled=True):
            command = [str(VESSEL), '--bind', address, '--database', str(root / 'vessel.db')]
            if enabled:
                command += ['--attachment-directory', str(root / 'authority'),
                            '--public-origin', origin, '--allow-insecure-loopback']
            process = subprocess.Popen(command, env=env, stdout=server_log, stderr=server_log)
            children.append(process)
            deadline = time.monotonic() + 10
            while True:
                assert process.poll() is None, 'Vessel exited during startup'
                try:
                    request('/ready', authenticated=False)
                    return process
                except (OSError, urllib.error.URLError):
                    assert time.monotonic() < deadline, 'Vessel startup timed out'
                    time.sleep(.03)

        def command(*args, target=directory):
            return [str(HELM), '--config', str(invalid_config), '--set', 'provider=not-a-provider',
                    'attachment', '--directory', str(target), '--allow-insecure-loopback', *args]

        def cli(*args, key=None, expected=0, target=directory):
            result = subprocess.run(command(*args, target=target), input=key, capture_output=True,
                                    text=True, env=env, timeout=10)
            for secret in secrets:
                assert secret not in result.stdout + result.stderr, 'secret escaped Helm output'
            assert result.returncode == expected, (args, result.returncode, expected)
            return json.loads(result.stdout) if result.stdout else None

        def enroll(target=directory):
            # The invitation intentionally contains a new synthetic credential.
            req = urllib.request.Request(origin + '/v2/enrollment/invitations',
                data=b'{"ttl_ms":60000}', headers={'Authorization': 'Bearer ' + TOKEN,
                    'Content-Type': 'application/json', 'x-voyage-request': '2'})
            with urllib.request.urlopen(req, timeout=2) as response:
                invitation = json.load(response)
            secrets.append(invitation['key'])
            return cli('--origin', origin, 'enroll', '--invitation-id', invitation['id'],
                       '--invitation-key-stdin', key=invitation['key'] + '\n', target=target)

        def connected():
            process = subprocess.Popen(command('connect'), stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
            children.append(process)
            assert select.select([process.stdout], [], [], 8)[0], 'connected notice timed out'
            line = process.stdout.readline()
            assert line, 'connection ended before notice'
            for secret in secrets:
                assert secret not in line, 'secret escaped connection notice'
            notice = json.loads(line)
            assert notice['event'] == 'attachment_connected' and notice['mode'] == 'presence_only'
            assert set(notice) == {'event', 'mode', 'machine_id', 'connection_id'}
            records = wait_count(1)
            assert records[0]['machine_id'] == notice['machine_id']
            assert records[0]['connection_id'] == notice['connection_id']
            return process, notice

        def finish(process, expected, sent_signal=None):
            if sent_signal is not None:
                process.send_signal(sent_signal)
            stdout, stderr = process.communicate(timeout=7)
            assert process.returncode == expected, (process.returncode, expected)
            for secret in secrets:
                assert secret not in (stdout or '') + (stderr or ''), 'secret escaped final output'
            return stderr or ''

        try:
            server = start_server(False)
            request('/v2/attachment', authenticated=False, expected=404)
            assert diagnostic()['attachment'] == 'disabled'
            assert diagnostic()['connections'] == []
            assert 'voyage_connectivity_enabled 0' in request('/metrics', authenticated=False)
            server.send_signal(signal.SIGTERM)
            server.wait(timeout=5)
            assert server.returncode == 0
            assert not (root / 'authority').exists()

            server = start_server()
            for route in ['/ui', '/v1/diagnostics']:
                request(route, authenticated=False, expected=401)
            assert diagnostic()['attachment'] == 'presence_only'
            assert diagnostic()['remote_execution'] == 'unavailable'
            assert 'voyage_connectivity_enabled 1' in request('/metrics', authenticated=False)
            assert upgrade_status('wrong-protocol') == 403
            info = enroll()
            assert info['status'] == 'active'
            original_state = (directory / 'client.json').read_bytes()
            process, first = connected()
            assert first['machine_id'] == info['machine_id']
            # A second process cannot borrow the exclusive enrollment identity.
            cli('connect', expected=1)
            assert wait_count(1)[0]['connection_id'] == first['connection_id']
            # Remain connected beyond the initial 10s lease through fresh heartbeats.
            time.sleep(11)
            assert process.poll() is None
            assert wait_count(1)[0]['connection_id'] == first['connection_id']
            # There must be no inbound Helm task listener, even on an ephemeral port.
            inodes = set()
            for fd in pathlib.Path(f'/proc/{process.pid}/fd').iterdir():
                try:
                    target = os.readlink(fd)
                except FileNotFoundError:
                    continue
                if target.startswith('socket:['):
                    inodes.add(target[8:-1])
            for table in ['/proc/net/tcp', '/proc/net/tcp6']:
                for row in pathlib.Path(table).read_text().splitlines()[1:]:
                    fields = row.split()
                    assert not (fields[3] == '0A' and fields[9] in inodes), 'Helm opened an inbound listener'
            stderr = finish(process, 130, signal.SIGINT)
            assert 'enrollment unchanged' in stderr
            wait_count(0)
            assert (directory / 'client.json').read_bytes() == original_state
            assert cli('status')['status'] == 'active'

            # A blocked initial stdout cannot retain the socket/enrollment lease forever.
            read_fd, write_fd = os.pipe()
            try:
                os.set_blocking(write_fd, False)
                while True:
                    try:
                        os.write(write_fd, b'x' * 4096)
                    except BlockingIOError:
                        break
                os.set_blocking(write_fd, True)
                blocked = subprocess.Popen(command('connect'), stdin=subprocess.DEVNULL,
                    stdout=write_fd, stderr=subprocess.PIPE, text=True, env=env)
                children.append(blocked)
                os.close(write_fd)
                write_fd = None
                finish(blocked, 1)
                wait_count(0)
            finally:
                os.close(read_fd)
                if write_fd is not None:
                    os.close(write_fd)
            # A broken output pipe is also an explicit failure, not a live orphan.
            read_fd, write_fd = os.pipe()
            os.close(read_fd)
            try:
                broken = subprocess.Popen(command('connect'), stdin=subprocess.DEVNULL,
                    stdout=write_fd, stderr=subprocess.PIPE, text=True, env=env)
                children.append(broken)
            finally:
                os.close(write_fd)
            finish(broken, 1)
            wait_count(0)
            process, second = connected()
            assert second['connection_id'] != first['connection_id']
            server.send_signal(signal.SIGTERM)
            server.wait(timeout=5)
            assert server.returncode == 0
            finish(process, 1)
            assert (directory / 'client.json').read_bytes() == original_state

            # Cancellation during a stalled pre-authentication HTTP request must
            # release the identity too; no Welcome or connected notice is possible.
            with socket.socket() as stalled:
                stalled.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                stalled.bind(('127.0.0.1', int(address.rsplit(':', 1)[1])))
                stalled.listen(1)
                stalled.settimeout(5)
                early = subprocess.Popen(command('connect'), stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
                children.append(early)
                peer, _ = stalled.accept()
                with peer:
                    peer.settimeout(5)
                    assert peer.recv(8192).startswith(b'POST /v2/enrollment/challenge ')
                    finish(early, 130, signal.SIGTERM)
                    try:
                        assert peer.recv(8192) == b'', 'cancelled handshake remained open'
                    except ConnectionResetError:
                        pass
            assert cli('status')['status'] == 'active'
            assert (directory / 'client.json').read_bytes() == original_state

            # Restart retains enrollment but requires a new proof and connection generation.
            server = start_server()
            assert wait_count(0) == []
            process, third = connected()
            assert third['connection_id'] not in {first['connection_id'], second['connection_id']}
            revoked = json.loads(request('/v2/enrollment/revoke', {
                'machine_id': info['machine_id'], 'expected_epoch': info['epoch'],
                'transaction_id': str(uuid.uuid4())}))
            assert revoked['revoked']
            finish(process, 1)
            wait_count(0)
            cli('connect', expected=1)
            assert (directory / 'client.json').read_bytes() == original_state

            # Offline detach prevents a fresh attachment without contacting any provider.
            other = root / 'detached-identity'
            enroll(other)
            assert cli('detach', target=other)['status'] == 'detached'
            cli('connect', target=other, expected=1)
            assert not (root / 'data').exists(), 'presence loaded provider/session runtime state'
            server.send_signal(signal.SIGTERM)
            server.wait(timeout=5)
            assert server.returncode == 0
            server_log.seek(0)
            log = server_log.read().decode(errors='replace')
            for secret in secrets:
                assert secret not in log, 'secret escaped Vessel logs'
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.kill()
                child.wait(timeout=5)
            server_log.close()
    print('attachment presence: real enrollment, leases, restart, cancellation, output and revocation passed')


if __name__ == '__main__':
    main()
