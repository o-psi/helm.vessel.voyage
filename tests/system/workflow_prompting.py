#!/usr/bin/env python3
"""Actual attended workflow CLI/PTY inputs, hidden values and terminal recovery."""
import fcntl
import hashlib
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
from http.server import ThreadingHTTPServer

import workflow_secrets as private

HELM = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()
SECRET = 'synthetic-private-雪-🦀'
PUBLIC = 'public-雪-λ $(touch never-created)'
DOCUMENT = '''schema_version=1
id='collect-inputs'
version='1'
description='Attended input fixture'
prompt='Public {{text}} count {{count}} mode {{mode}} optional {{optional}} private {{token}}'
[parameters.count]
type='integer'
required=true
minimum=1
maximum=3
[parameters.mode]
type='string'
default='safe'
choices=['safe','fast']
[parameters.optional]
type='boolean'
[parameters.text]
type='string'
required=true
[parameters.token]
type='string'
secret=true
required=true
'''


class Terminal:
    def __init__(self, args, env, root, stalled=False, stdout_terminal=False):
        self.master, self.slave = pty.openpty()
        self.before = termios.tcgetattr(self.slave)
        self.before[3] &= ~termios.ECHOK
        termios.tcsetattr(self.slave, termios.TCSANOW, self.before)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 16, 40, 400, 320))
        self.output = bytearray()
        if stalled:
            termios.tcflow(self.slave, termios.TCOOFF)
        self.proc = subprocess.Popen([str(HELM), *args], stdin=self.slave, stdout=self.slave if stdout_terminal else subprocess.PIPE,
                                     stderr=self.slave, cwd=root, env=env)

    def drain(self):
        if select.select([self.master], [], [], .03)[0]:
            self.output.extend(os.read(self.master, 65536))

    def marker(self, marker, count=1, hidden=True):
        deadline = time.monotonic() + 10
        while self.output.count(marker.encode()) < count:
            self.drain()
            assert self.proc.poll() is None and time.monotonic() < deadline, ('missing marker', marker)
        if hidden:
            assert not termios.tcgetattr(self.slave)[3] & termios.ECHO
        else:
            assert termios.tcgetattr(self.slave) == self.before

    def send(self, data):
        os.write(self.master, data.encode() if isinstance(data, str) else data)

    def finish(self, expected=0, timeout=12):
        deadline = time.monotonic() + timeout
        while self.proc.poll() is None:
            self.drain()
            assert time.monotonic() < deadline, 'collector did not exit promptly'
        stdout = self.proc.communicate(timeout=2)[0] or b''
        while select.select([self.master], [], [], .03)[0]:
            self.output.extend(os.read(self.master, 65536))
        assert self.proc.returncode == expected, (self.proc.returncode, bytes(self.output).decode(errors='replace'))
        assert termios.tcgetattr(self.slave) == self.before, 'exact terminal mode was not restored'
        probe = termios.tcgetattr(self.slave)
        probe[3] &= ~termios.ICANON
        probe[6][termios.VMIN] = 0
        probe[6][termios.VTIME] = 0
        termios.tcsetattr(self.slave, termios.TCSANOW, probe)
        assert os.read(self.slave, 65536) == b'', 'input tail remained queued'
        termios.tcsetattr(self.slave, termios.TCSANOW, self.before)
        assert SECRET.encode() not in self.output + stdout, 'hidden input entered public output'
        return stdout

    def close(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        termios.tcflow(self.slave, termios.TCOON)
        termios.tcsetattr(self.slave, termios.TCSANOW, self.before)
        if self.proc.stdout is not None:
            self.proc.stdout.close()
        os.close(self.master)
        os.close(self.slave)


def main():
    for operation in ('preview', 'run'):
        result = subprocess.run([str(HELM), 'workflow', operation, '--help'], capture_output=True, text=True, timeout=10)
        assert result.returncode == 0
        assert '--prompt-missing' in result.stdout, operation
        assert '--input-timeout-seconds' in result.stdout, operation
    server = ThreadingHTTPServer(('127.0.0.1', 0), private.Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-workflow-prompt-') as raw:
            root = Path(raw)
            user = root / 'workflows'
            user.mkdir()
            document = user / 'collect-inputs.toml'
            document.write_text(DOCUMENT)
            (user / 'review-change.toml').write_text(private.DOCUMENT)
            config = root / 'fixture.toml'
            config.write_text(f'provider="openai-chat"\nmodel="workflow-fixture"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="FIXTURE_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\n')
            env = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'),
                       XDG_DATA_HOME=str(root / 'data'), FIXTURE_KEY='public-fixture-key', RUST_LOG='trace')
            common = ['--config', str(config), '--workspace', str(root), 'workflow', '--user-directory', str(user)]
            preview = [*common, '--json', 'preview', 'collect-inputs', '--prompt-missing']
            for args in ([*common, 'preview', 'collect-inputs'], preview,
                         [*preview, '--input', 'count=invalid'], [*preview, '--input', 'token=not-allowed'],
                         [*preview, '--input-timeout-seconds', '0'], [*preview, '--input-timeout-seconds', '301']):
                result = subprocess.run([str(HELM), *args], stdin=subprocess.DEVNULL, capture_output=True,
                                        env=env, cwd=root, timeout=4)
                assert result.returncode != 0
            assert not private.Provider.requests
            terminal = Terminal(preview, env, root)
            try:
                terminal.marker('count: ')
                terminal.send('9\r')
                terminal.marker('count: ', 2)
                terminal.send('2\r')
                terminal.marker('text: ')
                terminal.send(PUBLIC + 'x\x7f\r')
                output = json.loads(terminal.finish())
                assert output['workflow']['inputs'] == {'count': 2, 'mode': 'safe', 'optional': None, 'text': PUBLIC}
                assert 'HELM_WORKFLOW_TOKEN' in output['prompt']
                assert b'public, model-visible; recorded when run is saved' in terminal.output
                assert b'token: ' not in terminal.output
                assert not (root / 'never-created').exists()
            finally:
                terminal.close()
            # Trust is checked before any prompt; changed definitions never dispatch.
            repository = root / '.helm/workflows'
            repository.mkdir(parents=True)
            repo_file = repository / 'collect-inputs.toml'
            repo_file.write_text(DOCUMENT)
            rejected = subprocess.run([str(HELM), *preview], stdin=subprocess.DEVNULL, capture_output=True, env=env, timeout=4)
            assert b'trust' in rejected.stderr.lower() and b'count: ' not in rejected.stderr
            digest = hashlib.sha256(DOCUMENT.encode()).hexdigest()
            terminal = Terminal([*preview, '--trust-repository', digest], env, root)
            try:
                terminal.marker('count: ')
                terminal.send('1\r')
                terminal.marker('text: ')
                repo_file.write_text(DOCUMENT.replace("version='1'", "version='2'"))
                terminal.send('safe\r')
                terminal.finish(1)
                assert b'changed during input collection' in terminal.output
                assert not private.Provider.requests
            finally:
                terminal.close()
            repo_file.unlink()
            terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing'], env, root)
            try:
                terminal.marker('target: ')
                private_document = user / 'review-change.toml'
                private_document.write_text(private.DOCUMENT.replace('version="2.0"', 'version="2.1"'))
                terminal.send(SECRET + '\r')
                terminal.finish(1)
                assert b'changed during input collection' in terminal.output
                assert not private.Provider.requests
                private_document.write_text(private.DOCUMENT)
            finally:
                terminal.close()
            # Hidden input uses the existing exact run binding and private shell adapter.
            private.Provider.sessions = root / 'data/helm/sessions'
            terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing'], env, root)
            try:
                terminal.marker('target: ')
                assert b'secret, hidden' in terminal.output
                fcntl.ioctl(terminal.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 90, 900, 600))
                terminal.send(SECRET + 'x\x7f\r')
                output = terminal.finish()
                assert b'private-workflow-finished' in output
                assert (root / 'value-digest').read_text() == hashlib.sha256(SECRET.encode()).hexdigest()
                assert private.Provider.requests and not private.Provider.failures, private.Provider.failures
                public = json.dumps(private.Provider.requests, ensure_ascii=False)
                for path in (root / 'data').rglob('*'):
                    if path.is_file():
                        public += path.read_bytes().decode(errors='replace')
                assert SECRET not in public
            finally:
                terminal.close()
            count = len(list(private.Provider.sessions.glob('*.json')))
            private.Provider.no_save = True
            terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing', '--no-save'], env, root)
            try:
                terminal.marker('target: ')
                terminal.send(SECRET + '\r')
                terminal.finish()
                assert len(list(private.Provider.sessions.glob('*.json'))) == count
            finally:
                terminal.close()
                private.Provider.no_save = False
            # The ordinary approval reader starts only after collection has released stdin.
            original_config = config.read_text()
            config.write_text(original_config.replace('access="unrestricted"', 'access="approval"'))
            (root / 'value-digest').unlink()
            terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing'], env, root, stdout_terminal=True)
            try:
                terminal.marker('target: ')
                terminal.send(SECRET + '\rqueued-y\r')
                terminal.marker('Proceed? [y/N] ', hidden=False)
                assert not (root / 'value-digest').exists(), 'queued collector input granted approval'
                terminal.send('y\r')
                terminal.finish()
                assert (root / 'value-digest').read_text() == hashlib.sha256(SECRET.encode()).hexdigest()
            finally:
                terminal.close()
                config.write_text(original_config)
            # A policy selected before collection must still be current afterward.
            base = ['--config', str(config), '--workspace', str(root), '--policy-directory', str(root / 'profiles')]
            def policy(*args):
                result = subprocess.run([str(HELM), *base, 'policy', *args], env=env, cwd=root,
                                        stdin=subprocess.DEVNULL, capture_output=True, timeout=8)
                assert result.returncode == 0, 'policy fixture preparation failed'
                return json.loads(result.stdout)
            policy('create', 'attended-current', '--preset', 'autonomous')
            snapshot = policy('inspect', 'attended-current')
            flags = ['--policy-profile', 'attended-current', '--policy-revision', '1', '--policy-digest', snapshot['digest']]
            transition = policy('preview', 'attended-current', '--revision', '1', '--digest', snapshot['digest'])['preview']
            if transition['requires_confirmation']:
                flags += ['--policy-confirm', transition['transition_digest']]
            request_count = len(private.Provider.requests)
            (root / 'value-digest').unlink()
            terminal = Terminal([*base, *flags, 'workflow', '--user-directory', str(user), 'run', 'review-change', '--prompt-missing'], env, root)
            try:
                terminal.marker('target: ')
                policy('delete', 'attended-current', '--expected-revision', '1')
                terminal.send(SECRET + '\r')
                terminal.finish(1)
                assert len(private.Provider.requests) == request_count
                assert not (root / 'value-digest').exists()
            finally:
                terminal.close()
            # Collection's TERM/HUP subscriptions remain active through an admitted run.
            for signum in (signal.SIGTERM, signal.SIGHUP):
                entered, release = threading.Event(), threading.Event()
                original_post = private.Provider.do_POST
                def hold(handler):
                    request = json.loads(handler.rfile.read(int(handler.headers['Content-Length'])))
                    private.Provider.requests.append(request)
                    entered.set()
                    release.wait(10)
                private.Provider.do_POST = hold
                terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing'], env, root)
                try:
                    terminal.marker('target: ')
                    terminal.send(SECRET + '\r')
                    assert entered.wait(8), 'provider did not receive admitted workflow'
                    terminal.proc.send_signal(signum)
                    terminal.finish(1, timeout=8)
                    assert not (root / 'value-digest').exists()
                    records = [json.loads(path.read_text()) for path in private.Provider.sessions.glob('*.json')]
                    latest = max(records, key=lambda record: record['updated_at'])
                    assert latest['run_summaries'][-1]['phase'] == 'interrupted'
                finally:
                    release.set()
                    private.Provider.do_POST = original_post
                    terminal.close()
            # No subsequent invocation/session/provider request occurs on any cancellation.
            request_count = len(private.Provider.requests)
            record_count = len(list(private.Provider.sessions.glob('*.json')))
            for ending in ('escape', 'ctrl-c', 'eof', 'term', 'hup', 'int', 'timeout', 'invalid', 'oversize'):
                terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing', '--input-timeout-seconds', '1'], env, root)
                try:
                    terminal.marker('target: ')
                    terminal.send(SECRET)
                    if ending in ('term', 'hup', 'int'):
                        terminal.proc.send_signal({'term': signal.SIGTERM, 'hup': signal.SIGHUP, 'int': signal.SIGINT}[ending])
                    elif ending in ('escape', 'ctrl-c', 'eof'):
                        terminal.send({'escape': b'\x1b', 'ctrl-c': b'\x03', 'eof': b'\x04'}[ending] + b'queued-tail')
                    elif ending == 'invalid':
                        terminal.send(b'\x00queued-tail')
                    elif ending == 'oversize':
                        terminal.send(b'x' * 9000)
                    terminal.finish(1 if ending in ('timeout', 'invalid', 'oversize') else 130, timeout=4)
                finally:
                    terminal.close()
            assert len(private.Provider.requests) == request_count
            assert len(list(private.Provider.sessions.glob('*.json'))) == record_count
            # Cancellation and timeout also terminate when the terminal cannot drain output.
            for ending in ('signal', 'timeout'):
                terminal = Terminal([*common, 'run', 'review-change', '--prompt-missing', '--input-timeout-seconds', '1'], env, root, stalled=True)
                try:
                    deadline = time.monotonic() + 5
                    while termios.tcgetattr(terminal.slave)[3] & termios.ECHO:
                        assert time.monotonic() < deadline
                        time.sleep(.01)
                    terminal.send(SECRET)
                    if ending == 'signal':
                        terminal.proc.send_signal(signal.SIGTERM)
                    terminal.proc.wait(timeout=4)
                    assert terminal.proc.returncode == (130 if ending == 'signal' else 1)
                    assert termios.tcgetattr(terminal.slave) == terminal.before
                    termios.tcflow(terminal.slave, termios.TCOON)
                    terminal.finish(130 if ending == 'signal' else 1)
                finally:
                    terminal.close()
            print('workflow prompting: typed preview, exact trust/change rejection, private native execution, nine cancellation/error paths and two stalled-output paths passed')
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
