"""Focused offline Linux Helm PTY journeys against already-built binaries.

  python3 voyage/tests/ui_journeys.py --bin-dir /home/psi/voyage/target/debug

No build, real provider, desktop service, credentials or human sign-off. Evidence
(including raw PTY traces and canonical snapshots) stays in a private temp dir.
Uses the existing images_composer PTY send/paste/stop helpers and two_voyages
bounded wait helper. This is not a general terminal emulator or a platform suite.
"""
import argparse
import errno
import fcntl
import http.server
import hashlib
import json
import os
from pathlib import Path
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time
import unicodedata
import urllib.request

import images_composer as pty_helpers
from two_voyages import wait_for, Provider as ToolProvider

F2, F9 = '\x1bOQ', '\x1b[20~'
DOWN, ENTER, ESC = '\x1b[B', '\r', '\x1b'
send, paste = pty_helpers.send, pty_helpers.paste


def launch(argv, env, cwd, trace, columns, rows):
    """Set geometry before exec, not a resize of an already wide startup."""
    master, slave = os.openpty()
    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, columns, 0, 0))
        process = subprocess.Popen(
            [sys.executable, str(Path(pty_helpers.__file__).resolve()), '_pty_exec', *argv],
            env=env, cwd=cwd, stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    except BaseException:
        os.close(master)
        raise
    finally:
        os.close(slave)
    state = dict(process=process, pid=process.pid, fd=master, output=bytearray(),
                 status=None, columns=columns, rows=rows, reader_errors=[])

    def drain():
        try:
            with trace.open('wb') as output:
                while True:
                    if not select.select([master], [], [], .1)[0]:
                        continue
                    data = os.read(master, 65536)
                    if not data:
                        break
                    state['output'].extend(data)
                    output.write(data)
                    output.flush()
        except OSError as error:
            if error.errno not in (errno.EIO, errno.EBADF):
                state['reader_errors'].append(repr(error))
        except Exception as error:
            state['reader_errors'].append(repr(error))
    state['reader'] = threading.Thread(target=drain, daemon=True)
    state['reader'].start()
    return state


def screen(pty):
    """Reconstruct ratatui differential frames, including wide/combining text.

    Assertions use ASCII UI labels here; exact Unicode is checked in persisted
    drafts and provider/canonical messages, not inferred from glyph appearance.
    """
    rows, columns = pty['rows'], pty['columns']
    cells = [[' '] * columns for _ in range(rows)]
    row = col = 0
    text = bytes(pty['output']).decode('utf-8', errors='replace')
    for token in re.finditer(r'\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|\x1b.|[^\x1b]', text):
        value = token.group()
        if value.startswith('\x1b['):
            command, params = value[-1], value[2:-1]
            if params.startswith('?'):
                continue
            if not re.fullmatch(r'[0-9;]*', params):
                continue
            nums = [int(x or 0) for x in params.split(';')]
            n = nums[0] or 1
            if command in ('H', 'f'):
                row, col = n - 1, (nums[1] or 1) - 1 if len(nums) > 1 else 0
            elif command == 'J' and nums[0] in (2, 3):
                cells = [[' '] * columns for _ in range(rows)]
            elif command == 'K' and 0 <= row < rows:
                lo, hi = (0, columns) if nums[0] == 2 else ((0, col + 1) if nums[0] == 1 else (col, columns))
                for c in range(max(0, lo), min(columns, hi)):
                    cells[row][c] = ' '
            elif command == 'G':
                col = n - 1
            elif command == 'A':
                row = max(0, row - n)
            elif command == 'B':
                row += n
            elif command == 'C':
                col += n
            elif command == 'D':
                col = max(0, col - n)
        elif value.startswith('\x1b'):
            continue
        elif value == '\r':
            col = 0
        elif value == '\n':
            row += 1
        elif value >= ' ':
            width = 0 if unicodedata.combining(value) else (2 if unicodedata.east_asian_width(value) in ('W', 'F') else 1)
            if 0 <= row < rows and 0 <= col < columns:
                if width == 0 and col:
                    cells[row][col - 1] += value
                else:
                    cells[row][col] = value
                    if width == 2 and col + 1 < columns:
                        cells[row][col + 1] = ''
            col += width
    return '\n'.join(''.join(line) for line in cells)


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        self.connection.settimeout(10)
        try:
            assert self.path == '/v1/chat/completions', self.path
            size = int(self.headers['Content-Length'])
            assert 0 < size <= 4 * 1024 * 1024
            body = json.loads(self.rfile.read(size))
            assert body['stream'] is True
            messages = body['messages']
            users = [m['content'] for m in messages if m['role'] == 'user']
            prompt = users[-1]
            assert prompt in self.server.prompts or prompt.startswith('WORKFLOW-UI '), users
            with self.server.lock:
                self.server.requests.append(body)
                self.server.counts[prompt] = self.server.counts.get(prompt, 0) + 1
                step = self.server.counts[prompt]
            if prompt.startswith('WORKFLOW-UI '):
                assert 'private-ui-sentinel-14' not in json.dumps(body)
                assert 'HELM_WORKFLOW_TOKEN' in prompt and 'ready' in prompt
                delta = {'content': 'WORKFLOW-UI-COMPLETED'}
            elif prompt == self.server.approval:
                if step == 1:
                    assert 'write_file' in {t['function']['name'] for t in body['tools']}
                    delta = ToolProvider.tool('denied-write', 'write_file', {
                        'path': 'must-not-exist.txt', 'content': 'NOT AUTHORIZED'})
                else:
                    assert step == 2 and messages[-1]['role'] == 'tool', messages[-1]
                    assert not (self.server.workspace / 'must-not-exist.txt').exists()
                    delta = {'content': 'Denial observed; no file written.'}
            else:
                assert step == 1, 'unexpected provider replay'
                self.server.arrived[prompt].set()
                assert self.server.release.wait(90), 'concurrent provider barrier timed out'
                delta = {'content': self.server.prompts[prompt]}
            payload = ''.join('data: ' + json.dumps(event) + '\n\n' for event in [
                {'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]},
                {'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls' if 'tool_calls' in delta else 'stop'}]}]) + 'data: [DONE]\n\n'
            payload = payload.encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except Exception as error:
            self.server.errors.append(repr(error))
            self.close_connection = True


