#!/usr/bin/env python3
"""Exercise the fixed Linux administrator path in a disposable user/mount namespace.

No host /etc writes or production loader bypass. Namespace-unavailable runners
report SKIP honestly; deterministic loader/runtime unit coverage remains required.
"""
from __future__ import annotations
import json
import re
import unicodedata
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def rendered_screen(data: bytes, rows: int = 14, columns: int = 60) -> str:
    """Decode the cursor-addressed full redraw; Ratatui skips blank cells."""
    grid = [[' '] * columns for _ in range(rows)]
    row = column = 0
    for part in re.split(r'(\x1b\[[0-?]*[ -/]*[@-~])', data.decode('utf-8', errors='replace')):
        if part.startswith('\x1b['):
            if part.endswith('H') or part.endswith('f'):
                coordinates = part[2:-1].split(';')
                row = int(coordinates[0] or '1') - 1
                column = int(coordinates[1] or '1') - 1 if len(coordinates) > 1 else 0
            elif part == '\x1b[2J':
                grid = [[' '] * columns for _ in range(rows)]
            continue
        for char in part:
            if char == '\r': column = 0
            elif char == '\n': row += 1
            elif char.isprintable():
                width = 0 if unicodedata.combining(char) else 2 if unicodedata.east_asian_width(char) in ('W', 'F') else 1
                if 0 <= row < rows and 0 <= column < columns:
                    if width == 0 and column: grid[row][column - 1] += char
                    else:
                        grid[row][column] = char
                        if width == 2 and column + 1 < columns: grid[row][column + 1] = ''
                column += width
    return '\n'.join(''.join(line) for line in grid)


