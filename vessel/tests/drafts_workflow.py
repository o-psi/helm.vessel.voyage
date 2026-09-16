"""Offline #309 shared-draft process checks; synthetic pixels, no provider calls.

Uses two independent HTTP clients against the same local owner authority. Public
pairing/grant boundaries are separately covered by Vessel's focused Rust tests.
"""
import argparse
import base64
import concurrent.futures
import json
import os
from pathlib import Path
import struct
import signal
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import zlib


def uid():
    return str(uuid.uuid4())


def png():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 2, 2, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(b'\x00\xff\x00\x00\x00\xff\x00' * 2)) + chunk(b'IEND', b''))


def wait_for(probe):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        try:
            value = probe()
            if value:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(.05)
    raise AssertionError('fixture readiness timeout')


def main(binaries):
    root = Path(tempfile.mkdtemp(prefix='voyage-drafts-309-'))
    print('evidence:', root, flush=True)
    workspace = root / 'workspace'
    workspace.mkdir(mode=0o700)
    directory = root / 'vessel'
    env = {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', 'PROVIDER_FIXTURE_KEY': 'synthetic-offline-only'}
    for key in ('HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME', 'XDG_CONFIG_HOME', 'XDG_CACHE_HOME'):
        path = root / key.lower()
        path.mkdir(mode=0o700)
        env[key] = str(path)
    log = (root / 'vessel.log').open('wb')
    supervisor = None

    def request(command, allow_error=False, token=None):
        credential = json.loads((directory / 'process-http.json').read_text())
        req = urllib.request.Request(credential['endpoint'] + '/v1/vessel/command',
            data=json.dumps({'protocol': 1, 'command': command}).encode(),
            headers={'Authorization': 'Bearer ' + (token or credential['token']), 'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=15) as response:
            reply = json.load(response)
        if not allow_error:
            assert reply.get('error') is None and reply.get('outcome_unknown') is False, reply
        return reply

    def draft(operation, allow_error=False):
        reply = request({'op': 'drafts', 'operation': operation}, allow_error)
        return reply if reply.get('error') else reply['result']

    def owned_pids():
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    argv = (entry / 'cmdline').read_bytes().split(b'\0')
                    if argv[:3] == [os.fsencode(binaries / 'voyage'), b'serve', b'--directory'] and Path(os.fsdecode(argv[3])).parent == directory / 'sessions':
                        found.append(int(entry.name))
                except (OSError, IndexError):
                    pass
        return found

    def start():
        nonlocal supervisor
        supervisor = subprocess.Popen([str(binaries / 'vessel'), 'local-serve', '--directory', str(directory),
            '--voyage-binary', str(binaries / 'voyage')], env=env, cwd=workspace,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        wait_for(lambda: (directory / 'process-http.json').exists() and request({'op': 'capabilities'}))

    try:
        start()
        assert request({'op': 'catalogue'})['result'] == [], 'saving must not create voyages'
        draft_id = uid()
        document = {'target': {'type': 'new_chat', 'workspace': str(workspace)},
                    'parts': [{'type': 'text', 'text': 'Typed on TUI\nUnicode: café 🌊'}]}
        put = {'op': 'put', 'command_id': uid(), 'draft_id': draft_id, 'expected_revision': 0, 'document': document}
        first = draft(put)
        assert first['revision'] == 1 and first['document'] == document
        assert draft(put) == first, 'lost response exact replay must not create another revision'
        assert draft({'op': 'get', 'draft_id': draft_id}) == first
        listed = draft({'op': 'list'})
        assert any(item['draft_id'] == draft_id for item in listed['drafts']), 'second device must discover IDs'
        assert request({'op': 'catalogue'})['result'] == [], 'draft allocated a voyage'
        changed = {**put, 'document': {**document, 'parts': [{'type': 'text', 'text': 'changed identity'}]}}
        assert draft(changed, True).get('error'), 'command ID reuse with different content accepted'
        try:
            request({'op': 'drafts', 'operation': {'op': 'list'}}, token='unauthorized')
            raise AssertionError('unauthorized draft discovery')
        except urllib.error.HTTPError as error:
            assert error.code in (401, 403)
        edits = [{**put, 'command_id': uid(), 'expected_revision': 1,
                  'document': {**document, 'parts': [{'type': 'text', 'text': text}]}}
                 for text in ('Edited on phone', 'Edited on laptop')]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            replies = list(pool.map(lambda value: draft(value, True), edits))
        assert sum(bool(reply.get('error')) for reply in replies) == 1, replies
        current = draft({'op': 'get', 'draft_id': draft_id})
        assert current['revision'] == 2
        image = base64.b64encode(png()).decode()
        upload = {'op': 'upload_image', 'command_id': uid(), 'draft_id': draft_id,
                  'name': 'pixels.png', 'data_base64': image}
        attachment = draft(upload)
        assert attachment['media_type'] == 'image/png' and attachment['width'] == attachment['height'] == 2
        assert draft(upload) == attachment
        chunk = draft({'op': 'read_image', 'draft_id': draft_id, 'attachment_id': attachment['id'], 'offset': 0, 'limit': 262144})
        assert base64.b64decode(chunk['data_base64']) == png() and chunk['next_offset'] is None
        invalid = {**upload, 'command_id': uid(), 'data_base64': base64.b64encode(b'not an image').decode()}
        assert draft(invalid, True).get('error')
        parts = [{'type': 'text', 'text': 'before\n'}, {'type': 'image', 'attachment': attachment},
                 {'type': 'text', 'text': '\nafter'}]
        saved = draft({**put, 'command_id': uid(), 'expected_revision': 2, 'document': {**document, 'parts': parts}})
        assert saved['revision'] == 3 and saved['document']['parts'] == parts
        # Restart only the supervisor: this fixture has deliberately created no voyage processes.
        supervisor.terminate()
        supervisor.wait(timeout=10)
        start()
        assert draft({'op': 'get', 'draft_id': draft_id}) == saved, 'restart lost draft or ordered attachments'
        assert request({'op': 'catalogue'})['result'] == []
        assert draft({'op': 'delete', 'command_id': uid(), 'draft_id': draft_id, 'expected_revision': 2}, True).get('error'), 'stale post-send clear erased newer edits'
        assert draft({'op': 'get', 'draft_id': draft_id}) == saved
        # Explicit promotion copies to a separately owned session without running a provider.
        config = root / 'config.toml'
        config.write_text(json.dumps({'version': 1, 'workspace': str(workspace), 'config': {'provider': 'openai-chat', 'model': 'gpt-4o', 'api_key_required': False, 'base_url': 'http://127.0.0.1:9/v1', 'access': 'read-only'}, 'explicit': {'access': 'read-only'}, 'selection': None, 'confirmation': None}))
        config.chmod(0o600)
        sid = uid()
        def account_cli(*arguments):
            reply = subprocess.run([str(binaries / 'vessel'), 'auth', 'accounts', *arguments], env=env, cwd=workspace, capture_output=True, text=True, timeout=15)
            assert reply.returncode == 0, reply.stderr
            return json.loads(reply.stdout)
        connection = account_cli('connect', '--label', 'draft-fixture', '--endpoint', 'http://127.0.0.1:9/v1', '--transports', 'openai-chat')
        account = account_cli('add', '--connection', connection['id'], '--account', 'fixture', '--env', 'PROVIDER_FIXTURE_KEY')
        binding = {'account_id': account['id'], 'connection_id': connection['id'], 'identity_generation': account['identity_generation'], 'connection_revision': connection['revision'], 'transport': 'openai_chat'}
        request({'op': 'account_set_default', 'command_id': uid(), 'workspace': str(workspace), 'account': binding, 'expected_revision': 0})
        request({'op': 'start_settings', 'session_id': sid, 'command_id': uid(), 'workspace': str(workspace), 'config_path': str(config), 'binding': binding, 'settings': {}})
        promotion = {'op': 'promote', 'command_id': uid(), 'draft_id': draft_id, 'expected_revision': 3, 'session_id': sid}
        promoted = draft(promotion)
        assert draft(promotion) == promoted, 'promotion retry changed session references'
        assert [part['type'] for part in promoted['parts']] == ['text', 'image', 'text']
        artifact = promoted['parts'][1]['attachment']
        assert artifact['sha256'] == attachment['sha256'] and artifact['id'] != attachment['id']
        snapshot = request({'op': 'snapshot', 'session_id': sid})['result']['result']
        assert snapshot.get('run') is None, 'promotion started a run'
        clear = {**put, 'command_id': uid(), 'expected_revision': 3, 'document': {**document, 'parts': []}}
        cleared = draft(clear)
        assert cleared['revision'] == 4
        # Edits arriving during a send-clear can reuse retained staged references at the new revision.
        restored = draft({**put, 'command_id': uid(), 'expected_revision': 4, 'document': {**document, 'parts': parts}})
        assert restored['revision'] == 5
        delete = {'op': 'delete', 'command_id': uid(), 'draft_id': draft_id, 'expected_revision': 5}
        deleted = draft(delete)
        assert draft(delete) == deleted
        assert not any(item['draft_id'] == draft_id for item in draft({'op': 'list'})['drafts'])
        assert draft({**put, 'command_id': uid()}, True).get('error'), 'stale device resurrected deleted identity'
        retained = request({'op': 'read_artifact', 'session_id': sid, 'artifact_id': artifact['id'], 'offset': 0, 'limit': 65536})['result']['result']
        assert base64.b64decode(retained['data_base64']) == png(), 'discarding draft removed session artifact'
        log.flush()
        assert image not in (root / 'vessel.log').read_text(errors='replace'), 'image bytes entered diagnostics'
        checks = ['new-chat discovery', 'no session allocation', 'exact mutation replay', 'changed-ID refusal',
                  'unauthorized reads', 'simultaneous CAS conflict', 'validated immutable image upload',
                  'ordered attachment persistence', 'supervisor restart', 'stale clear refusal', 'deletion tombstone',
                  'image diagnostic privacy', 'idempotent promotion without execution', 'post-clear edits', 'session artifacts survive draft deletion']
        (root / 'results.json').write_text(json.dumps({'status': 'passed', 'checks': checks}, indent=2) + '\n')
        print('PASS:', ', '.join(checks))
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
        if supervisor is not None and supervisor.poll() is None:
            supervisor.terminate()
            supervisor.wait(timeout=10)
        log.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    main(parser.parse_args().bin_dir.resolve())
