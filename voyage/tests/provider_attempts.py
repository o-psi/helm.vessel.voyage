"""Focused #261/#269 offline native retry/history/continuation checks; no live provider."""
import argparse
import http.server
import json
import subprocess
import tomllib
from pathlib import Path
import threading
import time
import uuid

from delivery_recovery import Fixture as BaseFixture, wait_for

SECRET = 'PRIVATE_PROVIDER_DIAGNOSTIC_261'
PARTIAL = 'Saved partial answer before interruption. '
ANSWER = 'Continuation completed with retained history.'
CASES = ('connection', 'recover', 'exhaust', 'auth', 'quota', 'request', 'long_wait', 'text', 'tool', 'eof', 'cancel', 'silent_headers', 'silent_stream', 'text_idle', 'tool_idle', 'heartbeat', 'cancel_stream', 'activity', 'recover_text', 'recover_tool', 'recover_after_tool', 'connection_recover')
MODES = {'responses': 'openai-responses', 'chat': 'openai-chat', 'anthropic': 'anthropic'}


class Fixture(BaseFixture):
    def __init__(self, binaries):
        super().__init__(binaries)
        self.env['PROVIDER_FIXTURE_KEY'] = SECRET
        self.observation_retries = 0

    def command(self, session, command, allow_error=False):
        if allow_error or command['op'] not in ('snapshot', 'provider_attempts'):
            return super().command(session, command, allow_error)
        # Retry positive read observations only. Deliberate invalid-page/revision
        # probes must return their refusal immediately; never replay mutations.
        deadline = time.monotonic() + 10
        while True:
            response = super().command(session, command, allow_error=True)
            if (response.get('error') == 'suspended observation unavailable'
                    and time.monotonic() < deadline):
                self.observation_retries += 1
                time.sleep(.05)
                continue
            assert response.get('error') is None, response
            return response['result']


