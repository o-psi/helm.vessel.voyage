#!/usr/bin/env python3
"""Real bracketed-paste regression: modal ownership, Unicode, resize and steering."""
import json
from pathlib import Path
import tempfile
import threading
from http.server import ThreadingHTTPServer

from voyage_ui import Server as BaseServer, Terminal

PASTE = 'PASTE-雪-λ'
DRAFT = 'original-draft'


class Server(BaseServer):
    entered = threading.Event()
    release = threading.Event()
    requests = []

    def do_POST(self):
        type(self).model_posts += 1
        self.requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
        self.entered.set()
        self.release.wait(20)
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.end_headers()
            self.wfile.write(b'data: {"choices":[{"delta":{"content":"fixture complete"}}]}\n\ndata: [DONE]\n\n')
        except (BrokenPipeError, ConnectionResetError):
            pass


def paste(terminal):
    terminal.send(b'\x1b[200~' + PASTE.encode() + b'\x1b[201~')
    # Allow the event loop to consume the paste before resizing or dismissing the panel.
    for _ in range(4):
        terminal.drain()


def close_panel(terminal, count=1):
    for _ in range(count):
        # Helm requests disambiguated keys; encode Escape explicitly, avoiding an
        # ambiguous raw ESC prefix combining with a subsequent control key.
        terminal.send(b'\x1b[27u')
        for _ in range(3):
            terminal.drain()


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    cases = ('help', 'todo-list', 'todo-inspect', 'todo-input', 'model', 'supervisor',
             'workflow-picker', 'workflow-form', 'voyage-library', 'voyage-form',
             'sessions', 'terminals', 'active-steering')
    try:
        for case in cases:
            Server.model_posts = 0
            Server.requests = []
            Server.entered.clear()
            Server.release.clear()
            with tempfile.TemporaryDirectory(prefix='helm-paste-') as raw:
                root = Path(raw)
                workflows = root / '.helm/workflows'
                workflows.mkdir(parents=True)
                (workflows / 'review.toml').write_text("schema_version=1\nid='review'\nversion='1'\ndescription='Paste fixture'\nprompt='Review {{topic}}'\n[parameters.topic]\ntype='string'\nrequired=true\n")
                terminal = Terminal(root, server.server_port)
                try:
                    terminal.text('Recent · Ctrl+S')
                    if case.startswith('todo'):
                        terminal.send(b'\x04')
                        terminal.text('Workspace plan')
                        terminal.send('nseed-todo\r')
                        terminal.text('seed-todo')
                        close_panel(terminal)
                    if case == 'active-steering':
                        terminal.send('hold-run\r')
                        terminal.wait(Server.entered.is_set, 'held provider request')
                    if case.startswith('workflow'):
                        terminal.send('/workflow\r')
                        terminal.text('Saved workflows')
                        terminal.text('review [')
                        expected_draft = '/workflow'
                    else:
                        terminal.send(DRAFT)
                        terminal.text(DRAFT)
                        expected_draft = DRAFT
                    if case == 'help':
                        terminal.send(b'\x1bOP')
                        terminal.text('shortcuts · F1/Esc close')
                    elif case.startswith('todo'):
                        terminal.send(b'\x04')
                        terminal.text('Workspace plan')
                        if case == 'todo-inspect':
                            terminal.send(b'\r')
                            terminal.text('Todo detail')
                        elif case == 'todo-input':
                            terminal.send('n')
                            terminal.text('Add · title')
                    elif case == 'model':
                        terminal.send(b'\x1b[109;5u')
                        terminal.text('Search models')
                    elif case == 'supervisor':
                        terminal.send(b'\x01')
                        terminal.text('Agent tree')
                    elif case == 'workflow-form':
                        terminal.send(b'\r')
                        terminal.text('Input 1/1: topic')
                    elif case.startswith('voyage'):
                        terminal.send(b'\x16')
                        terminal.text('Voyages · drafts')
                        if case == 'voyage-form':
                            terminal.send('n')
                            terminal.text('New voyage')
                    elif case == 'sessions':
                        terminal.send(b'\x13')
                        terminal.text('Recent · Ctrl+S')
                    elif case == 'terminals':
                        terminal.send(b'\x14')
                        terminal.text('Terminals')
                    if case == 'active-steering':
                        for opener, visible in [(b'\x1bOP', 'shortcuts · F1/Esc close'),
                                                (b'\x04', 'Workspace plan'),
                                                (b'\x01', 'Agent tree'),
                                                (b'\x13', 'Recent · Ctrl+S'),
                                                (b'\x14', 'Terminals')]:
                            terminal.send(opener)
                            terminal.text(visible)
                            paste(terminal)
                            close_panel(terminal)
                        terminal.send(b'\r')
                        terminal.wait(lambda: any(len(s.get('messages', [])) >= 2 for s in terminal.sessions()), 'durable steering')
                        terminal.send(b'\x1b[27u')
                        terminal.wait(lambda: 'not applied' in terminal.screen() or 'Cancelled' in terminal.screen() or 'Run stopped' in terminal.screen(), 'cancel held run')
                        expected_draft = ''
                    else:
                        terminal.resize(16, 48, 480, 320)
                        paste(terminal)
                        if case in ('todo-input', 'model', 'workflow-form', 'voyage-form'):
                            terminal.resize(32, 120, 1200, 640)
                            terminal.text(PASTE)
                        close_panel(terminal, 2 if case in ('todo-inspect', 'todo-input', 'voyage-form') else 1)
                    terminal.finish()
                    sessions = terminal.sessions()
                    assert len(sessions) == 1, (case, sessions)
                    assert sessions[0].get('draft', '') == expected_draft, (case, sessions[0].get('draft', ''))
                    assert PASTE not in json.dumps(sessions, ensure_ascii=False), case
                    assert Server.model_posts == (1 if case == 'active-steering' else 0), (case, Server.model_posts)
                    if case == 'active-steering':
                        user_text = [m['content'] for m in sessions[0]['messages'] if m['role'] == 'user']
                        assert user_text == ['hold-run', DRAFT], user_text
                    print(f'paste routing: {case} passed')
                finally:
                    Server.release.set()
                    terminal.close()
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    main()
