#!/usr/bin/env python3
"""Focused offline #255 real Helm journeys. No builds or live provider.

Run with --source-root pointing to the released binary's integrated source tree.
Private short-lived runtime state uses /tmp/vdr-* (Unix socket length limit).
Only sanitized summaries are copied to --evidence; reuses focused fixture helpers.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import uuid


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hash(root):
    files = sorted(p for name in ('helm', 'vessel', 'voyage', 'crates', 'installer')
                   for p in (root / name).rglob('*') if p.suffix == '.rs')
    files += [root / 'Cargo.toml', root / 'Cargo.lock']
    files += sorted(p for name in ('helm', 'vessel', 'voyage', 'crates', 'installer') for p in (root / name).rglob('Cargo.toml'))
    return {str(p.relative_to(root)): sha(p) for p in files}


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        s = self.server
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        s.bodies.append(body)
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        self.close_connection = True
        def emit(value):
            self.wfile.write(('data: ' + json.dumps(value, ensure_ascii=False) + '\n\n').encode())
            self.wfile.flush()
        def delta(index, text):
            emit({'type': 'response.function_call_arguments.delta', 'output_index': index, 'delta': text})
        def done(output):
            emit({'type': 'response.completed', 'response': {'id': 'synthetic255', 'status': 'completed',
                'output': output, 'usage': {'input_tokens': 1, 'output_tokens': 1}}})
        try:
            if len(s.bodies) > 1:
                done([{'type': 'message', 'role': 'assistant', 'content': [
                    {'type': 'output_text', 'text': 'FINAL255'}]}])
                return
            emit({'type': 'response.output_text.delta', 'delta': 'ANSWER255 before tools.'})
            emit({'type': 'response.reasoning_summary_text.delta', 'output_index': 2, 'delta': 'DISCLOSURE255 café '})
            args = [json.dumps({'path': f'effect{i}.txt', 'content': f'Unicode café λ 工具 {i}\n' + 'long payload ' * 90}, ensure_ascii=False) for i in range(2)]
            for i in range(2):
                emit({'type': 'response.output_item.added', 'output_index': i, 'item': {
                    'type': 'function_call', 'call_id': f'call255_{i}', 'name': 'write_file', 'arguments': ''}})
                delta(i, args[i][:42])
            s.stage.set()
            assert s.advance.wait(45), 'first fixture gate timeout'
            for start in range(42, len(args[0]), 31):
                for i in (1, 0):
                    delta(i, args[i][start:start+31])
                if start == 42:
                    emit({'type': 'response.reasoning_summary_text.delta', 'output_index': 2, 'delta': 'λ interleaved summary.'})
                time.sleep(.025)
            s.generated.set()
            assert s.finish.wait(45), 'final fixture gate timeout'
            if s.interrupt:
                return  # Actual EOF without response.completed, never executable.
            done([{'type': 'function_call', 'call_id': f'call255_{i}', 'name': 'write_file', 'arguments': args[i]} for i in range(2)])
        except (BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as error:
            s.errors.append(repr(error))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    for key in ('source_root', 'bin_dir', 'manifest', 'evidence'):
        setattr(args, key, getattr(args, key).resolve())
    args.evidence.mkdir(parents=True, exist_ok=True)
    # Unix socket pathname limits require short private runtime state.
    # Only fixture-owned ephemeral state goes in /tmp; evidence is copied below.
    tempfile.tempdir = '/tmp'
    sys.path[:0] = [str(args.source_root / 'voyage/tests'), str(args.source_root / 'helm/tests')]
    from provider_attempts import Fixture as AccountFixture, session
    class Fixture(AccountFixture):
        def request(self, command, allow_error=False):
            if command["op"] == "start_settings":
                path = Path(command["config_path"])
                config = json.loads(path.read_text())
                self.binding = config["config"]["account"]
                config["config"].update(provider_stream_idle_ms=60000, provider_response_timeout_ms=120000, provider_retry_attempts=1, context_window=0)
                path.write_text(json.dumps(config))
            return super().request(command, allow_error)
    from delivery_recovery import wait_for
    from approval_semantics import Gateway
    from private_terminal import OuterPTY
    manifest = json.loads(args.manifest.read_text())
    before = source_hash(args.source_root)
    binaries = {name: sha(args.bin_dir / name) for name in ('helm', 'vessel', 'voyage')}
    assert binaries == manifest['binary_sha256'], 'released binary hash mismatch'
    result = {'binary_sha256': binaries, 'build_manifest': manifest, 'checks': [], 'platform': sys.platform,
              'scope': 'synthetic provider; real Helm PTY; local and scoped loopback HTTP gateway, not separate host/TLS/native/live provider'}
    (args.evidence / 'source-before.json').write_text(json.dumps(before, sort_keys=True))
    try:
        for remote in (False, True):
            for interrupt in (False, True):
                f = Fixture(args.bin_dir)
                f.provider.RequestHandlerClass = Provider
                for name in ('stage', 'advance', 'generated', 'finish'):
                    setattr(f.provider, name, threading.Event())
                f.provider.errors = []
                f.provider.interrupt = interrupt
                gateway = None
                clients = []
                label = ('scoped-loopback' if remote else 'local') + ('-interrupted' if interrupt else '-complete')
                result[label] = {'evidence': str(f.root), 'checks': []}
                checks = result[label]['checks']
                def check(text):
                    checks.append(text)
                    print('PASS', label, text, flush=True)
                try:
                    f.start()
                    # Reuse the current named/default-account enrollment, not cache-only legacy launch.
                    sid = session(f, 'responses', 'success')
                    route = ['--directory', str(f.directory), '--no-start']
                    if remote:
                        gateway = Gateway(f)
                        grant = gateway.grant(sid, ['observe', 'history', 'execute', 'cancel', 'steer'], lifetime=300000)
                        access = f.root / 'scoped-access.json'
                        access.write_text(json.dumps(grant))
                        access.chmod(0o600)
                        route = ['--access-file', str(access)]
                    def cli(*values):
                        run = subprocess.run([str(args.bin_dir / 'helm'), 'connect', *route, *values],
                            env=f.env, cwd=f.workspace, capture_output=True, text=True, timeout=20)
                        assert run.returncode == 0, (run.stdout, run.stderr)
                        return run.stdout
                    def attach():
                        client = OuterPTY([str(args.bin_dir / 'helm'), 'connect', *route],
                            f.env, f.workspace, f.root / f'helm-{len(clients)}.ansi')
                        clients.append(client)
                        return client
                    def capture(client, name):
                        text = client.pump().text()
                        (f.root / (name + '.screen.txt')).write_text(text)
                        return text
                    cli('submit', '--expected-revision', str(f.command(sid, {'op': 'snapshot'})['revision']), '--command-id', str(uuid.uuid4()), '--expires-at-ms', str(int(time.time()*1000)+60000), sid, 'Synthetic slow stream255, perform the two file writes only after final calls.')
                    wait_for(f.provider.stage.is_set)
                    tui = attach()
                    tui.until(lambda s: 'Reasoning summary' in s.text() and 'write_file' in s.text(), 'live cards')
                    text = capture(tui, 'initial')
                    assert 'DISCLOSURE255' not in text, text
                    assert not list(f.workspace.glob('effect*.txt'))
                    check('live tool and independently collapsed reasoning label before final, no early effect')
                    # From the top, forward traversal selects the first disclosure (reasoning).
                    tui.send(b'\x1b[1;6B')
                    time.sleep(.15)
                    tui.send(b'\x00')
                    tui.until(lambda s: 'DISCLOSURE255' in s.text(), 'keyboard reasoning expansion')
                    capture(tui, 'reasoning-expanded')
                    check('Ctrl+Shift+Down then Ctrl+Space expands separate disclosure')
                    tui.send(b'\x00')
                    tui.until(lambda s: 'DISCLOSURE255' not in s.text(), 'reasoning collapse')
                    tui.send(b'\x1b[1;6B')  # Next tool disclosure.
                    time.sleep(.15)
                    tui.pump()
                    tui.send(b'\x1b[1;6A')  # Back to independent reasoning.
                    time.sleep(.15)
                    tui.send(b'\x00')
                    tui.until(lambda s: 'DISCLOSURE255' in s.text(), 'reverse keyboard traversal')
                    tui.send(b'\x00')
                    tui.until(lambda s: 'DISCLOSURE255' not in s.text(), 'independent collapse')
                    tui.resize(60, 32)
                    tui.until(lambda s: 'Reasoning summary' in s.text(), 'narrow disclosure label')
                    capture(tui, 'narrow')
                    check('Ctrl+Shift+Up/Down independent traversal and collapsed label at 60x32')
                    tui.resize(120, 36)
                    f.provider.advance.set()
                    wait_for(f.provider.generated.is_set)
                    snap = f.command(sid, {'op': 'snapshot'})
                    (f.root / 'before-final.json').write_text(json.dumps(snap, indent=2))
                    assert len(snap['run']['tool_previews']) == 2, snap['run']
                    assert all('工具' in p['arguments'] for p in snap['run']['tool_previews'])
                    assert not list(f.workspace.glob('effect*.txt'))
                    check('two interleaved Unicode/long-argument previews retained; complete JSON still has no effect before provider final')
                    tui.send(b'\x11')  # Ctrl+Q detaches Helm, not the voyage.
                    tui.exited_restored()
                    tui = attach()
                    tui.until(lambda s: 'write_file' in s.text(), 'reconnect live tool previews')
                    tui.send(b'\x1b[5~' * 6)  # PageUp through long bounded previews.
                    tui.until(lambda s: 'Reasoning summary' in s.text(), 'reconnect reasoning preview')
                    capture(tui, 'reconnected')
                    assert len(f.provider.bodies) == 1
                    assert not list(f.workspace.glob('effect*.txt'))
                    check('real Helm detach/reconnect retains provisional cards without request replay or effect')
                    f.provider.finish.set()
                    final = f.finished(sid)
                    (f.root / 'final.json').write_text(json.dumps(final, indent=2))
                    if interrupt:
                        assert final['run']['state'] == 'failed', final['run']
                        assert not list(f.workspace.glob('effect*.txt'))
                        tui.until(lambda s: 'unfinished disclosure' in s.text(), 'interrupted disclosure label')
                        check('actual interrupted HTTP stream fails, no partial call executes, unfinished disclosure label')
                    else:
                        assert final['run']['state'] == 'completed', final['run']
                        for i in range(2):
                            assert (f.workspace / f'effect{i}.txt').read_text() == f'Unicode café λ 工具 {i}\n' + 'long payload ' * 90
                        calls = [c for m in final['messages'] for c in m.get('tool_calls', [])]
                        assert len(calls) == 2, calls
                        assert not final['run'].get('tool_previews'), final['run']
                        tui.until(lambda s: 'FINAL255' in s.text(), 'final answer')
                        check('exact two canonical calls, full Unicode effects and no remaining provisional tool cards')
                    capture(tui, 'final')
                    assert not f.provider.errors, f.provider.errors
                finally:
                    f.provider.advance.set()
                    f.provider.finish.set()
                    for client in clients:
                        client.close()
                    if gateway:
                        gateway.close()
                    f.close()
                    check('owned fixture cleanup observed')
                    # Runtime credentials/state stay in the private fixture root.
                    (args.evidence / (label + '.json')).write_text(json.dumps(result[label], indent=2))
        result['behavioral_checks_passed'] = True
    finally:
        after = source_hash(args.source_root)
        result['source_unchanged'] = before == after
        result['changed_source_paths'] = [p for p in sorted(before.keys() | after.keys()) if before.get(p) != after.get(p)]
        result['binaries_unchanged'] = binaries == {name: sha(args.bin_dir / name) for name in binaries}
        result['passed'] = result.get('behavioral_checks_passed', False) and result['source_unchanged'] and result['binaries_unchanged']
        (args.evidence / 'result.json').write_text(json.dumps(result, indent=2))
        assert result['source_unchanged'] and result['binaries_unchanged'], 'source/build changed during journey'


if __name__ == '__main__':
    main()
