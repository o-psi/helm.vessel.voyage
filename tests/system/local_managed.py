#!/usr/bin/env python3
"""Private managed CLI across real processes, native HTTP and authoritative SQLite."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[2]
HELM = Path(os.environ.get('HELM_BIN', ROOT / 'target/release/helm')).resolve()


def run(args, env, expected=0):
    result = subprocess.run([str(HELM), *args], env=env, capture_output=True, text=True, timeout=20)
    assert result.returncode == expected, (args, result.returncode, result.stdout, result.stderr)
    return [json.loads(line) for line in result.stdout.splitlines() if line.strip()]


def main():
    with tempfile.TemporaryDirectory(prefix='helm-local-managed-') as temporary:
        root = Path(temporary).resolve()
        workspace = root / 'workspace'
        workspace.mkdir()
        config = root / 'config.toml'
        config.write_text('provider="openai-chat"\nmodel="managed-fixture"\n')
        env = dict(os.environ, XDG_DATA_HOME=str(root / 'data'), OPENAI_API_KEY='offline-fixture-key')
        storage = root / 'managed-installation'
        args = ['--config', str(config), '--workspace', str(workspace), 'managed', '--directory', str(storage), '--json']
        session_id = str(uuid.uuid4())
        created = run([*args, 'create', '--id', session_id, '--name', 'local managed session'], env)[0]
        assert created['event'] == 'session_created'
        assert created['session']['id'] == session_id
        assert created['session']['revision'] == 0
        listed = run([*args, 'list', '--limit', '1'], env)[0]
        assert listed['event'] == 'session_list'
        assert [session['id'] for session in listed['sessions']] == [session_id]
        assert 'messages' not in json.dumps(listed) and 'provider_state' not in json.dumps(listed)
        run([*args, 'create', '--id', session_id], env, expected=1)
        assert not list((root / 'data').glob('helm/sessions/*.json'))
    print('local managed CLI: create-only identity, bounded metadata and private SQLite authority passed')


if __name__ == '__main__':
    main()