def inside() -> None:
    import pty
    import select
    import time
    import termios
    import fcntl
    import struct
    import signal
    work = Path('/work')
    work.mkdir()
    Path('/etc/helm').mkdir(parents=True)
    ceiling = Path('/etc/helm/policy-ceiling.toml')
    requests: list[dict] = []
    model_requests: list[str] = []
    scenario = ['plain']
    nested_cwds: list[str] = []
    nested_failures: list[str] = []

    def nested_response(body, outputs):
        users = ' '.join(str(value.get('content', '')) for value in body['input'] if value.get('role') == 'user')
        role = re.search(r'nested-role-(root|child|grandchild)', users).group(1)
        def call(name, args):
            serial = len(requests)
            return [{'type': 'function_call', 'id': f'fc_nested_{serial}', 'call_id': f'nested_{serial}', 'name': name, 'arguments': json.dumps(args)}]
        def message(text):
            return [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': text}]}]
        if role == 'grandchild':
            if not outputs: return call('shell', {'command': 'pwd'})
            if len(outputs) == 1:
                value = outputs[0]['output']
                assert value.startswith('exit: 0\nstdout:\n'), value
                cwd = value.split('stdout:\n', 1)[1].split('\nstderr:', 1)[0].strip()
                nested_cwds.append(cwd)
                return call('read_file', {'path': '/work/tracked.txt'})
            assert 'read outside allowed roots' in outputs[-1]['output'], outputs[-1]
            return message(nested_cwds[-1])
        if not outputs:
            return call('subagent', {'action': 'spawn', 'name': 'isolated' if role == 'root' else 'nested', 'task': 'nested-role-child' if role == 'root' else 'nested-role-grandchild', 'worktree': role == 'root'})
        if len(outputs) == 1:
            child = json.loads(outputs[0]['output'])['id']
            return call('subagent', {'action': 'wait', 'id': child})
        result = json.loads(outputs[-1]['output'])
        assert result['status'] == 'completed', result
        return message(result['result']['summary'])

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            model_requests.append(self.path)
            data = json.dumps({'data': [{'id': 'fixture'}]}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(body)
            if scenario[0] == 'change':
                cap('unrestricted')
            outputs = [v for v in body.get('input', []) if v.get('type') == 'function_call_output']
            if scenario[0] == 'nested':
                try: output = nested_response(body, outputs)
                except Exception as error:
                    nested_failures.append(repr(error) + " outputs=" + repr(outputs)[-2500:])
                    output = [{'type': 'message', 'role': 'assistant', 'content': [{'type': 'output_text', 'text': 'nested fixture failed'}]}]
            elif scenario[0] == 'environment' and not outputs:
                output = [{'type': 'function_call', 'id': 'fc_env', 'call_id': 'env',
                           'name': 'shell', 'arguments': json.dumps({'command': '/usr/bin/env'})}]
            else:
                output = [{'type': 'message', 'role': 'assistant',
                           'content': [{'type': 'output_text', 'text': 'fixture complete'}]}]
            data = ('data: ' + json.dumps({'type': 'response.completed',
                    'response': {'output': output, 'usage': {'input_tokens': 1, 'output_tokens': 1}}}) + '\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = '\n'.join([
        'provider = "openai-responses"', 'model = "fixture"',
        'api_key_env = "FIXTURE_KEY"', f'base_url = "http://127.0.0.1:{server.server_port}/v1"',
        'provider_retry_attempts = 1', 'workspace = "/work"',
        'access = "unrestricted"', 'unattended_approval = "allow"',
        'inherit_env = ["PATH", "AMBIENT_REMOVED"]',
        '[env]', 'EXPLICIT = "configured-value"', 'REMOVED = "do-not-export-canary"',
    ]) + '\n'
    config = work / 'config.toml'
    config.write_text(base)
    environment = dict(os.environ, HOME='/work/home', XDG_DATA_HOME='/work/data',
                       XDG_CONFIG_HOME='/work/config', FIXTURE_KEY='offline-fixture',
                       AMBIENT_REMOVED='ambient-canary', PATH='/usr/bin:/bin')

    def run(*args, input=None):
        return subprocess.run(['/helm', '--config', str(config), *args], input=input,
                              capture_output=True, text=True, env=environment, timeout=30)

    def cap(mode='unrestricted', roots='["$workspace"]'):
        ceiling.write_text(f'''schema = 1
[rules]
access = "{mode}"
unattended = "deny"
read_roots = {roots}
write_roots = {roots}
deny_commands = ["shutdown"]
inherit_env = ["PATH", "EXPLICIT"]
''')
        ceiling.chmod(0o600)

    # Existing no-ceiling startup and explicit environment behavior.
    scenario[0] = 'environment'
    result = run('run', '--no-save', 'fixture')
    assert result.returncode == 0, result.stderr
    output = json.dumps(requests[-1]['input'])
    assert 'EXPLICIT=' in output and 'AMBIENT_REMOVED=ambient-canary' in output, output
    requests.clear()
    cap()
    result = run('run', '--no-save', 'fixture')
    assert result.returncode == 0, result.stderr
    output = json.dumps(requests[-1]['input'])
    assert 'EXPLICIT=' in output, output
    assert 'REMOVED=' not in output and 'AMBIENT_REMOVED=' not in output, output

    # Actual MCP startup preserves server-over-global precedence only for capped names.
    mcp = work / 'mcp.py'
    mcp.write_text("""import json, os, sys
from pathlib import Path
Path('/work/mcp-env.json').write_text(json.dumps(dict(os.environ)))
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    result = {'tools': []} if request['method'] == 'tools/list' else {'protocolVersion': '2025-06-18', 'capabilities': {}, 'serverInfo': {'name': 'fixture', 'version': '1'}}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
""")
    config.write_text(base + '\n[mcp_servers.env]\ncommand = "/usr/bin/python3"\nargs = ["/work/mcp.py"]\n[mcp_servers.env.env]\nEXPLICIT = "server-value"\nREMOVED = "server-canary"\n')
    scenario[0] = 'plain'
    result = run('run', '--no-save', 'fixture')
    assert result.returncode == 0, result.stderr
    mcp_environment = json.loads((work / 'mcp-env.json').read_text())
    assert mcp_environment['EXPLICIT'] == 'server-value'
    assert 'REMOVED' not in mcp_environment and 'AMBIENT_REMOVED' not in mcp_environment

    # Read-only clamps tools before starting an explicitly configured MCP process.
    scenario[0] = 'plain'
    config.write_text(base + '\n[mcp_servers.marker]\ncommand = "/bin/sh"\nargs = ["-c", "touch /work/mcp-started"]\n')
    cap('read-only')
    requests.clear()
    result = run('run', '--no-save', 'fixture')
    assert result.returncode == 0, result.stderr
    names = [t.get('name') for t in requests[0]['tools']]
    assert 'shell' not in names and 'write_file' not in names, names
    assert not (work / 'mcp-started').exists()
    label = run('chat', '--plain', input='/access\n/exit\n')
    assert label.returncode == 0 and 'read-only' in label.stdout, (label.stdout, label.stderr)

    # The real TUI displays the clamped mode and never starts the denied MCP.
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 30, 100, 0, 0))
    prior_mode = termios.tcgetattr(slave)
    request_count, model_count = len(requests), len(model_requests)
    child = subprocess.Popen(['/helm', '--config', str(config), 'chat'], stdin=slave, stdout=slave, stderr=slave,
                             env=dict(environment, TERM='xterm-256color'))
    captured = bytearray()
    deadline = time.monotonic() + 10
    try:
        while time.monotonic() < deadline and b'read-only' not in captured:
            if select.select([master], [], [], .1)[0]:
                captured.extend(os.read(master, 65536))
            if child.poll() is not None: break
        assert b'read-only' in captured, captured[-2000:]
        cap('unrestricted')
        cutoff = len(captured)
        refused = 'tui refused before canonical 界'
        os.write(master, (refused + '\r').encode())
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and b'policy' not in captured[cutoff:].lower():
            if select.select([master], [], [], .1)[0]: captured.extend(os.read(master, 65536))
        assert b'policy' in captured[cutoff:].lower(), captured[-2000:]
        cutoff = len(captured)
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 14, 60, 0, 0))
        os.kill(child.pid, signal.SIGWINCH)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and refused not in rendered_screen(captured[cutoff:]):
            if select.select([master], [], [], .1)[0]: captured.extend(os.read(master, 65536))
        assert refused in rendered_screen(captured[cutoff:]), ('refused prompt must stay in composer after resize', rendered_screen(captured[cutoff:]))
        os.write(master, b'\x11')
        child.wait(timeout=10)
        assert child.returncode == 0, captured[-2000:]
        assert (len(requests), len(model_requests)) == (request_count, model_count)
        assert termios.tcgetattr(slave) == prior_mode
        assert not (work / 'mcp-started').exists()
        assert all(refused not in path.read_text() for path in Path('/work/data').rglob('sessions/*.json'))
    finally:
        if child.poll() is None: child.kill(); child.wait()
        os.close(master)
        os.close(slave)

    # Reused line-mode Agent observes a changed ceiling before a second turn.
    requests.clear()
    cap('read-only')
    scenario[0] = 'change'
    result = run('chat', '--plain', input='first accepted\nsecond must not be sent\n/exit\n')
    assert result.returncode == 0, result.stderr
    assert len(requests) == 1, requests
    assert 'restart or rebuild' in result.stderr, result.stderr
    session_files = list(Path('/work/data').rglob('sessions/*.json'))
    assert session_files, 'completed first turn must remain durable'
    saved = next(value for path in session_files if 'first accepted' in json.dumps(value := json.loads(path.read_text())))
    assert 'second must not be sent' not in json.dumps(saved)
    scenario[0] = 'plain'
    ceiling.write_text('malformed =')
    requests.clear()
    result = run('run', '--no-save', '--resume', saved['id'], 'resume denied')
    assert result.returncode != 0 and not requests, result.stderr

    # Every invalid source stops before model requests or MCP effects.
    for invalid in ['malformed', 'mode', 'symlink', 'hardlink', 'workspace']:
        if ceiling.is_symlink():
            ceiling.unlink()
        cap()
        if invalid == 'malformed': ceiling.write_text('not valid TOML =')
        elif invalid == 'mode': ceiling.chmod(0o666)
        elif invalid == 'symlink':
            ceiling.rename('/work/linked-ceiling')
            ceiling.symlink_to('/work/linked-ceiling')
        elif invalid == 'hardlink': os.link(ceiling, '/work/hardlinked-ceiling')
        elif invalid == 'workspace': cap(roots='["/etc"]')
        requests.clear()
        result = run('run', '--no-save', 'fixture')
        assert result.returncode != 0 and not requests, (invalid, result.stdout, result.stderr)
        assert not (work / 'mcp-started').exists()
        assert 'do-not-export-canary' not in result.stderr
        model_requests.clear()
        result = run('models', '--json')
        assert result.returncode != 0 and not requests and not model_requests, (invalid, result.stdout, result.stderr)
        result = run('chat', '--plain', input='/models\n/exit\n')
        assert not requests and not model_requests and 'model discovery failed' in result.stderr, (invalid, result.stdout, result.stderr)
        if invalid == 'hardlink': Path('/work/hardlinked-ceiling').unlink()
    # A worktree child loses the root workspace under a workspace-relative ceiling.
    # Its non-owning descendant must execute in that child's cwd, not regain /work.
    Path('/state').mkdir()
    for arguments in [('init', '-q'), ('config', 'user.email', 'fixture@example.invalid'), ('config', 'user.name', 'Fixture')]:
        subprocess.run(['/usr/bin/git', *arguments], cwd=work, check=True, capture_output=True)
    (work / 'tracked.txt').write_text('base')
    subprocess.run(['/usr/bin/git', 'add', 'tracked.txt'], cwd=work, check=True, capture_output=True)
    subprocess.run(['/usr/bin/git', 'commit', '-qm', 'base'], cwd=work, check=True, capture_output=True)
    config.write_text(base.replace('[env]', 'allow_read = ["/state"]\nallow_write = ["/state"]\nsubagent_max_concurrency = 2\n[env]'))
    cap(roots='["$workspace", "/state"]')
    environment['XDG_DATA_HOME'] = '/state'
    scenario[0] = 'nested'
    result = run('run', '--no-save', 'nested-role-root')
    assert result.returncode == 0 and not nested_failures, (result.stderr, nested_failures)
    assert len(nested_cwds) == 1 and nested_cwds[0].startswith('/state/helm/worktrees/'), nested_cwds
    scenario[0] = 'plain'
    environment['XDG_DATA_HOME'] = '/work/data'

    # The compatibility bridge cannot spawn through model discovery under denial.
    sentinel = work / 'codex-sentinel'
    sentinel.write_text('#!/bin/sh\ntouch /work/bridge-started\nexit 99\n')
    sentinel.chmod(0o700)
    config.write_text(base.replace('provider = "openai-responses"', 'provider = "codex-compatibility"\ncodex_command = "/work/codex-sentinel"'))
    ceiling.write_text('malformed =')
    for arguments, input_text in [(('models', '--json'), None), (('chat', '--plain'), '/models\n/exit\n')]:
        result = run(*arguments, input=input_text)
        assert not (work / 'bridge-started').exists(), result.stderr
        assert 'policy' in result.stderr, result.stderr
    server.shutdown()
    print('policy ceiling fixed-path CLI: passed')