def file_sha256(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    args = parser.parse_args()
    binary = args.bin_dir.resolve()
    assert sys.platform == 'linux' and hasattr(os, 'pidfd_open'), 'Linux pidfds required'
    for name in ('helm', 'vessel', 'voyage'):
        assert (binary / name).is_file(), binary / name
    root = Path(tempfile.mkdtemp(prefix='helm-ui-journeys-'))
    print('evidence:', root, flush=True)
    (root / 'build-identity.json').write_text(json.dumps({
        'binaries': {name: file_sha256(binary / name)
                     for name in ('helm', 'vessel', 'voyage')},
        'fixture_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'platform': sys.platform, 'python': sys.version, 'argv': sys.argv,
    }, indent=2))
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'TERM': 'xterm-256color'}
    for key in ('HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME'):
        path = root / key.lower()
        path.mkdir(mode=0o700)
        env[key] = str(path)
    workspace = root / 'workspace'
    workspace.mkdir()
    directory = Path(env['XDG_STATE_HOME']) / 'voyage' / 'vessel'
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.daemon_threads = False  # server_close joins all bounded request handlers.
    server.requests, server.errors, server.counts = [], [], {}
    server.lock, server.release = threading.Lock(), threading.Event()
    alpha = 'orchard-unique 日本語 e\u0301 🧭\nsecond line'
    beta = 'harbor-unique Ελληνικά'
    server.approval = 'approval-unique try the denied write'
    server.prompts = {alpha: 'ORCHARD-REPLY-ONLY', beta: 'HARBOR-REPLY-ONLY', server.approval: ''}
    server.arrived = {p: threading.Event() for p in (alpha, beta)}
    server.workspace = workspace
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / 'config.toml'
    config.write_text('provider = "openai-chat"\nmodel = "fixture-model"\napi_key_required = false\n'
                      f'base_url = "http://127.0.0.1:{server.server_port}/v1"\n'
                      'provider_retry_attempts = 1\naccess = "approval"\ncontext_window = 0\n')
    config.chmod(0o600)
    ptys, checks, cleanup_errors = [], [], []
    supervisor = None
    log = (root / 'vessel.log').open('wb')

    def request(command):
        credential = json.loads((directory / 'process-http.json').read_text())
        req = urllib.request.Request(credential['endpoint'] + '/v1/vessel/command',
            data=json.dumps({'protocol': 1, 'command': command}).encode(),
            headers={'Authorization': 'Bearer ' + credential['token'], 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=5) as response:
            value = json.load(response)
        assert value.get('error') is None, value
        return value['result']

    def snapshot(sid):
        value = request({'op': 'snapshot', 'session_id': sid})
        assert value.get('error') is None, value
        return value.get('result', value)

    def catalogue():
        return request({'op': 'catalogue'})

    def drafts():
        return [json.loads(p.read_text()) for p in root.rglob('helm-new-drafts/*.json')]

    def saved(text):
        return any(d['text'] == text for d in drafts())

    def expect(pty, text):
        def observe():
            assert not server.errors, server.errors
            assert pty['process'].poll() is None, 'Helm exited early'
            assert not pty['reader_errors'], pty['reader_errors']
            return text in screen(pty)
        try:
            wait_for(observe, timeout=15)
        except AssertionError:
            (root / 'failed-screen.txt').write_text(screen(pty))
            raise AssertionError(f'expected {text!r}; see {root / "failed-screen.txt"}') from None

    def start(columns=120, rows=32):
        pty = launch([str(binary / 'helm'), '--config', str(config), 'chat'], env, workspace,
                     root / f'helm-{len(ptys)}-{columns}x{rows}.pty', columns, rows)
        ptys.append(pty)
        wait_for(lambda: any(label in screen(pty) for label in
                            ('First message', 'Permission needed', 'F2 Voyages')), timeout=15)
        return pty

    def stop(pty):
        pty_helpers.stop_pty(pty, wait_for)
        assert pty['process'].returncode == 0, pty['process'].returncode
        assert not pty['reader_errors'], pty['reader_errors']

    def pick(pty, query):
        send(pty, F2)
        expect(pty, 'Find voyages')
        paste(pty, query)
        expect(pty, 'Search: ' + query)
        # Paste is query input, never a navigation/submit event.
        assert 'Find voyages' in screen(pty)
        send(pty, ENTER)
        wait_for(lambda: 'Find voyages' not in screen(pty), timeout=10)

    def session_for(prompt):
        for entry in catalogue():
            value = snapshot(entry['session_id'])
            if any(m['role'] == 'user' and m['content'] == prompt for m in value['messages']):
                return entry['session_id']
        return None

    def finished(sid):
        def observe():
            assert not server.errors, server.errors
            value = snapshot(sid)
            state = (value.get('run') or {}).get('state')
            assert state not in ('failed', 'cancelled'), value.get('run')
            return value if state == 'completed' and value.get('pending_cleanup_run') is None else None
        return wait_for(observe)

    def owned_pids():
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    argv = (entry / 'cmdline').read_bytes().split(b'\0')
                    if (argv[:3] == [os.fsencode(binary / 'voyage'), b'serve', b'--directory']
                            and len(argv) > 3 and Path(os.fsdecode(argv[3])).parent == directory / 'sessions'):
                        found.append(int(entry.name))
                except (FileNotFoundError, ProcessLookupError):
                    pass
        return found

    def signal_owned(sig):
        for pid in owned_pids():
            try:
                fd = os.pidfd_open(pid)
                try:
                    if pid in owned_pids():
                        signal.pidfd_send_signal(fd, sig)
                finally:
                    os.close(fd)
            except ProcessLookupError:
                pass

    try:
        supervisor = subprocess.Popen([str(binary / 'vessel'), 'local-serve', '--directory', str(directory),
            '--voyage-binary', str(binary / 'voyage')], env=env, cwd=workspace,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / 'process-http.json').exists())
        # Each size is a real startup, with no sessions and no provider calls.
        for columns, rows in ((40, 18), (80, 24), (120, 32)):
            pty = start(columns, rows)
            expect(pty, 'New voyage')
            expect(pty, 'First message')
            assert 'Enlarge to at least' not in screen(pty)
            text = f'size-{columns}-unique 界 e\u0301'
            if columns != 40:
                send(pty, '\x0e')
                expect(pty, 'First message')
            paste(pty, text)
            wait_for(lambda: saved(text))
            send(pty, F2)
            expect(pty, 'Find voyages')
            paste(pty, 'definitely-no-such-item')
            expect(pty, 'No matching voyages or drafts')
            send(pty, ENTER)
            time.sleep(.2)
            expect(pty, 'No matching voyages or drafts')
            assert saved(text) and not catalogue() and not server.requests
            send(pty, ESC)
            expect(pty, 'First message')
            stop(pty)
            assert saved(text), 'detach lost the draft'
            checks.append(f'{columns}x{rows}: normal startup, exact Unicode draft, empty F2 search cannot submit')

        pty = start()
        pick(pty, 'size-40-unique')
        expect(pty, 'First message')
        send(pty, '\x0e')
        expect(pty, 'First message')
        paste(pty, alpha)
        wait_for(lambda: saved(alpha))
        send(pty, '\x0e')
        paste(pty, beta)
        wait_for(lambda: saved(beta))
        pick(pty, 'orchard-unique')
        expect(pty, 'First message')
        send(pty, ENTER)
        wait_for(lambda: server.arrived[alpha].is_set())
        sid_a = wait_for(lambda: session_for(alpha))
        pick(pty, 'harbor-unique')
        expect(pty, 'First message')
        send(pty, ENTER)
        wait_for(lambda: server.arrived[beta].is_set())
        sid_b = wait_for(lambda: session_for(beta))
        assert sid_a != sid_b
        assert len(owned_pids()) == 2, owned_pids()
        overlap = {sid: snapshot(sid) for sid in (sid_a, sid_b)}
        assert all(v['run']['state'] == 'running' for v in overlap.values())
        (root / 'overlap.json').write_text(json.dumps({'pids': owned_pids(), 'snapshots': overlap}, indent=2))
        # An actual Helm detach leaves both independent Voyage processes running.
        stop(pty)
        assert all(snapshot(sid)['run']['state'] == 'running' for sid in (sid_a, sid_b))
        server.release.set()
        for sid, prompt in ((sid_a, alpha), (sid_b, beta)):
            value = finished(sid)
            assert [m['content'] for m in value['messages'] if m['role'] == 'user'] == [prompt]
            assert [m['content'] for m in value['messages'] if m['role'] == 'assistant' and not m.get('tool_calls')] == [server.prompts[prompt]]
            (root / f'{sid}-completed.json').write_text(json.dumps(value, indent=2))
        checks.append('two exact draft selections submit unchanged Unicode/multiline prompts to distinct overlapping voyages; detach does not cancel')

        pty = start()
        for query, reply in (('orchard-unique', server.prompts[alpha]), ('harbor-unique', server.prompts[beta])):
            pick(pty, query)
            expect(pty, reply)
        checks.append('reconnect and name-based F2 switching show the selected canonical replies')
        # F9 Actions is the entry point, not slash-command lifecycle shortcuts.
        pick(pty, 'orchard-unique')
        send(pty, F9)
        expect(pty, 'Branch')
        send(pty, DOWN * 2 + ENTER)
        expect(pty, 'Optional name')
        paste(pty, 'branch-unique')
        expect(pty, 'branch-unique')
        before = {e['session_id'] for e in catalogue()}
        send(pty, ENTER)
        def branched():
            new = [e['session_id'] for e in catalogue() if e['session_id'] not in before]
            assert len(new) <= 1, new
            return new[0] if new else None
        branch = wait_for(branched)
        branch_value = wait_for(lambda: snapshot(branch) if snapshot(branch).get('messages') else None)
        assert branch_value['messages'] == snapshot(sid_a)['messages'], 'branch changed canonical history'
        pick(pty, 'branch-unique')
        expect(pty, server.prompts[alpha])
        send(pty, F9)
        expect(pty, 'Archive')
        send(pty, DOWN + ENTER)
        wait_for(lambda: snapshot(branch).get('lifecycle', {}).get('archived') is True)
        assert not snapshot(sid_a).get('lifecycle', {}).get('archived'), 'archived parent instead of branch'
        assert not snapshot(sid_b).get('lifecycle', {}).get('archived'), 'archived peer instead of branch'
        (root / 'branch-archived.json').write_text(json.dumps(snapshot(branch), indent=2))
        checks.append('F9 Actions branches exact selected history then archives only the branch')

        pick(pty, 'orchard-unique')
        paste(pty, server.approval)
        send(pty, ENTER)
        def pending():
            value = snapshot(sid_a)
            return value['decisions'][0] if value.get('decisions') else None
        decision = wait_for(pending)
        assert decision['request']['kind'] == 'approval', decision
        expect(pty, 'Permission needed')
        paste(pty, 'approved\ny\n\r')
        # A bounded observation window proves paste did not act as consent.
        until = time.monotonic() + 1
        while time.monotonic() < until:
            assert pending()['decision_id'] == decision['decision_id']
            assert not (workspace / 'must-not-exist.txt').exists()
            time.sleep(.05)
        stop(pty)
        assert pending()['decision_id'] == decision['decision_id']
        pty = start()
        # A reconnect may already select the pending review; F2 correctly refuses
        # to steal focus from that review. Otherwise select its exact voyage.
        wait_for(lambda: 'Permission needed' in screen(pty) or 'First message' in screen(pty) or 'F2 Voyages' in screen(pty))
        if 'Permission needed' not in screen(pty):
            pick(pty, 'orchard-unique')
        expect(pty, 'Permission needed')
        assert pending()['decision_id'] == decision['decision_id']
        send(pty, ESC)  # Review cancellation is an explicit denial, never approval.
        value = finished(sid_a)
        assert not value.get('decisions')
        assert not (workspace / 'must-not-exist.txt').exists()
        tools = [m for m in value['messages'] if m['role'] == 'tool' and m.get('tool_call_id') == 'denied-write']
        assert len(tools) == 1 and tools[0]['tool_outcome']['execution'] != 'succeeded', tools
        assert [m['content'] for m in value['messages'] if m['role'] == 'user'] == [alpha, server.approval]
        assert server.counts == {alpha: 1, beta: 1, server.approval: 2}, server.counts
        (root / 'approval-denied.json').write_text(json.dumps(value, indent=2))
        checks.append('approval review ignores pasted consent; reconnect retains decision identity; Esc denies without file effect')
        # Idle built-in discovery and typed manual execution must not need a model turn.
        pick(pty, 'harbor-unique')
        (workspace / 'operator-input.txt').write_text('OPERATOR-READ-CANARY')
        prior = snapshot(sid_b)
        prior_run = prior['run']['run_id']
        send(pty, '\x1b[19~')  # F8
        expect(pty, 'Run a tool')
        send(pty, ENTER)
        expect(pty, 'Search actions:')
        paste(pty, 'read_file')
        send(pty, ENTER)
        expect(pty, 'path *:')
        paste(pty, 'operator-input.txt')
        send(pty, '\t' + ENTER)
        expect(pty, 'Review read_file')
        send(pty, '\x1b[6~' * 20)
        time.sleep(.2)
        send(pty, ENTER)
        def operator_done():
            value = snapshot(sid_b)
            if (value.get('run') or {}).get('run_id') == prior_run or value.get('pending_cleanup_run'):
                return None
            return value if any('OPERATOR-READ-CANARY' in m.get('content', '') for m in value['messages']) else None
        op_result = wait_for(operator_done)
        (root / 'operator-read.json').write_text(json.dumps(op_result, indent=2))
        assert server.counts == {alpha: 1, beta: 1, server.approval: 2}, server.counts
        checks.append('F8 idle built-in discovery and typed read_file execute without model inference or raw JSON input')

        # Exact digest preview and optional secret references, with no secret persistence.
        workflows = workspace / '.helm' / 'workflows'
        workflows.mkdir(parents=True)
        (workflows / 'ui-flow.toml').write_text('schema_version = 1\nid = "ui-flow"\nversion = "1"\ndescription = "UI workflow"\nprompt = "WORKFLOW-UI {{word}} {{token}}"\n[parameters.token]\ntype = "string"\nsecret = true\n[parameters.word]\ntype = "string"\ndefault = "ready"\n')
        send(pty, '\x1b[19~')
        expect(pty, 'Saved workflows')
        send(pty, DOWN * 4 + ENTER)
        expect(pty, 'ui-flow')
        send(pty, ENTER)
        expect(pty, 'Definition (untrusted content)')
        send(pty, '\x1b[6~' * 20)
        time.sleep(.2)
        send(pty, 't')
        expect(pty, 'PRIVATE')
        paste(pty, 'private-ui-sentinel-14')
        assert 'private-ui-sentinel-14' not in screen(pty)
        send(pty, ENTER)
        expect(pty, 'word')
        send(pty, ENTER)
        expect(pty, 'WORKFLOW-UI ready')
        send(pty, '\x1b[6~' * 20)
        time.sleep(.2)
        send(pty, 'y')
        wait_for(lambda: any('WORKFLOW-UI-COMPLETED' in m.get('content', '') for m in snapshot(sid_b)['messages']))
        completed_workflow = finished(sid_b)
        (root / 'workflow-completed.json').write_text(json.dumps(completed_workflow, indent=2))
        stop(pty)
        for path in root.rglob('*'):
            if path.is_file():
                assert b'private-ui-sentinel-14' not in path.read_bytes(), f'private workflow value persisted: {path}'
        assert sum(count for prompt, count in server.counts.items() if prompt.startswith('WORKFLOW-UI ')) == 1
        checks.append('F8 workflow named selection, exact digest trust/host preview, optional masked secret and single submission; private canary absent from provider requests and persisted files')
        assert not server.errors, server.errors
    finally:
        server.release.set()
        for pty in ptys:
            try:
                stop(pty)
            except Exception as error:
                cleanup_errors.append('Helm: ' + repr(error))
        try:
            signal_owned(signal.SIGTERM)
            wait_for(lambda: not owned_pids(), timeout=10)
        except Exception as error:
            cleanup_errors.append('Voyage graceful cleanup: ' + repr(error))
            signal_owned(signal.SIGKILL)
            try:
                wait_for(lambda: not owned_pids(), timeout=10)
            except Exception as error:
                cleanup_errors.append('Voyage forced cleanup: ' + repr(error))
        if supervisor is not None:
            supervisor.terminate()
            try:
                supervisor.wait(timeout=10)
            except subprocess.TimeoutExpired:
                cleanup_errors.append('Vessel required SIGKILL')
                supervisor.kill()
                supervisor.wait(timeout=10)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        log.close()
        cleanup = dict(errors=cleanup_errors, remaining_owned_voyage_pids=owned_pids(),
                       supervisor_returncode=supervisor.poll() if supervisor else None,
                       helm_returncodes=[p['process'].poll() for p in ptys],
                       provider_thread_alive=thread.is_alive())
        (root / 'cleanup.json').write_text(json.dumps(cleanup, indent=2))
        (root / 'checks.json').write_text(json.dumps(checks, indent=2))
        (root / 'provider-requests.json').write_text(json.dumps(server.requests, indent=2))
        (root / 'provider-errors.json').write_text(json.dumps(server.errors, indent=2))
    assert not cleanup_errors and not cleanup['remaining_owned_voyage_pids'] and not thread.is_alive(), cleanup
    print('PASS: ' + '; '.join(checks))


if __name__ == '__main__':
    main()