def frames(mode, partial=None):
    if mode == 'responses':
        if partial == 'text':
            return [{'type': 'response.output_text.delta', 'delta': PARTIAL}]
        if partial == 'tool':
            return [{'type': 'response.output_item.added', 'output_index': 0, 'item': {
                'type': 'function_call', 'call_id': 'unexecuted', 'name': 'write_file', 'arguments': ''}},
                {'type': 'response.function_call_arguments.delta', 'output_index': 0, 'delta': '{'}]
        return [{'type': 'response.completed', 'response': {'id': 'response261', 'status': 'completed',
            'output': [{'type': 'message', 'role': 'assistant', 'content': [
                {'type': 'output_text', 'text': ANSWER}]}], 'usage': {'input_tokens': 1, 'output_tokens': 1}}}]
    if mode == 'chat':
        delta = {'content': PARTIAL if partial else ANSWER}
        if partial == 'tool':
            delta = {'tool_calls': [{'index': 0, 'id': 'unexecuted', 'type': 'function',
                                    'function': {'name': 'write_file', 'arguments': '{'}}]}
        result = [{'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]}]
        return result if partial else result + [{'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]}]
    result = [{'type': 'message_start', 'message': {'id': 'message261', 'role': 'assistant',
        'content': [], 'usage': {'input_tokens': 1, 'output_tokens': 0}}}]
    if partial == 'tool':
        return result + [{'type': 'content_block_start', 'index': 0, 'content_block': {
            'type': 'tool_use', 'id': 'unexecuted', 'name': 'write_file', 'input': {}}}]
    result += [{'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
        {'type': 'content_block_delta', 'index': 0, 'delta': {
            'type': 'text_delta', 'text': PARTIAL if partial else ANSWER}}]
    return result if partial else result + [{'type': 'content_block_stop', 'index': 0},
        {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn'}, 'usage': {'output_tokens': 1}},
        {'type': 'message_stop'}]


def tool_frames(mode):
    arguments = {'path': 'recovery-effect.txt', 'content': 'Recorded exactly once.'}
    if mode == 'responses':
        return [{'type': 'response.completed', 'response': {'id': 'tool269', 'status': 'completed',
            'output': [{'type': 'function_call', 'call_id': 'completed269', 'name': 'write_file',
                'arguments': json.dumps(arguments)}], 'usage': {'input_tokens': 1, 'output_tokens': 1}}}]
    if mode == 'chat':
        return [{'choices': [{'index': 0, 'delta': {'tool_calls': [{'index': 0,
            'id': 'completed269', 'type': 'function', 'function': {'name': 'write_file',
            'arguments': json.dumps(arguments)}}]}, 'finish_reason': None}]},
            {'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls'}]}]
    return [{'type': 'message_start', 'message': {'id': 'tool269', 'role': 'assistant',
        'content': [], 'usage': {'input_tokens': 1, 'output_tokens': 0}}},
        {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'tool_use',
            'id': 'completed269', 'name': 'write_file', 'input': {}}},
        {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'input_json_delta',
            'partial_json': json.dumps(arguments)}},
        {'type': 'content_block_stop', 'index': 0},
        {'type': 'message_delta', 'delta': {'stop_reason': 'tool_use'}, 'usage': {'output_tokens': 1}},
        {'type': 'message_stop'}]


def activity_frame(mode):
    if mode == 'responses':
        return {'type': 'response.reasoning_summary_text.delta', 'delta': 'Synthetic reasoning activity.'}
    if mode == 'chat':
        return {'choices': [{'index': 0, 'delta': {'reasoning_content': 'Synthetic reasoning activity.'},
            'finish_reason': None}]}
    return {'type': 'ping'}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            self.server.bodies.append(body)
            self.server.times.append(time.monotonic())
            mode, case = self.server.scenario
            step = len(self.server.bodies)
            assert body['stream'] is True
            if case == 'activity':
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('x-request-id', 'req261')
                self.end_headers()
                for _ in range(12):
                    self.wfile.write(('data: ' + json.dumps(activity_frame(mode)) + '\n\n').encode())
                    self.wfile.flush()
                    time.sleep(.04)
                for value in frames(mode):
                    self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
                self.wfile.flush()
                self.close_connection = True
                return
            if case == 'recover_after_tool' and step == 1:
                self.stream(tool_frames(mode))
                return
            if case in ('recover_text', 'recover_tool', 'recover_after_tool'):
                interrupted_step = 2 if case == 'recover_after_tool' else 1
                if case == 'recover_after_tool':
                    effect = self.server.workspace / 'recovery-effect.txt'
                    assert effect.read_text() == 'Recorded exactly once.'
                    if step == 2:
                        self.server.effect_mtime = effect.stat().st_mtime_ns
                    else:
                        assert effect.stat().st_mtime_ns == self.server.effect_mtime
                self.stream(frames(mode, ('tool' if case == 'recover_tool' else 'text')
                    if step == interrupted_step else None), incomplete=step == interrupted_step)
                return
            if case == 'silent_headers':
                time.sleep(1)
                return
            if case in ('silent_stream', 'text_idle', 'tool_idle', 'heartbeat', 'cancel_stream'):
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('x-request-id', 'req261')
                self.send_header('Content-Length', '100000')
                self.end_headers()
                if case in ('text_idle', 'tool_idle'):
                    for value in frames(mode, case.split('_')[0]):
                        self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
                    self.wfile.flush()
                for _ in range(50):
                    if case == 'heartbeat':
                        self.wfile.write(b': heartbeat\n\n')
                        self.wfile.flush()
                    time.sleep(.02)
                return
            if case in ('success', 'connection_recover') or (case == 'recover' and step == 2):
                self.stream(frames(mode))
                return
            if case in ('text', 'tool'):
                self.stream(frames(mode, case), incomplete=True)
                return
            if case == 'eof':
                self.stream([])
                return
            status = {'auth': 401, 'quota': 429, 'request': 400}.get(case, 503)
            payload = json.dumps({'error': {'message': SECRET, 'code':
                'insufficient_quota' if case == 'quota' else 'unknown_code_261'}}).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('x-request-id', 'req261')
            self.send_header('Content-Length', str(len(payload)))
            if case in ('long_wait', 'cancel'):
                self.send_header('Retry-After', '60' if case == 'long_wait' else '2')
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            self.server.errors.append(repr(error))

    def stream(self, values, incomplete=False):
        payload = ''.join('data: ' + json.dumps(v) + '\n\n' for v in values).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('x-request-id', 'req261')
        # Truncated HTTP body is an actual transport failure after the emitted deltas.
        self.send_header('Content-Length', str(len(payload) + (100 if incomplete else 0)))
        self.end_headers()
        self.wfile.write(payload)
        self.wfile.flush()
        self.close_connection = True


def session(fixture, mode, case):
    sid = str(uuid.uuid4())
    config = fixture.root / (sid + '.toml')
    config.write_text(f'provider = "{MODES[mode]}"\nmodel = "fixture-model"\n'
        f'base_url = "http://127.0.0.1:{0 if case == "connection" else fixture.provider.server_port}/v1"\n'
        f'api_key_required = {str(mode == "anthropic").lower()}\naccess = "unrestricted"\nmax_tokens = 1024\n'
        f'provider_retry_attempts = 3\nprovider_retry_initial_ms = {2000 if case == "connection_recover" else 10}\n'
        f'provider_retry_max_ms = {3000 if case in ("cancel", "connection_recover") else 40}\n'
        'provider_retry_elapsed_ms = 10000\ncommand_timeout_secs = 2\n'
        f'provider_response_timeout_ms = {100 if case == "silent_headers" else 3000}\n'
        f'provider_stream_idle_ms = {100 if case != "cancel_stream" else 3000}\n')
    settings = tomllib.loads(config.read_text())
    config.write_text(json.dumps({'version': 1, 'workspace': str(fixture.workspace),
        'config': settings, 'explicit': {'access': 'unrestricted'},
        'selection': None, 'confirmation': None}))
    config.chmod(0o600)
    # Creation now requires an explicit named account. All credentials/endpoints
    # remain synthetic and local; never inherit the operator's default account.
    def account_cli(*args):
        result = subprocess.run([str(fixture.binaries / 'vessel'), 'auth', 'accounts', *args],
            env=fixture.env, cwd=fixture.workspace, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)
    connection = account_cli('connect', '--label', sid, '--endpoint',
        f'http://127.0.0.1:{0 if case == "connection" else fixture.provider.server_port}/v1',
        '--transports', MODES[mode])
    account = account_cli('add', '--connection', connection['id'], '--account', sid,
        '--env', 'PROVIDER_FIXTURE_KEY')
    binding = {'account_id': account['id'], 'connection_id': connection['id'],
        'identity_generation': account['identity_generation'],
        'connection_revision': connection['revision'], 'transport': MODES[mode].replace('-', '_')}
    if not getattr(fixture, 'provider_default_set', False):
        fixture.request({'op': 'account_set_default', 'command_id': str(uuid.uuid4()),
            'workspace': str(fixture.workspace), 'account': binding, 'expected_revision': 0})
        fixture.provider_default_set = True
    launch = json.loads(config.read_text())
    launch['config']['account'] = binding
    config.write_text(json.dumps(launch))
    fixture.request({'op': 'start_settings', 'session_id': sid, 'command_id': str(uuid.uuid4()),
        'workspace': str(fixture.workspace), 'config_path': str(config),
        'binding': binding, 'settings': {}})
    fixture.sessions.append(sid)
    return sid


def history(fixture, sid, run=None):
    out, offset, revision = [], 0, None
    while True:
        page = fixture.command(sid, {'op': 'provider_attempts', 'run_id': run,
            'offset': offset, 'limit': 2, 'expected_revision': revision})
        revision = page['revision']
        out.extend(page['attempts'])
        if not page['has_more']:
            return out
        offset = page['next_offset']


def helm_attempts(fixture, sid, run_id):
    from ui_journeys import launch, screen, send
    from images_composer import stop_pty
    env = {**fixture.env, 'TERM':'xterm-256color'}
    ui = launch([str(fixture.binaries / 'helm'), 'connect', '--directory', str(fixture.directory), '--no-start'], env, fixture.workspace, fixture.root / 'attempts.pty', 100, 32)
    try:
        wait_for(lambda: 'F2' in screen(ui), timeout=15)
        send(ui, '/use ' + sid + '\r')
        time.sleep(.3)
        send(ui, '/attempts ' + run_id + '\r')
        wait_for(lambda: 'Provider attempts' in screen(ui), timeout=10)
        shown = screen(ui)
        assert 'req261' in shown, shown
        assert 'attempt limit reached' in shown, shown
        assert SECRET not in bytes(ui['output']).decode(errors='replace')
        fixture.record('helm-attempt-history-pty', {'screen':shown})
    finally:
        stop_pty(ui, wait_for)
        assert ui['process'].returncode == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--mode', choices=tuple(MODES))
    parser.add_argument('--case', choices=CASES)
    args = parser.parse_args()
    modes = [args.mode] if args.mode else MODES
    cases = [args.case] if args.case else CASES
    # Each adverse case owns a fresh supervisor/account registry. This matrix
    # verifies provider recovery; multi-session catalogue stress is a separate check.
    for mode in modes:
        for case in cases:
            fixture = Fixture(args.bin_dir.resolve())
            fixture.env["ANTHROPIC_API_KEY"] = SECRET
            deferred_connection = case == 'connection_recover'
            if deferred_connection:
                fixture.provider.shutdown()
                fixture.provider.server_close()
                fixture.thread.join(timeout=5)
                fixture.provider = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler,
                    bind_and_activate=False)
                # Reserve the endpoint without listening. It refuses connections
                # until the first durable recovery observation permits our fixture
                # to restore service at exactly that address.
                fixture.provider.server_bind()
                fixture.thread = threading.Thread(target=fixture.provider.serve_forever, daemon=True)
            fixture.provider.RequestHandlerClass = Handler
            fixture.provider.errors = []
            fixture.provider.times = []
            fixture.provider.workspace = fixture.workspace
            try:
                fixture.start()
                fixture.provider.bodies = []
                fixture.provider.times = []
                fixture.provider.scenario = (mode, case)
                sid = session(fixture, mode, case)
                original = fixture.submit(sid, 'Remember lighthouse and do the task.')
                receipt = fixture.command(sid, original)
                assert receipt['status'] == 'accepted', receipt
                if case == 'connection_recover':
                    def refused():
                        snapshot = fixture.command(sid, {'op': 'snapshot'})
                        rows = snapshot['run']['provider_attempts']
                        return rows if rows and rows[-1]['category'] == 'connection' and rows[-1]['decision'] == 'retry_scheduled' else None
                    refused_rows = wait_for(refused)
                    assert len(refused_rows) == 1 and not fixture.provider.bodies, refused_rows
                    fixture.provider.server_activate()
                    fixture.thread.start()
                    deferred_connection = False
                if case in ('cancel', 'cancel_stream'):

                    def backoff():
                        s = fixture.command(sid, {'op': 'snapshot'})
                        return s if s['run']['provider_attempts'] and (s['run']['provider_attempts'][-1]['decision'] == 'retry_scheduled' if case == 'cancel' else s['run']['provider_attempts'][-1]['phase'] == 'stream') else None
                    saved = wait_for(backoff)
                    fixture.command(sid, {'op': 'cancel', 'command_id': str(uuid.uuid4()),
                        'run_id': receipt['run_id'], 'expected_revision': saved['revision'],
                        'expires_at_ms': int(time.time()*1000) + 60000})
                saved = fixture.finished(sid)
                fixture.suspended(sid)
                attempts = history(fixture, sid, receipt['run_id'])
                count = {'connection': 3, 'recover': 2, 'exhaust': 3, 'silent_headers': 3, 'silent_stream': 3, 'heartbeat': 3, 'text': 3, 'tool': 3, 'text_idle': 3, 'tool_idle': 3, 'eof': 3, 'recover_text': 2, 'recover_tool': 2, 'recover_after_tool': 3, 'connection_recover': 2}.get(case, 1)
                received_count = 0 if case == 'connection' else count - (case == 'connection_recover')
                assert len(attempts) == count, (case, attempts)
                assert len(fixture.provider.bodies) == received_count, (case, attempts)
                decision = {'connection': 'attempts_exhausted', 'recover': 'completed', 'exhaust': 'attempts_exhausted',
                    'long_wait': 'server_delay_limit', 'text': 'attempts_exhausted',
                    'tool': 'attempts_exhausted', 'text_idle': 'attempts_exhausted', 'tool_idle': 'attempts_exhausted', 'cancel': 'cancelled', 'cancel_stream': 'cancelled', 'silent_headers': 'attempts_exhausted', 'silent_stream': 'attempts_exhausted', 'heartbeat': 'attempts_exhausted', 'eof': 'attempts_exhausted', 'activity': 'completed', 'recover_text': 'completed', 'recover_tool': 'completed', 'recover_after_tool': 'completed', 'connection_recover': 'completed'}.get(case, 'non_retryable')
                last = attempts[-1]['attempt']
                assert last['decision'] == decision, last
                assert SECRET not in json.dumps(saved) + json.dumps(attempts)
                assert len({a['attempt']['attempt_id'] for a in attempts}) == count
                continuing = case in ('text', 'tool', 'text_idle', 'tool_idle', 'recover_text', 'recover_tool', 'recover_after_tool')
                rows = [a['attempt'] for a in attempts]
                assert len({a['request_id'] for a in rows}) == (count if continuing else 1)
                recovery_rows = rows[1:] if case == 'recover_after_tool' else rows
                assert [a['attempt'] for a in recovery_rows] == list(range(1, len(recovery_rows) + 1))
                if continuing:
                    for previous, current in zip(recovery_rows, recovery_rows[1:]):
                        assert previous['decision'] == 'continuation_scheduled', previous
                        assert current['retry']['recovery_of'] == previous['attempt_id'], current
                        assert current['retry']['recovery_deadline_at_ms'] == previous['retry']['recovery_deadline_at_ms']
                    segments = [m for m in saved['messages'] if m.get('interrupted_attempt')]
                    assert len(segments) == len(recovery_rows) - 1, saved['messages']
                    assert [m['interrupted_attempt'] for m in segments] == [a['attempt_id'] for a in recovery_rows[:-1]]
                    assert all(not m.get('tool_calls') for m in segments)
                    if case not in ('tool', 'tool_idle', 'recover_tool'):
                        assert all(m['content'] == PARTIAL for m in segments), segments
                        assert PARTIAL in json.dumps(fixture.provider.bodies[-1])
                tool_results = [m for m in saved['messages'] if m['role'] == 'tool']
                if case == 'recover_after_tool':
                    assert len(tool_results) == 1 and tool_results[0]['tool_call_id'] == 'completed269', tool_results
                    assert 'completed269' in json.dumps(fixture.provider.bodies[-1])
                    assert (fixture.workspace / 'recovery-effect.txt').stat().st_mtime_ns == fixture.provider.effect_mtime
                else:
                    assert not tool_results
                if decision == 'completed':
                    assert saved['run']['state'] == 'completed', saved['run']
                    assert saved['messages'][-1]['content'] == ANSWER
                    assert not saved['messages'][-1].get('interrupted_attempt')
                if case == 'activity':
                    assert rows[0]['duration_ms'] >= 400, rows
                    assert 'Synthetic reasoning activity.' not in json.dumps(saved['messages'])
                if case not in ('connection', 'silent_headers'):
                    assert last['retry']['upstream_request_id'] == 'req261', last
                    assert last['http_status'] is not None, last
                if case in ('silent_headers', 'silent_stream', 'text_idle', 'tool_idle', 'heartbeat'):
                    assert last['category'] == 'timeout', last
                denied = fixture.command(sid, {'op':'provider_attempts', 'run_id':receipt['run_id'], 'offset':0, 'limit':33, 'expected_revision':None}, allow_error=True)
                assert denied.get('error'), denied
                if case == 'long_wait':
                    assert last['retry']['server_delay_ms'] == 60000
                    assert last['retry']['max_delay_ms'] == 40
                if case == 'quota':
                    assert last['retry']['provider_code'] == 'insufficient_quota'
                page = fixture.command(sid, {'op':'provider_attempts', 'run_id':receipt['run_id'], 'offset':0, 'limit':1, 'expected_revision':None})
                stale = fixture.command(sid, {'op':'provider_attempts', 'run_id':receipt['run_id'], 'offset':0, 'limit':1, 'expected_revision':page['revision'] + 1}, allow_error=True)
                assert stale.get('error'), stale
                replay = fixture.command(sid, original)
                assert replay['run_id'] == receipt['run_id']
                assert len(fixture.provider.bodies) == received_count
                # Explicit continuation is a new command, retaining the old run and history.
                if case in ('text', 'tool'):
                    fixture.provider.scenario = (mode, 'success')
                    next_receipt = fixture.command(sid, fixture.submit(sid, 'Continue using the retained lighthouse task.'))
                    assert next_receipt['run_id'] != receipt['run_id']
                    continued = fixture.finished(sid)
                    assert continued['run']['state'] == 'completed', continued['run']
                    assert continued['messages'][:len(saved['messages'])] == saved['messages']
                    assert 'lighthouse' in json.dumps(fixture.provider.bodies[-1])
                    assert len(history(fixture, sid, receipt['run_id'])) == count
                    fixture.suspended(sid)
                if mode == 'responses' and case == 'exhaust':
                    helm_attempts(fixture, sid, receipt['run_id'])
                    assert len(fixture.provider.bodies) == count
                fixture.record(mode + '-' + case, {'run': saved['run'], 'attempts': attempts,
                    'observation_retries': fixture.observation_retries})
                assert not fixture.provider.errors, fixture.provider.errors
            finally:
                if deferred_connection:
                    # Allow the common fixture cleanup to shut down/join this server
                    # even if setup or the durable-refusal assertion failed.
                    fixture.provider.server_activate()
                    fixture.thread.start()
                fixture.close()
    print(f'PASS: {len(modes) * len(cases)} selected native failure/retry/history scenarios and observed cleanup')


if __name__ == '__main__':
    main()
