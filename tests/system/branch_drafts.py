#!/usr/bin/env python3
"""Actual PTY branching preserves source drafts, ownership and recoverable input."""
import json
from pathlib import Path
import tempfile
import threading
from http.server import ThreadingHTTPServer

from voyage_ui import Server, Terminal


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Server)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        for slash in (False, True):
            for failure in (False, True):
                with tempfile.TemporaryDirectory(prefix='helm-branch-drafts-') as raw:
                    root = Path(raw)
                    terminal = Terminal(root, server.server_port)
                    try:
                        terminal.text('Recent · Ctrl+S')
                        terminal.send('/name source-voyage\r')
                        terminal.text('Session renamed')
                        terminal.send('old draft')
                        terminal.text('old draft')
                        terminal.finish()
                        source, = terminal.sessions()
                    finally:
                        terminal.close()
                    terminal = Terminal(root, server.server_port, source['id'])
                    try:
                        terminal.text('old draft')
                        if slash:
                            terminal.send(b'\x7f' * len('old draft'))
                            terminal.send('/branch child-voyage')
                            expected = ''
                            key = b'\r'
                        else:
                            terminal.send(' edited 雪 λ')
                            expected = 'old draft edited 雪 λ'
                            key = b'\x02'
                        terminal.text('/branch child-voyage' if slash else expected)
                        terminal.resize(18, 52, 500, 700)
                        for _ in range(4):
                            terminal.drain()
                        path = root / 'data/helm/sessions' / (source['id'] + '.json')
                        original = path.read_bytes()
                        if failure:
                            path.write_text('{bad-json')
                        terminal.send(key)
                        if failure:
                            terminal.text('Cannot branch')
                            assert path.read_text() == '{bad-json'
                            assert len(list(path.parent.glob('*.json'))) == 1
                            terminal.text('/branch child-voyage' if slash else expected)
                            path.write_bytes(original)
                            terminal.send(key)
                        terminal.text('Branched session')
                        terminal.wait(lambda: len(terminal.sessions()) == 2, 'both persisted identities')
                        records = {record['id']: record for record in terminal.sessions()}
                        child, = [record for record in records.values() if record['id'] != source['id']]
                        assert child['parent_id'] == source['id']
                        for record in records.values():
                            assert record.get('draft', '') == expected, (slash, failure, record.get('draft'))
                            assert not record['messages']
                        terminal.finish()
                    finally:
                        terminal.close()
                    for identity in (source['id'], child['id']):
                        terminal = Terminal(root, server.server_port, identity)
                        try:
                            terminal.text('Recent · Ctrl+S')
                            if expected:
                                terminal.text(expected)
                            terminal.finish()
                            record = next(r for r in terminal.sessions() if r['id'] == identity)
                            assert record.get('draft', '') == expected
                            assert not record['messages']
                        finally:
                            terminal.close()
                    print(f'branch slash={slash} save-failure={failure}: source/child/restart passed')
        assert Server.model_posts == 0, Server.model_posts
    finally:
        server.shutdown()
    print('branch drafts: four PTY paths, Unicode/resize/recovery/restart, zero model requests passed')


if __name__ == '__main__':
    main()