def main() -> None:
    if sys.argv[1:] == ['--inside']:
        inside()
        return
    if sys.platform != 'linux' or not shutil.which('unshare'):
        print('SKIP policy ceiling namespace fixture: Linux user/mount namespaces unavailable')
        return
    probe = subprocess.run(['unshare', '--user', '--map-root-user', '--mount', '--propagation', 'private', 'true'], capture_output=True, timeout=10)
    if probe.returncode:
        print('SKIP policy ceiling namespace fixture: user/mount namespace permission unavailable')
        return
    helm = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()
    with tempfile.TemporaryDirectory(prefix='helm-ceiling-') as directory:
        root = Path(directory)
        for name in ['usr', 'dev', 'proc', 'tmp']:
            (root / name).mkdir()
        for name in ['bin', 'sbin', 'lib', 'lib64']:
            source = Path('/') / name
            if source.is_symlink(): (root / name).symlink_to(os.readlink(source))
            elif source.exists(): (root / name).mkdir()
        shutil.copy2(helm, root / 'helm')
        shutil.copy2(__file__, root / 'fixture.py')
        script = '''set -eu
root=$1
mount --bind /usr "$root/usr"
mkdir "$root/dev/pts"
mount -t devpts devpts "$root/dev/pts" -o newinstance,ptmxmode=0666,mode=0620
ln -s pts/ptmx "$root/dev/ptmx"
for device in null zero urandom random; do
    touch "$root/dev/$device"
    mount --bind "/dev/$device" "$root/dev/$device"
done
for part in lib lib64 bin sbin; do
    if [ -d "/$part" ] && [ ! -L "/$part" ]; then mount --bind "/$part" "$root/$part"; fi
done
exec chroot "$root" /usr/bin/python3 /fixture.py --inside
'''
        subprocess.run(['unshare', '--user', '--map-root-user', '--mount', '--propagation', 'private',
                        '/bin/sh', '-c', script, 'fixture', str(root)], check=True, timeout=180)

if __name__ == '__main__':
    main()
