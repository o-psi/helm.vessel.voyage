#!/usr/bin/env python3
"""Actual native-provider/Helm/persistence boundaries for the counting oracle."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch
from completion_gate import normalize, response

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('counting_runner', ROOT/'eval/run.py')
runner = importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)
CASE = next(case for case in runner.load() if case['id'] == 'completion-evidence-verification')
KEY = 'offline-counting-fixture-key'


class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        data = b'{"data":[{"id":"counting-fixture"}]}'
        self.send_response(200); self.send_header('Content-Length', str(len(data))); self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        case = self.server.case
        try:
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            step = len(case.requests); case.requests.append(body)
            outputs, systems = normalize(case.provider, body)
            expected_path = {'openai-chat':'/v1/chat/completions', 'openai-responses':'/v1/responses', 'anthropic':'/v1/messages'}[case.provider]
            assert self.path == expected_path, self.path
            if case.provider == 'anthropic': assert self.headers['x-api-key'] == KEY
            else: assert self.headers['Authorization'] == 'Bearer '+KEY
            assert body['model'] == 'counting-fixture'
            if case.mode != 'artifact-only' and step >= 2:
                assert 'Helm has withheld final acceptance.' in systems and case.todo in systems, 'missing targeted reconciliation'
            value = case.respond(step, outputs)
            data = response(case.provider, value, step)
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
        except Exception as error:
            case.errors.append(repr(error)); self.send_error(500)


class Case:
    def __init__(self, root, provider, mode):
        self.root, self.provider, self.mode = root, provider, mode
        self.requests, self.errors = [], []
        self.todo = self.snapshot = None
        # Independent fixture expected result. The successful result is still
        # supplied by a real policy-controlled counter invocation and checked below.
        import hashlib
        self.expected = {'claimed': 99, 'measured': 42, 'claim_matches': False,
                         'source_sha256': {name: hashlib.sha256(data.encode()).hexdigest()
                                           for name, data in CASE['seed_files'].items()}}
        self.prose = 'claims.json is incorrect: it claims 99 records, but records.csv contains 42 data rows.'

    def respond(self, step, outputs):
        report = copy.deepcopy(self.expected)
        if self.mode == 'wrong-report': report['measured'] = 142
        if self.mode == 'artifact-only':
            if step == 0: return 'write_file', {'path':'verification.json', 'content':json.dumps(report)}
            if step == 1: return 'write_file', {'path':'verification.md', 'content':self.prose}
            assert step == 2
            return '42 records verified and accounted'
        if step == 0: return 'todo', {'action':'create', 'title':'Verify the record-count claim'}
        if step == 1:
            self.todo = json.loads(outputs[-1])['id']
            return 'Premature answer: 42, everything is finished.'
        if step == 2: return 'read_file', {'path':'claims.json'}
        if step == 3:
            assert CASE['seed_files']['claims.json'] in outputs[-1]
            return 'read_file', {'path':'records.csv'}
        if step == 4:
            assert CASE['seed_files']['records.csv'] in outputs[-1]
            return 'shell', {'command':'python3 count_records.py'}
        if step == 5:
            if self.mode == 'denied-counter':
                assert 'denied' in outputs[-1].lower(), outputs[-1]
            else:
                assert outputs[-1].startswith('exit: 0\nstdout:\n'), outputs[-1]
                measured = json.loads(outputs[-1].split('stdout:\n',1)[1].split('\nstderr:\n',1)[0])
                assert measured == self.expected, measured
            return 'write_file', {'path':'verification.json', 'content':json.dumps(report)}
        if step == 6: return 'write_file', {'path':'verification.md', 'content':self.prose}
        if step == 7:
            if self.mode == 'missing-evidence': return 'todo', {'action':'list'}
            evidence = dict(self.expected)
            if self.mode == 'false-evidence': evidence['measured'] = 142
            return 'todo', {'action':'evidence', 'id':self.todo, 'text':json.dumps(evidence)}
        if step == 8: return 'todo', {'action':'status', 'id':self.todo, 'status':'completed'}
        if step == 9: return 'completion', {'action':'snapshot'}
        if step == 10:
            self.snapshot = json.loads(outputs[-1])
            assert self.snapshot['total'] == 1 and self.snapshot['accounted'] == 0
            return 'completion', {'action':'read', 'kind':'todo', 'id':self.todo}
        if step == 11:
            assert json.loads(outputs[-1])['id'] == self.todo
            return 'completion', {'action':'account', 'kind':'todo', 'id':self.todo,
                                  'revision':self.snapshot['revision'], 'fingerprint':self.snapshot['fingerprint'],
                                  'disposition':'completed_with_evidence', 'reason':'Reviewed counter output and source-attributed reports'}
        if step == 12:
            if self.mode == 'missing-evidence': assert outputs[-1] == 'tool failed: disposition does not account for todo status/evidence', outputs[-1]
            else: assert json.loads(outputs[-1]) == {'accounted':self.todo}, outputs[-1]
            if self.mode == 'stale-account': return 'todo', {'action':'note', 'id':self.todo, 'text':'Changed after review'}
        else:
            assert step == 13 and self.mode == 'stale-account', (step,self.mode)
        return '42 records: claim 99 is incorrect; reports and accounting are complete.'

    def run(self, helm):
        server = ThreadingHTTPServer(('127.0.0.1',0), Fixture); server.case = self
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            config = self.root/'config/helm/config.toml'; config.parent.mkdir(parents=True)
            config.write_text(f'''provider = "{self.provider}"
model = "counting-fixture"
api_key_env = "HELM_COUNTING_KEY"
base_url = "http://127.0.0.1:{server.server_port}/v1"
provider_retry_attempts = 1
context_window = 200000
max_tokens = 2048
access = "read-only"
'''+('deny_commands = ["python3"]\n' if self.mode == 'denied-counter' else ''))
            evidence = self.root/'evidence.json'
            oracle_errors = []
            original_oracle = runner.count_acceptance
            def observed_oracle(*args):
                try:
                    return original_oracle(*args)
                except Exception as error:
                    oracle_errors.append(repr(error))
                    raise
            with patch.object(runner, 'count_acceptance', observed_oracle), patch.dict(os.environ, {'HOME':str(self.root/'home'), 'XDG_CONFIG_HOME':str(self.root/'config'),
                                         'XDG_DATA_HOME':str(self.root/'operator-data'), 'HELM_COUNTING_KEY':KEY}):
                status = runner.run_live([CASE], str(helm), evidence, timeout=20)
            result = json.loads(evidence.read_text())['results'][0]
            assert not self.errors, self.errors
            assert result['passed'] == (self.mode == 'verified') and status == int(self.mode != 'verified'), (result, oracle_errors)
            assert result['checks']['output:42'], result
            assert result['checks']['file:verification.md:incorrect'], result
            assert result['checks']['completion-count-v1'] == (self.mode == 'verified'), result
            assert len(self.requests) == (3 if self.mode == 'artifact-only' else 14 if self.mode == 'stale-account' else 13), len(self.requests)
            assert result['exit_code'] == (1 if self.mode in ['missing-evidence','stale-account'] else 0), result
            assert result['direct_process_reaped'] and not result['timed_out'] and not result['output_limited'], result
            assert KEY not in evidence.read_text()
            assert not (self.root/'operator-data/helm/sessions').exists(), 'oracle wrote into operator data'
            if self.mode == 'verified':
                assert result['oracle_evidence']['measured'] == 42 and result['oracle_evidence']['todo_id'] == self.todo
            print('counting oracle:', self.provider, self.mode, 'passed', flush=True)
        finally:
            server.shutdown(); server.server_close(); thread.join(5)


def main():
    helm = Path(os.environ.get('HELM_BIN', ROOT/'target/release/helm')).resolve()
    assert helm.is_file(), helm
    with tempfile.TemporaryDirectory(prefix='eval-counting-fixture-') as raw:
        for provider in ('openai-chat','openai-responses','anthropic'):
            for mode in ('verified','artifact-only','wrong-report','false-evidence','missing-evidence','denied-counter','stale-account'):
                Case(Path(raw)/(provider+'-'+mode), provider, mode).run(helm)


if __name__ == '__main__':
    main()
