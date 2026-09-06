#!/usr/bin/env python3
"""Native human attachment keeps echo and delayed repeats out of canonical model data."""
import json
import os
from pathlib import Path
import tempfile
import termios
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from completion_gate import response
from terminal_navigation import Tui


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        state = self.server.state
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        try:
            assert 'human-private-canary' not in json.dumps(body), 'human bytes reached provider'
            prompt = next(m['content'] for m in reversed(body['messages']) if m['role'] == 'user')
            state['requests'].append(body)
            step = state['steps'].get(prompt, 0) + 1
            state['steps'][prompt] = step
            if prompt.startswith('start-'):
                if step == 1:
                    echo = 'echo' if prompt == 'start-echo' else '-echo'
                    command = f'''stty {echo}; printf attach-ready; IFS= read -r human; printf 'app-repeat:%s\\n' "$human"; while [ ! -f release-late ]; do sleep 0.01; done; printf 'late-repeat:%s\\n' "$human"; exec /bin/sh'''
                    value = ('process', {'action': 'start', 'name': 'private-shell', 'command': command})
                else:
                    result = next(m['content'] for m in reversed(body['messages']) if m['role'] == 'tool')
                    state['terminal'] = result.rsplit(' ', 1)[1]
                    value = prompt + '-done'
            else:
                assert prompt.startswith('inspect-'), prompt
                if step == 1:
                    value = ('process', {'action': 'read', 'id': state['terminal']})
                elif step == 2:
                    result = next(m['content'] for m in reversed(body['messages']) if m['role'] == 'tool')
                    assert 'model capture unavailable' in result, result
                    value = ('process', {'action': 'write', 'id': state['terminal'], 'data': 'echo forbidden > model-effect\n'})
                elif step == 3:
                    result = next(m['content'] for m in reversed(body['messages']) if m['role'] == 'tool')
                    assert 'private' in result, result
                    value = ('process', {'action': 'list'})
                else:
                    value = prompt + '-done'
            data = response('openai-chat', value, len(state['requests']))
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        except Exception as error:
            state['failures'].append(repr(error))
            self.send_error(500)


def main():
    state = {'requests': [], 'steps': {}, 'failures': []}
    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    server.state = state
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        for mode in ['echo', 'noecho']:
            with tempfile.TemporaryDirectory(prefix='helm-terminal-privacy-') as directory:
                root = Path(directory)
                workspace = root / 'workspace'
                workspace.mkdir()
                env = dict(os.environ, TERM='xterm-256color', HOME=str(root/'home'), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), PRIVATE_FIXTURE_KEY='offline-fixture')
                config = root / 'provider.toml'
                config.write_text(f'provider="openai-chat"\nmodel="privacy-model"\nbase_url="http://127.0.0.1:{server.server_port}/v1"\napi_key_env="PRIVATE_FIXTURE_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\n')
                ui = Tui(['--config', str(config), '--workspace', str(workspace), 'chat'], env)
                try:
                    ui.text('HELM')
                    ui.turn('start-' + mode)
                    ui.send(b'\x14')
                    ui.text('Enter attach')
                    ui.send('\r')
                    ui.text('model capture private')
                    ui.text('attach-ready')
                    # Bracketed paste goes through crossterm's real Event::Paste
                    # and the native PTY, including Unicode and a newline.
                    canary = 'human-private-canary-界-' + mode
                    ui.send(b'\x1b[200~' + (canary+'\n').encode() + b'\x1b[201~')
                    ui.text('app-repeat:' + canary)
                    ui.send(b'\x14')
                    ui.text('Detached; terminal keeps running')
                    (workspace / 'release-late').touch()
                    # Reattach proves delayed application output remained visible
                    # to its human owner after the first view detached.
                    ui.send(b'\x14')
                    ui.text('Enter attach')
                    ui.send('\r')
                    ui.text('late-repeat:' + canary)
                    ui.send(b'\x1d')
                    ui.text('Detached; terminal keeps running')
                    ui.turn('inspect-' + mode)
                    assert not (workspace/'model-effect').exists(), 'model wrote to private terminal'
                    ui.finish()
                    restored = termios.tcgetattr(ui.master)
                    assert restored[3] & termios.ICANON and restored[3] & termios.ECHO, 'outer terminal modes not restored'
                    sessions = list((root/'data/helm/sessions').glob('*.json'))
                    assert sessions, 'canonical session was not saved'
                    for path in (root/'data').rglob('*'):
                        if path.is_file():
                            assert b'human-private-canary' not in path.read_bytes(), ('private bytes persisted', path.name)
                    record = json.loads(sessions[0].read_text())
                    assert 'model capture unavailable' in json.dumps(record), 'honest privacy gap was not persisted'
                    assert not state['failures'], state['failures']
                finally:
                    ui.close()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(5)
    print('terminal privacy: native echo/noecho, paste, delayed repeats, both detach chords, reattach, private model denial, canonical persistence and terminal restoration passed')


if __name__ == '__main__':
    main()
