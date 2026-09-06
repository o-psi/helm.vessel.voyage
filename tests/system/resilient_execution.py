#!/usr/bin/env python3
"""Offline native-provider effect identity and bounded retry acceptance."""
import json
import os
from pathlib import Path
import subprocess
import signal
import time
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import event, response

HELM = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()


def calls_response(provider, calls):
    if provider == 'openai-chat':
        return event({'choices': [{'delta': {'tool_calls': [
            {'index': i, 'id': call['id'], 'type': 'function', 'function': {'name': call['name'], 'arguments': json.dumps(call['arguments'])}}
            for i, call in enumerate(calls)]}}], 'usage': {'prompt_tokens': 3, 'completion_tokens': 2}}) + b'data: [DONE]\n\n'
    if provider == 'openai-responses':
        return event({'type': 'response.completed', 'response': {'output': [
            {'type': 'function_call', 'id': 'item-'+str(i), 'call_id': call['id'], 'name': call['name'], 'arguments': json.dumps(call['arguments'])}
            for i, call in enumerate(calls)], 'usage': {'input_tokens': 3, 'output_tokens': 2}}})
    frames = [event({'type': 'message_start', 'message': {'usage': {'input_tokens': 3}}})]
    for i, call in enumerate(calls):
        frames += [event({'type': 'content_block_start', 'index': i, 'content_block': {'type': 'tool_use', 'id': call['id'], 'name': call['name']}}),
                   event({'type': 'content_block_delta', 'index': i, 'delta': {'type': 'input_json_delta', 'partial_json': json.dumps(call['arguments'])}})]
    return b''.join(frames) + event({'type': 'message_delta', 'usage': {'output_tokens': 2}}) + event({'type': 'message_stop'})


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if not self.path.startswith('/v1/models/'):
            self.send_error(404)
            return
        payload = json.dumps({'id': self.path.rsplit('/', 1)[-1], 'max_tokens': 131072}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_POST(self):
        case = self.server.case
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        case['requests'].append(body)
        step = len(case['requests'])
        case['times'].append(time.monotonic())
        if step > 1 and case['mode'] == 'duplicate':
            if case['provider'] == 'openai-responses':
                prior = [item for item in body['input'] if item.get('type') == 'function_call']
            elif case['provider'] == 'openai-chat':
                prior = [call for message in body['messages'] for call in message.get('tool_calls', [])]
            else:
                prior = [item for message in body['messages'] if isinstance(message.get('content'), list) for item in message['content'] if item.get('type') == 'tool_use']
            if len(prior) != 1:
                self.send_error(400, 'duplicate call survived canonical provider continuation')
                return
        if case['mode'] in ['retry', 'retry-after', 'retry-too-long', 'cancel-retry']:
            failures = 2 if case['mode'] == 'retry' else 1
            if step <= failures:
                self.send_response(429)
                if case['mode'] != 'retry':
                    self.send_header('Retry-After', '60' if case['mode'] == 'retry-too-long' else '1')
                data = b'{"error":{"message":"offline throttle"}}'
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                case['first_response'].set()
                return
            data = response(case['provider'], 'resilience-done', step)
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        call = {'id': 'reused-call', 'name': 'shell', 'arguments': {'command': "printf 'effect\\n' >> effects"}}
        if case['mode'] == 'timeout':
            call['arguments']['command'] += '; sleep 30'
        if step == 1:
            other = dict(call)
            if case['mode'] == 'conflict-arguments':
                other['arguments'] = {'command': "printf 'conflict\\n' >> effects"}
            elif case['mode'] == 'conflict-name':
                other['name'] = 'process'
            elif case['mode'] == 'distinct':
                other['id'] = 'another-call'
            calls = [call] if case['mode'] == 'fresh' else [call, other]
            data = calls_response(case['provider'], calls)
        elif case['mode'] == 'fresh' and step == 2:
            data = calls_response(case['provider'], [call])
        else:
            data = response(case['provider'], 'resilience-done', step)
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


def main():
    for provider in ['openai-chat', 'openai-responses', 'anthropic']:
        for mode in ['duplicate', 'conflict-arguments', 'conflict-name', 'distinct', 'fresh', 'retry', 'retry-after', 'retry-too-long', 'cancel-retry', 'timeout', 'denied']:
            with tempfile.TemporaryDirectory(prefix='helm-resilient-') as directory:
                root = Path(directory)
                case = {'provider': provider, 'mode': mode, 'requests': [], 'times': [], 'first_response': threading.Event()}
                server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
                server.case = case
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    config = root/'provider.toml'
                    config.write_text(f'provider="{provider}"\nmodel="resilience-model"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="RESILIENCE_KEY"\naccess="{"read-only" if mode == "denied" else "unrestricted"}"\ncommand_timeout_secs=1\nprovider_retry_attempts=4\nprovider_retry_initial_ms=200\nprovider_retry_max_ms=2000\n')
                    env = dict(os.environ, HOME=str(root/'home'), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), RESILIENCE_KEY='offline-fixture')
                    command = [str(HELM), '--config', str(config), '--workspace', str(root), 'run', 'exercise '+mode]
                    if mode == 'cancel-retry':
                        process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                        try:
                            assert case['first_response'].wait(10)
                            time.sleep(0.03)
                            process.send_signal(signal.SIGINT)
                            stdout, stderr = process.communicate(timeout=15)
                            result = subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
                        finally:
                            if process.poll() is None:
                                process.kill(); process.wait(5)
                    else:
                        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=15)
                    effects = (root/'effects').read_text().splitlines() if (root/'effects').exists() else []
                    if mode in ['retry', 'retry-after', 'retry-too-long', 'cancel-retry']:
                        if mode in ['retry-too-long', 'cancel-retry']:
                            assert result.returncode != 0 and len(case['requests']) == 1, (provider, mode, result, case['times'])
                        else:
                            assert result.returncode == 0, (provider, mode, result.stderr)
                            assert len(case['requests']) == (3 if mode == 'retry' else 2)
                            if mode == 'retry-after':
                                assert case['times'][1]-case['times'][0] >= 1, case['times']
                        assert not effects
                    elif mode.startswith('conflict'):
                        assert result.returncode != 0 and not effects, (provider, mode, result.returncode, effects, result.stderr)
                        assert len(case['requests']) == 1
                        records = list((root/'data/helm/sessions').glob('*.json'))
                        assert len(records) == 1
                        saved = json.loads(records[0].read_text())
                        assert saved['usage'] == {'input_tokens': 3, 'output_tokens': 2}, saved
                        assert not any(message.get('tool_calls') for message in saved['messages']), saved
                    else:
                        expected = 1 if mode in ['duplicate', 'timeout', 'denied'] else 2
                        assert result.returncode == 0, (provider, mode, result.stderr)
                        assert effects == ([] if mode == 'denied' else ['effect']*expected), (provider, mode, effects)
                        records = list((root/'data/helm/sessions').glob('*.json'))
                        assert len(records) == 1
                        saved = json.loads(records[0].read_text())
                        calls = [call for message in saved['messages'] for call in message.get('tool_calls', [])]
                        results = [message for message in saved['messages'] if message['role'] == 'tool']
                        assert len(calls) == expected and len(results) == expected, (provider, mode, saved)
                        if mode in ['timeout', 'denied']:
                            assert all(message['tool_success'] is False for message in results), results
                        if mode == 'fresh':
                            case['requests'].clear(); case['times'].clear()
                            resumed = subprocess.run([*command, '--resume', saved['id']], env=env, capture_output=True, text=True, timeout=15)
                            assert resumed.returncode == 0, (provider, resumed.stderr)
                            assert (root/'effects').read_text().splitlines() == ['effect']*4, 'historical IDs suppressed a fresh run'
                finally:
                    server.shutdown()
                    server.server_close()
                    thread.join(5)
    print('resilient execution: response-scoped identity, fresh calls across requests/restart, bounded retries, server delays and cancellation passed through all three native providers')


if __name__ == '__main__':
    main()
