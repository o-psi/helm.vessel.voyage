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


class AcceptanceProvider(Provider):
    """Bounded gates expose each security boundary before provider finalization."""
    def do_POST(self):
        s = self.server
        s.bodies.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        self.close_connection = True
        def emit(value):
            self.wfile.write(('data: ' + json.dumps(value) + '\n\n').encode())
            self.wfile.flush()
        def gate(name):
            assert getattr(s, name).wait(30), name + ' timeout'
        try:
            if s.case == 'anthropic':
                emit({'type': 'message_start', 'message': {'id': 'thinking255', 'role': 'assistant', 'content': [], 'usage': {'input_tokens': 1}}})
                emit({'type': 'content_block_start', 'index': 0, 'content_block': {'type': 'thinking', 'thinking': ''}})
                emit({'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'thinking_delta', 'thinking': 'THINKING255 café independent disclosure.'}})
                emit({'type': 'content_block_delta', 'index': 0, 'delta': {'type': 'signature_delta', 'signature': 'OPAQUE_SIGNATURE255'}})
                emit({'type': 'content_block_stop', 'index': 0})
                emit({'type': 'content_block_start', 'index': 1, 'content_block': {'type': 'redacted_thinking', 'data': 'OPAQUE_REDACTED255'}})
                emit({'type': 'content_block_stop', 'index': 1})
                s.stage.set()
                gate('finish')
                emit({'type': 'content_block_start', 'index': 2, 'content_block': {'type': 'text', 'text': ''}})
                emit({'type': 'content_block_delta', 'index': 2, 'delta': {'type': 'text_delta', 'text': 'FINAL255'}})
                emit({'type': 'content_block_stop', 'index': 2})
                emit({'type': 'message_delta', 'delta': {'stop_reason': 'end_turn'}, 'usage': {'output_tokens': 1}})
                emit({'type': 'message_stop'})
                return
            emit({'type': 'response.output_item.added', 'output_index': 0, 'item': {'type': 'function_call', 'call_id': 'accept255', 'name': 'write_file', 'arguments': ''}})
            arguments = '{"path":"effect.txt","content":"'
            arguments += s.secret[:16] if s.case == 'secret' else 'PREVIEW255'
            emit({'type': 'response.function_call_arguments.delta', 'output_index': 0, 'delta': arguments})
            s.stage.set()
            gate('advance')
            tail = s.secret[16:] + '"}' if s.case == 'secret' else '"}'
            if s.case == 'malformed':
                tail += 'INVALID'
            emit({'type': 'response.function_call_arguments.delta', 'output_index': 0, 'delta': tail})
            s.generated.set()
            gate('finish')
            if s.case == 'failure':
                emit({'type': 'response.failed', 'response': {'error': {'code': 'invalid_request_error', 'message': 'Synthetic explicit failure255'}}})
            elif s.case in ('cancel', 'secret'):
                return
            else:
                emit({'type': 'response.completed', 'response': {'id': 'malformed255', 'status': 'completed', 'output': [{'type': 'function_call', 'call_id': 'accept255', 'name': 'write_file', 'arguments': arguments + tail}], 'usage': {'input_tokens': 1, 'output_tokens': 1}}})
        except (BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as error:
            s.errors.append(repr(error))


def extended_acceptance(args, Fixture, session, wait_for, OuterPTY, result):
    from provider_attempts import SECRET, history
    for case in ('malformed', 'cancel', 'failure', 'secret', 'anthropic'):
        f = Fixture(args.bin_dir)
        f.provider.RequestHandlerClass = AcceptanceProvider
        f.provider.case, f.provider.secret, f.provider.errors = case, SECRET, []
        for name in ('stage', 'advance', 'generated', 'finish'):
            setattr(f.provider, name, threading.Event())
        label = 'acceptance-' + case
        record = result[label] = {'evidence': str(f.root), 'checks': []}
        tui = None
        def check(text):
            record['checks'].append(text)
            print('PASS', label, text, flush=True)
        def observe():
            snap = f.command(sid, {'op': 'snapshot'})
            text = tui.pump().text()
            (f.root / 'observed.json').write_text(json.dumps(snap))
            (f.root / 'observed.screen.txt').write_text(text)
            return snap, text
        try:
            f.start()
            sid = session(f, 'anthropic' if case == 'anthropic' else 'responses', 'cancel_stream')
            receipt = f.command(sid, f.submit(sid, 'Offline acceptance255 fixture.'))
            wait_for(f.provider.stage.is_set)
            tui = OuterPTY([str(args.bin_dir / 'helm'), 'connect', '--directory', str(f.directory), '--no-start'], f.env, f.workspace, f.root / 'acceptance.ansi')
            tui.until(lambda s: ('Provider-exposed thinking' if case == 'anthropic' else 'write_file') in s.text(), 'acceptance preview')
            snap, text = observe()
            assert not (f.workspace / 'effect.txt').exists()
            check('real Helm provisional disclosure visible; no early file effect')
            if case == 'anthropic':
                assert 'THINKING255' not in text
                tui.send(b'\x1b[1;6B')
                time.sleep(.15)
                tui.send(b'\x00')
                tui.until(lambda s: 'THINKING255' in s.text(), 'thinking expanded')
                snap, text = observe()
                assert 'OPAQUE_' not in json.dumps(snap) + text
                check('thinking expands independently; signature and redacted block payload absent from snapshot and Helm')
            if case == 'secret':
                assert SECRET[:16] not in json.dumps(snap) + text
                check('configured account-secret prefix withheld before remaining chunk arrives')
            f.provider.advance.set()
            if case != 'anthropic':
                wait_for(f.provider.generated.is_set)
                time.sleep(.3)
            if case == 'secret':
                snap, text = observe()
                assert SECRET not in json.dumps(snap) + text and SECRET[:16] not in json.dumps(snap) + text
                assert '[REDACTED]' in json.dumps(snap['run']['tool_previews'])
                check('split configured secret redacted after assembly, absent from public snapshot and Helm')
            if case in ('cancel', 'secret'):
                snap = f.command(sid, {'op': 'snapshot'})
                f.command(sid, {'op': 'cancel', 'command_id': str(uuid.uuid4()), 'expected_revision': snap['revision'], 'expires_at_ms': int(time.time()*1000)+60000, 'run_id': receipt['run_id']})
            f.provider.finish.set()
            final = f.finished(sid)
            (f.root / 'final.json').write_text(json.dumps(final))
            expected = 'completed' if case == 'anthropic' else 'cancelled' if case in ('cancel', 'secret') else 'failed'
            assert final['run']['state'] == expected, final['run']
            assert not (f.workspace / 'effect.txt').exists()
            assert not [c for m in final['messages'] for c in m.get('tool_calls', [])]
            attempts = history(f, sid)
            (f.root / 'attempts.json').write_text(json.dumps(attempts))
            assert len(f.provider.bodies) == 1, len(f.provider.bodies)
            check(expected + '; zero canonical tool calls/effects; exactly one provider request')
            if case == 'anthropic':
                tui.until(lambda s: 'FINAL255' in s.text(), 'anthropic final')
                snap, text = observe()
                assert 'THINKING255' not in text, text
                assert 'OPAQUE_' not in json.dumps(final) + json.dumps(attempts) + text
                check('expanded live thinking automatically collapsed on finalization; opaque payload absent from final/attempt history')
                # No traversal here: the keyboard disclosure anchor must survive reconciliation.
                tui.send(b'\x00')
                try:
                    tui.until(lambda s: 'THINKING255' in s.text(), 'final anchor expansion', timeout=3)
                except AssertionError:
                    record['failure'] = 'finalized reasoning keyboard anchor lost; Ctrl+Space does not expand without re-selection'
                    print('FAIL', label, record['failure'], flush=True)
                    tui.send(b'\x1b[1;6B')
                    time.sleep(.15)
                    tui.send(b'\x00')
                    tui.until(lambda s: 'THINKING255' in s.text(), 'reselected final thinking')
                    check('explicit keyboard re-selection recovers finalized disclosure access')
                tui.send(b'\x00')
                tui.until(lambda s: 'THINKING255' not in s.text(), 'final anchor collapse')
                if 'failure' not in record:
                    check('same keyboard anchor expands/collapses finalized thinking without re-selection')
            else:
                tui.send(b'\x11')
                tui.exited_restored()
                tui.close()
                tui = OuterPTY([str(args.bin_dir / 'helm'), 'connect', '--directory', str(f.directory), '--no-start'], f.env, f.workspace, f.root / 'reattach.ansi')
                tui.until(lambda s: 'Offline acceptance255' in s.text(), 'reattach history')
                time.sleep(.5)
                assert len(f.provider.bodies) == 1 and not (f.workspace / 'effect.txt').exists()
                check('real Helm detach/reattach after terminal state does not replay provider or effect')
            assert not f.provider.errors, f.provider.errors
            record['passed'] = 'failure' not in record
        finally:
            f.provider.advance.set()
            f.provider.finish.set()
            if tui:
                tui.close()
            f.close()
            check('owned fixture cleanup observed')
            (args.evidence / (label + '.json')).write_text(json.dumps(record, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--extended-only', action='store_true', help='run only the five additional local acceptance journeys')
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
    if binaries != manifest['binary_sha256']:
        (args.evidence / 'preflight.json').write_text(json.dumps({
            'passed': False, 'reason': 'released binary hash mismatch',
            'manifest': manifest, 'observed_binary_sha256': binaries}, indent=2))
        raise AssertionError('released binary hash mismatch')
    result = {'binary_sha256': binaries, 'build_manifest': manifest, 'checks': [], 'platform': sys.platform,
              'scope': 'synthetic provider; real Helm PTY; local and scoped loopback HTTP gateway, not separate host/TLS/native/live provider'}
    (args.evidence / 'source-before.json').write_text(json.dumps(before, sort_keys=True))
    try:
        for remote in (() if args.extended_only else (False, True)):
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
        extended_acceptance(args, Fixture, session, wait_for, OuterPTY, result)
        result['behavioral_checks_passed'] = all(v.get('passed', True) for k, v in result.items() if k.startswith('acceptance-'))
    finally:
        after = source_hash(args.source_root)
        result['source_unchanged'] = before == after
        result['changed_source_paths'] = [p for p in sorted(before.keys() | after.keys()) if before.get(p) != after.get(p)]
        result['binaries_unchanged'] = binaries == {name: sha(args.bin_dir / name) for name in binaries}
        result['passed'] = result.get('behavioral_checks_passed', False) and result['source_unchanged'] and result['binaries_unchanged']
        (args.evidence / 'result.json').write_text(json.dumps(result, indent=2))
        assert result['source_unchanged'] and result['binaries_unchanged'], 'source/build changed during journey'
        assert result['passed'], 'acceptance matrix has recorded failures'


if __name__ == '__main__':
    main()
