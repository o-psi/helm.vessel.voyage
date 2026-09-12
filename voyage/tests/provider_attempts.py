"""Focused #261 offline native retry/history/continuation checks; no live provider."""
import argparse
import http.server
import json
from pathlib import Path
import threading
import time
import uuid

from delivery_recovery import Fixture, wait_for

SECRET = 'PRIVATE_PROVIDER_DIAGNOSTIC_261'
PARTIAL = 'Saved partial answer before interruption. '
ANSWER = 'Continuation completed with retained history.'
CASES = ('connection', 'recover', 'exhaust', 'auth', 'quota', 'request', 'long_wait', 'text', 'tool', 'eof', 'cancel', 'silent_headers', 'silent_stream', 'text_idle', 'tool_idle', 'heartbeat', 'cancel_stream')
MODES = {'responses': 'openai-responses', 'chat': 'openai-chat', 'anthropic': 'anthropic'}


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
            if case == 'success' or (case == 'recover' and step == 2):
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
    fixture.sessions.append(sid)
    config = fixture.root / (sid + '.toml')
    config.write_text(f'provider = "{MODES[mode]}"\nmodel = "fixture-model"\n'
        f'base_url = "http://127.0.0.1:{0 if case == "connection" else fixture.provider.server_port}/v1"\n'
        f'api_key_required = {str(mode == "anthropic").lower()}\naccess = "unrestricted"\nmax_tokens = 1024\n'
        'provider_retry_attempts = 3\nprovider_retry_initial_ms = 10\n'
        f'provider_retry_max_ms = {3000 if case == "cancel" else 40}\n'
        'provider_retry_elapsed_ms = 10000\ncommand_timeout_secs = 2\n'
        f'provider_response_timeout_ms = {100 if case == "silent_headers" else 3000}\n'
        f'provider_stream_idle_ms = {100 if case != "cancel_stream" else 3000}\n')
    config.chmod(0o600)
    fixture.request({'op': 'start_configured', 'session_id': sid, 'command_id': str(uuid.uuid4()),
        'workspace': str(fixture.workspace), 'config_path': str(config)})
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
    args = parser.parse_args()
    fixture = Fixture(args.bin_dir.resolve())
    fixture.env["ANTHROPIC_API_KEY"] = SECRET
    fixture.provider.RequestHandlerClass = Handler
    fixture.provider.errors = []
    fixture.provider.times = []
    try:
        fixture.start()
        for mode in MODES:
            for case in CASES:
                fixture.provider.bodies = []
                fixture.provider.times = []
                fixture.provider.scenario = (mode, case)
                sid = session(fixture, mode, case)
                original = fixture.submit(sid, 'Remember lighthouse and do the task.')
                receipt = fixture.command(sid, original)
                assert receipt['status'] == 'accepted', receipt
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
                count = {'connection': 3, 'recover': 2, 'exhaust': 3, 'silent_headers': 3, 'silent_stream': 3, 'heartbeat': 3}.get(case, 1)
                assert len(attempts) == count, (case, attempts)
                assert len(fixture.provider.bodies) == (0 if case == 'connection' else count), (case, attempts)
                decision = {'connection': 'attempts_exhausted', 'recover': 'completed', 'exhaust': 'attempts_exhausted',
                    'long_wait': 'server_delay_limit', 'text': 'partial_response',
                    'tool': 'partial_response', 'text_idle': 'partial_response', 'tool_idle': 'partial_response', 'cancel': 'cancelled', 'cancel_stream': 'cancelled', 'silent_headers': 'attempts_exhausted', 'silent_stream': 'attempts_exhausted', 'heartbeat': 'attempts_exhausted'}.get(case, 'non_retryable')
                last = attempts[-1]['attempt']
                assert last['decision'] == decision, last
                assert SECRET not in json.dumps(saved) + json.dumps(attempts)
                assert len({a['attempt']['attempt_id'] for a in attempts}) == count
                assert len({a['attempt']['request_id'] for a in attempts}) == 1
                assert not any(m['role'] == 'tool' for m in saved['messages'])
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
                assert len(fixture.provider.bodies) == (0 if case == 'connection' else count)
                # Explicit continuation is a new command, retaining the old run and history.
                if case in ('text', 'tool'):
                    fixture.provider.scenario = (mode, 'success')
                    next_receipt = fixture.command(sid, fixture.submit(sid, 'Continue using the retained lighthouse task.'))
                    assert next_receipt['run_id'] != receipt['run_id']
                    continued = fixture.finished(sid)
                    assert continued['run']['state'] == 'completed', continued['run']
                    assert continued['messages'][:len(saved['messages'])] == saved['messages']
                    assert 'lighthouse' in json.dumps(fixture.provider.bodies[-1])
                    assert len(history(fixture, sid, receipt['run_id'])) == 1
                    fixture.suspended(sid)
                if mode == 'responses' and case == 'exhaust':
                    helm_attempts(fixture, sid, receipt['run_id'])
                    assert len(fixture.provider.bodies) == count
                fixture.record(mode + '-' + case, {'run': saved['run'], 'attempts': attempts})
                assert not fixture.provider.errors, fixture.provider.errors
    finally:
        fixture.close()
    print('PASS: 51 native failure/retry/history scenarios, duplicate receipts, explicit continuation and observed cleanup')


if __name__ == '__main__':
    main()
