"""Offline #74 supervised image workflow. Synthetic pixels and loopback provider only."""
import argparse
import base64
import http.server
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
import zlib


def wait_for(fn, timeout=30):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = fn()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError("runtime observation timed out")


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(b'\x00\xff\x00\x00\x00\xff\x00' * 2)) + chunk(b'IEND', b''))


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({'data': [{'id': getattr(self.server, 'model', 'gpt-4o'), 'input_modalities': ['text', 'image']}], 'has_more': False, 'max_tokens': 128}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.server.requests.append(body)
        if self.server.fail:
            payload = json.dumps({'error': {'message': 'PRIVATE-IMAGE-ECHO:' + self.server.image}}).encode()
            self.send_response(400)
            self.send_header('Content-Type', 'application/json')
        elif getattr(self.server, 'provider', 'openai-chat') in ('openai-responses', 'chatgpt-oauth'):
            result = {'type': 'response.completed', 'response': {'id': 'resp-' + str(len(self.server.requests)),
                'status': 'completed', 'output': [{'type': 'message', 'id': 'msg-' + str(len(self.server.requests)),
                'role': 'assistant', 'status': 'completed', 'content': [{'type': 'output_text', 'text': 'Image received.', 'annotations': []}]}],
                'usage': {'input_tokens': 1, 'output_tokens': 1}}}
            payload = ('data: ' + json.dumps(result) + '\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
        elif getattr(self.server, 'provider', 'openai-chat') == 'anthropic':
            events = [
                {'type': 'message_start', 'message': {'id': 'msg-' + str(len(self.server.requests)), 'type': 'message',
                    'role': 'assistant', 'content': [], 'usage': {'input_tokens': 1, 'output_tokens': 0}}},
                {'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'text', 'text': ''}},
                {'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'text_delta', 'text': 'Image received.'}},
                {'type': 'content_block_stop', 'index': 0},
                {'type': 'message_delta', 'delta': {'stop_reason': 'end_turn'}, 'usage': {'output_tokens': 1}},
                {'type': 'message_stop'}]
            payload = ''.join('data: ' + json.dumps(event) + '\n\n' for event in events).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
        else:
            payload = ('data: ' + json.dumps({'choices': [{'index': 0, 'delta': {'content': 'Image received.'}, 'finish_reason': None}]})
                       + '\n\ndata: ' + json.dumps({'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]})
                       + '\n\ndata: [DONE]\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)


def normalized_messages(request):
    result = []
    for message in request.get('messages', request.get('input', [])):
        content = message.get('content')
        if isinstance(content, list):
            blocks = []
            for block in content:
                kind = block['type']
                if kind in ('text', 'input_text', 'output_text'):
                    blocks.append({'type': 'text', 'text': block['text']})
                elif kind == 'image_url':
                    blocks.append(block)
                elif kind == 'input_image':
                    blocks.append({'type': 'image_url', 'image_url': {'url': block['image_url']}})
                elif kind == 'image':
                    assert block['source']['type'] == 'base64'
                    source = block['source']
                    blocks.append({'type': 'image_url', 'image_url': {'url': 'data:' + source['media_type'] + ';base64,' + source['data']}})
                else:
                    raise AssertionError('unexpected native content type: ' + kind)
            message = {**message, 'content': blocks}
        result.append(message)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--provider', choices=['openai-chat', 'openai-responses', 'anthropic', 'chatgpt-oauth'], default='openai-chat')
    args = parser.parse_args()
    binary = args.bin_dir.resolve()
    root = Path(tempfile.mkdtemp(prefix='voyage-images-'))
    print('evidence:', root, flush=True)
    workspace = root / 'workspace'
    workspace.mkdir()
    directory = root / 'vessel'
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}
    for key in ('HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME'):
        folder = root / key.lower()
        folder.mkdir(mode=0o700)
        env[key] = str(folder)
    image = base64.b64encode(png()).decode()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.requests, server.fail, server.image = [], False, image
    server.provider = args.provider
    server.model = 'claude-3-5-sonnet-20241022' if args.provider == 'anthropic' else 'gpt-4o'
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    config = root / 'config.toml'
    endpoint = f'http://127.0.0.1:{server.server_port}/v1'
    auth_required = 'true' if args.provider in ('anthropic', 'chatgpt-oauth') else 'false'
    env['IMAGE_FIXTURE_KEY'] = 'synthetic-fixture-key'
    config.write_text(f'provider = "{args.provider}"\nmodel = "{server.model}"\napi_key_required = {auth_required}\napi_key_env = "IMAGE_FIXTURE_KEY"\n'
                      f'base_url = "{endpoint}"\nchatgpt_base_url = "{endpoint}"\n'
                      'max_tokens = 128\nprovider_retry_attempts = 1\naccess = "read-only"\ncontext_window = 0\n')
    if args.provider == 'chatgpt-oauth':
        tokens = Path(env['XDG_DATA_HOME']) / 'helm/chatgpt-oauth.json'
        tokens.parent.mkdir(mode=0o700)
        tokens.write_text(json.dumps({'access_token': 'synthetic-access', 'refresh_token': 'synthetic-refresh',
            'account_id': 'synthetic-account', 'expires_at': int(time.time()) + 3600}))
        tokens.chmod(0o600)
    config.chmod(0o600)
    log = (root / 'vessel.log').open('wb')
    supervisor = None
    sessions = []

    def request(command, allow_error=False, bearer=None):
        credential = json.loads((directory / 'process-http.json').read_text())
        req = urllib.request.Request(credential['endpoint'] + '/v1/vessel/command',
            data=json.dumps({'protocol': 1, 'command': command}).encode(),
            headers={'Authorization': 'Bearer ' + (bearer or credential['token']), 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=30) as response:
            value = json.load(response)
        if not allow_error:
            assert value.get('error') is None, value
        return value

    def command(sid, value, allow_error=False):
        outer = request({**value, 'session_id': sid}, allow_error)
        if outer.get('error'):
            return outer
        reply = outer['result']
        if not allow_error:
            assert reply.get('error') is None, reply
        return reply.get('result', reply)

    def start():
        sid = str(uuid.uuid4())
        request({'op': 'start_configured', 'session_id': sid, 'command_id': str(uuid.uuid4()),
                 'workspace': str(workspace), 'config_path': str(config)})
        sessions.append(sid)
        return sid

    def submit(sid, parts):
        saved = command(sid, {'op': 'snapshot'})
        return {'op': 'submit_content', 'command_id': str(uuid.uuid4()), 'expected_revision': saved['revision'],
                'expires_at_ms': int(time.time()*1000) + 60000, 'content': parts}

    def finished(sid, state='completed'):
        def observe():
            saved = command(sid, {'op': 'snapshot'})
            return saved if saved.get('run', {}).get('state') == state and saved.get('pending_cleanup_run') is None else None
        saved = wait_for(observe)
        wait_for(lambda: request({'op': 'inspect', 'session_id': sid})['result']['state'] == 'suspended')
        return saved

    def owned_pids():
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    argv = (entry / 'cmdline').read_bytes().split(b'\0')
                    if argv[:3] == [os.fsencode(binary / 'voyage'), b'serve', b'--directory'] and Path(os.fsdecode(argv[3])).parent == directory / 'sessions':
                        found.append(int(entry.name))
                except (OSError, IndexError):
                    pass
        return found

    try:
        supervisor = subprocess.Popen([str(binary / 'vessel'), 'local-serve', '--directory', str(directory),
            '--voyage-binary', str(binary / 'voyage')], env=env, cwd=workspace,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / 'process-http.json').exists())
        sid = start()
        upload = {'op': 'upload_image', 'upload_id': str(uuid.uuid4()), 'name': 'pixels.png', 'data_base64': image}
        try:
            request({**upload, 'session_id': sid}, bearer='invalid')
            raise AssertionError('unauthorized upload admitted')
        except urllib.error.HTTPError as e:
            assert e.code in (401, 403), e
        metadata = command(sid, upload)
        assert metadata['media_type'] == 'image/png' and metadata['width'] == metadata['height'] == 2
        assert command(sid, upload) == metadata, 'upload duplicate changed immutable metadata'
        conflict = command(sid, {**upload, 'name': 'changed.png'}, True)
        assert conflict.get('error'), conflict
        malformed = command(sid, {**upload, 'upload_id': str(uuid.uuid4()), 'data_base64': base64.b64encode(b'not an image').decode()}, True)
        assert malformed.get('error'), malformed
        part = {'type': 'image', 'attachment': metadata}
        first = submit(sid, [part])
        admitted = command(sid, first)
        saved = finished(sid)
        assert command(sid, first)['run_id'] == admitted['run_id'], 'exact command lost across resume'
        assert len(server.requests) == 1, 'deduplication dispatched twice'
        assert normalized_messages(server.requests[0])[-1]['content'][0]['type'] == 'image_url'
        assert normalized_messages(server.requests[0])[-1]['content'][0]['image_url']['url'].endswith(image)
        assert image not in json.dumps(saved) and any(m.get('parts') == [part] for m in saved['messages'])
        changed = command(sid, {**first, 'content': [{'type': 'text', 'text': 'changed'}, part]}, True)
        assert changed.get('error'), changed
        other = start()
        cross = command(other, submit(other, [part]), True)
        assert cross.get('error'), cross
        assert len(server.requests) == 1
        ordered = [{'type': 'text', 'text': 'before\n'}, part, {'type': 'text', 'text': '\nafter'}]
        command(sid, submit(sid, ordered))
        saved = finished(sid)
        latest = normalized_messages(server.requests[-1])[-1]['content']
        assert [x['type'] for x in latest] == ['text', 'image_url', 'text']
        assert latest[0]['text'] == 'before\n' and latest[2]['text'] == '\nafter'
        assert sum(isinstance(m.get('content'), list) for m in normalized_messages(server.requests[-1])) >= 2, 'resume omitted retained image'
        oversized = command(other, submit(other, [part] * 5), True)
        assert oversized.get('error'), oversized
        # Unsupported models refuse new image turns before provider dispatch.
        other_image = command(other, {**upload, 'upload_id': str(uuid.uuid4())})
        current = command(other, {'op': 'snapshot'})
        command(other, {'op': 'set_model', 'command_id': str(uuid.uuid4()),
            'expected_revision': current['revision'], 'expires_at_ms': int(time.time()*1000)+60000,
            'model': 'gpt-3.5-turbo'})
        unsupported = command(other, submit(other, [{'type': 'image', 'attachment': other_image}]), True)
        assert unsupported.get('error'), unsupported
        assert len(server.requests) == 2, 'unsupported model reached provider'
        # A branch receives its own durable copy, including repeated references.
        current = command(sid, {'op': 'snapshot'})
        owner = request({'op': 'inspect', 'session_id': sid})['result']
        branch = str(uuid.uuid4())
        request({'op': 'branch', 'session_id': sid, 'incarnation': owner['incarnation'],
            'command_id': str(uuid.uuid4()), 'branch_id': branch, 'name': 'Image branch',
            'expected_revision': current['revision'], 'expires_at_ms': int(time.time()*1000)+60000})
        sessions.append(branch)
        command(branch, submit(branch, [{'type': 'text', 'text': 'read retained branch images'}]))
        branched = finished(branch)
        assert any(m.get('parts') == [part] for m in branched['messages'])
        assert len(server.requests) == 3
        server.fail = True
        command(sid, submit(sid, [{'type': 'text', 'text': 'force provider error'}]))
        failed = finished(sid, 'failed')
        assert 'PRIVATE-IMAGE-ECHO' not in json.dumps(failed) and image not in json.dumps(failed)
        assert len(server.requests) == 4
        log.flush()
        assert 'PRIVATE-IMAGE-ECHO' not in (root / 'vessel.log').read_text(errors='replace')
        assert image not in (root / 'vessel.log').read_text(errors='replace')
        (root / 'results.json').write_text(json.dumps({'status': 'passed', 'provider': args.provider, 'requests': len(server.requests), 'sessions': sessions}))
        print('PASS: authorization, upload integrity/dedup, malformed files, image-only send, exact command conflict/dedup, ordered content, cross-session refusal, retained images across suspension, limits, unsupported models, branch persistence and provider-error privacy')
    finally:
        for pid in owned_pids():
            try:
                fd = os.pidfd_open(pid)
                try:
                    if pid in owned_pids():
                        signal.pidfd_send_signal(fd, signal.SIGTERM)
                finally:
                    os.close(fd)
            except ProcessLookupError:
                pass
        wait_for(lambda: not owned_pids())
        if supervisor is not None:
            supervisor.terminate()
            supervisor.wait(timeout=10)
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        log.close()


if __name__ == '__main__':
    main()
