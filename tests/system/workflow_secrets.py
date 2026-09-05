#!/usr/bin/env python3
"""Transient workflow bindings across actual native HTTP, shell, CLI and TUI boundaries."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import tui_workflows as tui

HELM = Path(os.environ.get('HELM_BIN', 'target/release/helm')).resolve()
DOCUMENT = '''schema_version=1
id="review-change"
version="2.0"
description="Use an explicitly bound private value"
prompt="Use {{target}} only through the private shell binding. Public count={{count}}"
[parameters.count]
type="integer"
default=2
[parameters.target]
type="string"
secret=true
required=true
'''
COMMAND = '''python3 - <<'PY'
import base64, hashlib, os, pathlib
value = os.environ['HELM_WORKFLOW_TARGET'].encode()
pathlib.Path('value-digest').write_text(hashlib.sha256(value).hexdigest())
for byte in value:
    os.write(1, bytes([byte]))
os.write(2, base64.b64encode(value))
os.write(1, value.hex().encode())
PY'''


class Provider(BaseHTTPRequestHandler):
    requests = []
    failures = []
    sessions = None
    no_save = False
    denied = False
    environment_conflict = False
    before_tool = None

    def log_message(self, *_):
        pass

    def do_GET(self):
        data = json.dumps({'data': [{'id': 'workflow-fixture'}]}).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.requests.append(body)
        try:
            for tool in body['tools']:
                definition = tool['function']
                properties = definition['parameters'].get('properties', {})
                if definition['name'] == 'shell':
                    binding = properties['workflow_secrets']
                    assert binding['type'] == 'array' and binding['uniqueItems'] is True
                    assert binding['maxItems'] == 32 and binding['items']['type'] == 'string'
                    assert 'workflow_secrets' not in definition['parameters'].get('required', [])
                else:
                    assert 'workflow_secrets' not in properties
            if not self.no_save:
                saved = [json.loads(p.read_text()) for p in self.sessions.glob('*.json')]
                accepted = [s for s in saved if s.get('workflow_runs')]
                assert accepted, 'workflow metadata missing before provider bytes'
                for session in accepted:
                    invocation = session['workflow_runs'][-1]
                    assert invocation['inputs'] == {'count': 2}, 'secret persisted as workflow input'
                    assert invocation['digest'] == hashlib.sha256(DOCUMENT.encode()).hexdigest()
            tool = next((m for m in reversed(body['messages']) if m['role'] == 'tool'), None)
            if tool is None:
                callback, type(self).before_tool = type(self).before_tool, None
                if callback:
                    callback()
                delta = {'tool_calls': [{'index': 0, 'id': 'private-workflow-shell', 'type': 'function', 'function': {'name': 'shell', 'arguments': json.dumps({'command': COMMAND, 'workflow_secrets': ['target']})}}]}
            else:
                if self.environment_conflict:
                    assert 'conflicts with configured environment' in tool['content'], 'private binding replaced configured alias'
                elif self.denied:
                    assert 'denied' in tool['content'].lower() or 'policy' in tool['content'].lower(), 'private shell ignored selected profile'
                else:
                    assert json.loads(tool['content']) == {'status': 'exited', 'code': 0}, tool['content']
                delta = {'content': 'private-workflow-finished'}
            payload = ('data: ' + json.dumps({'choices': [{'delta': delta, 'finish_reason': 'stop'}]}) + '\n\ndata: [DONE]\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except Exception as error:
            self.failures.append(type(error).__name__ + ': ' + str(error))
            self.send_error(500)


def assert_private(values, *texts):
    for text in texts:
        for value in values:
            assert value not in text, 'private input leaked into a public record'


def cli_cases(root, port):
    root.mkdir()
    workflows = root / 'workflows'
    workflows.mkdir()
    (workflows / 'review-change.toml').write_text(DOCUMENT)
    config = root / 'config.toml'
    config.write_text(f'provider="openai-chat"\nmodel="workflow-fixture"\nbase_url="http://127.0.0.1:{port}/v1"\napi_key_env="FIXTURE_API_KEY"\naccess="unrestricted"\nprovider_retry_attempts=1\n')
    env = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'), FIXTURE_API_KEY='fixture-public-key')
    Provider.sessions = root / 'data/helm/sessions'
    profiles = root / 'profiles'
    common = [str(HELM), '--config', str(config), '--workspace', str(root), '--policy-directory', str(profiles)]
    captures = []

    def invoke(*args, ok=True, values=None):
        result = subprocess.run([*common, *args], env=dict(env, **(values or {})), cwd=root, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=25)
        captures.append(result.stdout + result.stderr)
        assert (result.returncode == 0) == ok, (args, result.returncode, result.stdout, result.stderr)
        return result

    def workflow(*args, **kw):
        return invoke('workflow', '--user-directory', str(workflows), *args, **kw)

    source = ['--secret-env', 'target=FIXTURE_PRIVATE_SOURCE']
    preview = workflow('--json', 'preview', 'review-change', *source)
    assert 'HELM_WORKFLOW_TARGET' in preview.stdout and not Provider.requests
    workflow('run', 'review-change', *source, ok=False)
    workflow('run', 'review-change', '--input', 'target=never-serialize-input', ok=False)
    assert not Provider.requests
    values = ['unique-private-秘密🦀', 'private-quote-"slash\\newline\nvalue']
    for value in values:
        workflow('run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': value})
        digest = root / 'value-digest'
        assert digest.read_text() == hashlib.sha256(value.encode()).hexdigest()
        digest.unlink()
    # Explicit bindings survive no-save execution, but never produce session documents.
    count = len(list(Provider.sessions.glob('*.json')))
    Provider.no_save = True
    workflow('run', 'review-change', *source, '--no-save', values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    Provider.no_save = False
    assert len(list(Provider.sessions.glob('*.json'))) == count
    (root / 'value-digest').unlink()
    # Portable reserved-name rejection happens before a private subprocess exists.
    original_config = config.read_text()
    config.write_text(original_config + '[env]\nhelm_workflow_target="configured-public-sentinel"\n')
    Provider.environment_conflict = True
    workflow('run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert not (root / 'value-digest').exists()
    Provider.environment_conflict = False
    config.write_text(original_config)
    # Read-only selected profile is stricter than the ordinary unrestricted config.
    invoke('policy', 'create', 'private-review', '--preset', 'restricted')
    snapshot = json.loads(invoke('policy', 'inspect', 'private-review').stdout)
    flags = ['--policy-profile', 'private-review', '--policy-revision', '1', '--policy-digest', snapshot['digest']]
    Provider.denied = True
    invoke(*flags, 'workflow', '--user-directory', str(workflows), 'run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert not (root / 'value-digest').exists()
    # A profile valid at admission becomes stale before tool dispatch. No private effect.
    invoke('policy', 'create', 'private-fresh', '--preset', 'autonomous')
    snapshot = json.loads(invoke('policy', 'inspect', 'private-fresh').stdout)
    flags = ['--policy-profile', 'private-fresh', '--policy-revision', '1', '--policy-digest', snapshot['digest']]
    preview = json.loads(invoke('policy', 'preview', 'private-fresh', '--revision', '1', '--digest', snapshot['digest']).stdout)['preview']
    if preview['requires_confirmation']:
        flags += ['--policy-confirm', preview['transition_digest']]
    Provider.before_tool = lambda: invoke('policy', 'delete', 'private-fresh', '--expected-revision', '1')
    # Tool denial is a structured result; a later model reply can finish the run.
    # Provider assertions require the denial, and the no-effect assertion is unchanged.
    invoke(*flags, 'workflow', '--user-directory', str(workflows), 'run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert not (root / 'value-digest').exists()
    # Persistent defaults must constrain private bindings just like explicit profiles.
    defaults = root / 'defaults'
    anchor = json.loads(invoke('policy', 'defaults', 'init', '--directory', str(defaults)).stdout)
    enabled = root / 'defaults-enabled.toml'
    invoke('policy', 'defaults', 'enable', '--directory', str(defaults), '--store-id', anchor['store_id'], '--output', str(enabled))
    common[common.index('--config') + 1] = str(enabled)

    def set_default(name, expected_revision):
        record = json.loads(invoke('policy', 'inspect', name).stdout)
        invoke('policy', 'defaults', 'set', '--global', '--profile-directory', str(profiles), name,
               '--revision', str(record['profile']['revision']), '--digest', record['digest'],
               '--expected-revision', str(expected_revision))

    set_default('private-review', 0)
    workflow('run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert not (root / 'value-digest').exists(), 'private shell bypassed restrictive defaults'
    invoke('policy', 'create', 'private-default', '--preset', 'autonomous')
    set_default('private-default', 1)
    preview = json.loads(invoke('policy', 'defaults', 'preview').stdout)['preview']
    assert preview['requires_confirmation'], 'clearing restrictive defaults requires explicit activation'
    invoke('policy', 'defaults', 'activate', '--expected-revision', '0', '--confirm', preview['transition_digest'])
    Provider.denied = False
    workflow('run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert (root / 'value-digest').read_text() == hashlib.sha256(values[0].encode()).hexdigest()
    (root / 'value-digest').unlink()
    # Even unchanged rules at a new defaults revision invalidate admitted private release.
    Provider.denied = True
    Provider.before_tool = lambda: set_default('private-default', 2)
    workflow('run', 'review-change', *source, values={'FIXTURE_PRIVATE_SOURCE': values[0]})
    assert not (root / 'value-digest').exists(), 'private shell released a secret under stale defaults'
    Provider.denied = False
    # Config display and exports are ordinary public artifacts, never secret containers.
    invoke('config')
    saved = [p.read_text() for p in Provider.sessions.glob('*.json')]
    assert_private(values + ['never-serialize-input'], json.dumps(Provider.requests, ensure_ascii=False), *captures, *saved)
    assert config.read_text().find('PRIVATE_SOURCE') == -1
    assert not Provider.failures, Provider.failures
    print('workflow secrets CLI: explicit sources, preview, private use, no-save, denial, stale-profile and persistent-defaults fencing passed')


def tui_case(root, port):
    tui.HELM = HELM
    case = tui.Case(root, port, 'private', access='unrestricted')
    Provider.sessions = case.sessions
    value = 'masked-tui-秘密🦀'
    try:
        case.path.write_text(DOCUMENT)
        case.text('HELM')
        case.open('repository')
        case.send(b'\t\x1b[200~' + value.encode() + b'\x1b[201~')
        case.text('[hidden]')
        assert_private([value], case.output.decode(errors='replace'))
        case.send('\r')
        case.text('SHA-256:')
        assert_private([value], case.output.decode(errors='replace'))
        case.send('tr')
        case.text('private-workflow-finished')
        case.wait(lambda: case.saved().get('run_summaries', [{}])[-1].get('phase') == 'completed', 'canonical completion')
        assert (root / 'value-digest').read_text() == hashlib.sha256(value.encode()).hexdigest()
        saved = case.saved()
        assert saved['workflow_runs'][0]['inputs'] == {'count': 2}
        assert_private([value], json.dumps(saved, ensure_ascii=False), json.dumps(Provider.requests, ensure_ascii=False), case.output.decode(errors='replace'))
        case.finish()
        assert not Provider.failures, Provider.failures
    finally:
        case.close()
    print('workflow secrets TUI: hidden Unicode input, digest trust, actual private shell use and public-only persistence passed')


def main():
    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='helm-workflow-secrets-') as directory:
            root = Path(directory)
            cli_cases(root / 'cli', server.server_port)
            tui_case(root / 'tui', server.server_port)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(5)


if __name__ == '__main__':
    main()
