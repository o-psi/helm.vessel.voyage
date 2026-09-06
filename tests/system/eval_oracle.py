#!/usr/bin/env python3
"""Lightweight independent adversarial evidence and bounded-runner tests."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'eval'))
from voyage_eval_io import EvidenceReader, strict_json, MAX_FILE
from voyage_eval_oracle import count_acceptance
from voyage_eval_process import execute
spec = importlib.util.spec_from_file_location('evaluation_runner', ROOT / 'eval/run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
CASE = next(case for case in runner.load() if case['id'] == 'completion-evidence-verification')
SID, RID, TID = ('00000000-0000-4000-8000-00000000000' + str(i) for i in range(1, 4))


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()


class StoredEvidence:
    def __init__(self, root):
        self.workspace, self.data = root/'workspace', root/'data'
        self.workspace.mkdir(exist_ok=True); self.data.mkdir(exist_ok=True)
        for name, content in CASE['seed_files'].items():
            (self.workspace/name).write_text(content)
        self.report = {'claimed': 99, 'measured': 42, 'claim_matches': False,
                       'source_sha256': {name: hashlib.sha256(content.encode()).hexdigest()
                                         for name, content in CASE['seed_files'].items()}}
        (self.workspace/'verification.json').write_text(json.dumps(self.report))
        (self.workspace/'verification.md').write_text('claims.json is incorrect: 99 claimed, 42 counted in records.csv.')
        when = '2026-09-06T00:00:00Z'
        self.item = {'id': TID, 'title': 'Verify measured records', 'description': '', 'status': 'completed',
                     'priority': 'normal', 'order': 0, 'dependencies': [], 'blockers': [], 'assignees': [],
                     'notes': [], 'progress': [], 'evidence': [{'at': when, 'author': None, 'text': json.dumps(self.report)}],
                     'created_at': when, 'updated_at': when, 'completed_at': when, 'archived_at': None}
        self.ledger = {'version': 2, 'run_id': RID, 'revision': 3,
                       'entries': [{'obligation': {'todo': TID}, 'dispositions': [{
                           'kind': 'completed_with_evidence', 'reason': 'Counter and reports verified',
                           'reviewed': digest(self.item)}]}], 'state': {'status': 'open'}}
        before = copy.deepcopy(self.ledger); before['revision'] = 2
        readiness = {'run_id': RID, 'revision': 2,
                     'fingerprint': digest([before, [[{'todo': TID}, digest(self.item)]]]),
                     'total': 1, 'accounted': 1, 'completed': 1, 'incomplete': 0,
                     'incomplete_obligations': [], 'unresolved': [], 'omitted_unresolved': 0}
        self.ledger['state'] = {'status': 'sealed', 'decision': {
            'outcome': 'completed', 'readiness': readiness, 'reason': None}}
        self.messages = [{'role': 'user', 'content': CASE['prompt']}]
        def tool(name, arguments, result):
            identity = 'call-' + str(len(self.messages))
            self.messages.extend([{'role': 'assistant', 'content': '', 'tool_calls': [
                {'id': identity, 'name': name, 'arguments': arguments}]},
                {'role': 'tool', 'content': result, 'tool_call_id': identity, 'tool_success': True}])
        tool('todo', {'action': 'create', 'title': 'Verify measured records'}, json.dumps({'id': TID}))
        for name in ('claims.json', 'records.csv'):
            tool('read_file', {'path': name}, 'sha256: '+self.report['source_sha256'][name]+'\n'+CASE['seed_files'][name])
        tool('shell', {'command': 'python3 count_records.py'},
             'exit: 0\nstdout:\n' + json.dumps(self.report) + '\n\nstderr:\n')
        for name in ('verification.json', 'verification.md'):
            tool('write_file', {'path': name, 'content': (self.workspace/name).read_text()}, 'written')
        tool('todo', {'action': 'evidence', 'id': TID, 'text': json.dumps(self.report)}, json.dumps(self.item))
        tool('todo', {'action': 'status', 'id': TID, 'status': 'completed'}, json.dumps(self.item))
        tool('completion', {'action': 'account', 'kind': 'todo', 'id': TID,
                            'disposition': 'completed_with_evidence'}, json.dumps({'accounted': TID}))
        self.messages.append({'role': 'assistant', 'content': 'Measured 42 records; claim 99 is incorrect.'})
        self.session = {'id': SID, 'workspace': str(self.workspace), 'messages': self.messages,
                        'completion_runs': [{'session_id': SID, 'run_id': RID}],
                        'run_summaries': [{'run_id': RID, 'phase': 'completed', 'partial_output': '',
                                           'message_start': 0, 'message_end': len(self.messages)}]}
        self.refresh_messages()
        self.key = hashlib.sha256(os.fsencode(self.workspace)).hexdigest()
        self.session_path = self.data/f'helm/sessions/{SID}.json'
        self.ledger_path = self.data/f'helm/completion/{self.key}/ledgers/{SID}-{RID}.json'
        self.todo_path = self.data/f'helm/todos/{self.key}.json'
        self.save()

    def refresh_messages(self):
        self.session['run_summaries'][0]['message_fingerprints'] = [hashlib.sha256(json.dumps({
            'role': message['role'], 'content': message['content'],
            'tool_calls': message.get('tool_calls', []), 'tool_call_id': message.get('tool_call_id')},
            sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest() for message in self.messages]

    def save(self):
        values = [(self.session_path, self.session), (self.ledger_path, {'version': 1,
            'scope': {'workspace': str(self.workspace), 'session_id': SID}, 'ledger': json.dumps(self.ledger)}),
            (self.todo_path, {'version': 1, 'revision': 3,
             'scope': {'workspace': str(self.workspace), 'session_id': None}, 'items': {TID: self.item}})]
        for path, value in values:
            path.parent.mkdir(parents=True, exist_ok=True); path.write_text(json.dumps(value))

    def check(self):
        return count_acceptance(CASE, self.workspace, self.data)


class OracleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(); self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.fixture = StoredEvidence(self.root)

    def rejected(self):
        with self.assertRaises((ValueError, OSError, KeyError, TypeError)):
            self.fixture.check()

    def test_current_count_artifacts_and_accounting(self):
        result = self.fixture.check()
        self.assertEqual((result['measured'], result['run_id'], result['todo_id']), (42, RID, TID))

    def test_report_claim_is_not_an_oracle(self):
        for value in (99, 142, True, 42.0):
            with self.subTest(value=value):
                report = dict(self.fixture.report, measured=value)
                (self.fixture.workspace/'verification.json').write_text(json.dumps(report)); self.rejected()

    def test_seed_tampering(self):
        (self.fixture.workspace/'records.csv').write_text('record_id,value\nfake,42\n'); self.rejected()

    def test_counter_invocation_and_result_required(self):
        for key, value in [('name', 'read_file'), ('arguments', {'command': 'printf 42'})]:
            with self.subTest(key=key):
                before = copy.deepcopy(self.fixture.messages)
                self.fixture.messages[7]['tool_calls'][0][key] = value
                self.fixture.refresh_messages(); self.fixture.save(); self.rejected()
                self.fixture.messages[:] = before
        self.fixture.messages[8]['tool_success'] = False
        self.fixture.refresh_messages(); self.fixture.save(); self.rejected()

    def test_nonzero_counter_is_not_success(self):
        self.fixture.messages[8]['content'] = self.fixture.messages[8]['content'].replace('exit: 0', 'exit: 1')
        self.fixture.refresh_messages(); self.fixture.save(); self.rejected()

    def test_canonical_mutation_is_not_accepted(self):
        self.fixture.messages[8]['content'] = 'exit: 0\nstdout:\n42\nstderr:\n'
        self.fixture.save(); self.rejected()

    def test_missing_open_incomplete_wrong_run_ledger(self):
        original = copy.deepcopy(self.fixture.ledger)
        for change in ('open', 'incomplete', 'run', 'no-account', 'stale'):
            with self.subTest(change=change):
                self.fixture.ledger = copy.deepcopy(original)
                if change == 'open': self.fixture.ledger['state'] = {'status': 'open'}
                elif change == 'incomplete': self.fixture.ledger['state']['decision']['outcome'] = 'incomplete'
                elif change == 'run': self.fixture.ledger['run_id'] = SID
                elif change == 'no-account': self.fixture.ledger['entries'][0]['dispositions'] = []
                else: self.fixture.ledger['entries'][0]['dispositions'][0]['reviewed'] = '0'*64
                self.fixture.save(); self.rejected()
        self.fixture.ledger_path.unlink(); self.rejected()

    def test_numeric_bool_negative_float_and_overflow(self):
        decision = self.fixture.ledger['state']['decision']
        for field in ('revision', 'total', 'accounted', 'completed', 'incomplete', 'omitted_unresolved'):
            original = decision['readiness'][field]
            for value in (True, -1, 1.0, 2**65):
                with self.subTest(field=field, value=value):
                    decision['readiness'][field] = value; self.fixture.save(); self.rejected()
            decision['readiness'][field] = original

    def test_empty_wrong_stale_and_hidden_todo_evidence(self):
        original = copy.deepcopy(self.fixture.item)
        for change in ('empty', 'fake', 'pending', 'archived', 'changed'):
            with self.subTest(change=change):
                self.fixture.item = copy.deepcopy(original)
                if change == 'empty': self.fixture.item['evidence'] = []
                elif change == 'fake': self.fixture.item['evidence'][0]['text'] = '42'
                elif change == 'pending': self.fixture.item['status'] = 'pending'
                elif change == 'archived': self.fixture.item['archived_at'] = '2026-09-06T00:00:00Z'
                else: self.fixture.item['title'] = 'changed after accounting'
                self.fixture.save(); self.rejected()

    def test_malformed_and_nil_identities(self):
        for value in ('../escape', '00000000-0000-0000-0000-000000000000', True, None, 'A'*36):
            with self.subTest(value=value):
                self.fixture.session['completion_runs'][0]['run_id'] = value
                self.fixture.save(); self.rejected()

    def test_unrelated_session_or_todo_scope(self):
        self.fixture.session['completion_runs'][0]['session_id'] = TID
        self.fixture.save(); self.rejected()

    def test_unsafe_artifact_kinds_and_sizes(self):
        path = self.fixture.workspace/'verification.json'
        for kind in ('symlink', 'fifo', 'oversized', 'directory'):
            with self.subTest(kind=kind):
                path.unlink()
                if kind == 'symlink': path.symlink_to(self.fixture.session_path)
                elif kind == 'fifo': os.mkfifo(path)
                elif kind == 'oversized':
                    with path.open('wb') as output: output.truncate(MAX_FILE+1)
                else: path.mkdir()
                self.rejected()
                if kind == 'directory': path.rmdir(); path.write_text('{}')

    def test_duplicate_json_keys_fail(self):
        self.fixture.session_path.write_text('{"id":"a","id":"b"}')
        self.rejected()
        for invalid in ('{"x":NaN}', '{"x":1e9999}', '{"x":-Infinity}'):
            with self.assertRaises(ValueError): strict_json(invalid)

    def test_reader_traversal_and_ancestor_link(self):
        (self.fixture.workspace/'outside').symlink_to(self.fixture.data, target_is_directory=True)
        with EvidenceReader(self.fixture.workspace) as reader:
            for name in ('../verification.json', '/etc/passwd', 'a/../verification.json',
                         'outside/helm/sessions/'+SID+'.json', 'a\\b'):
                with self.subTest(name=name), self.assertRaises((ValueError, OSError)):
                    reader.read(name)

    def test_fake_successful_read_does_not_prove_source_inspection(self):
        self.fixture.messages[4]['content'] = '42'
        self.fixture.refresh_messages(); self.fixture.save(); self.rejected()

    def test_artifact_without_successful_write_record(self):
        self.fixture.messages[10]['tool_success'] = False
        self.fixture.refresh_messages(); self.fixture.save(); self.rejected()

    def test_directory_and_aggregate_limits(self):
        from unittest.mock import patch
        directory = self.fixture.workspace/'many'; directory.mkdir()
        for number in range(3): (directory/f'{number}.json').write_text('{}')
        with EvidenceReader(self.fixture.workspace) as reader:
            with self.assertRaises(ValueError): reader.files('many', '.json', limit=2)
        with patch('voyage_eval_io.MAX_TOTAL', 3), EvidenceReader(self.fixture.workspace) as reader:
            self.assertEqual(reader.read('many/0.json'), b'{}')
            with self.assertRaises(ValueError): reader.read('many/1.json')



class ProcessTests(unittest.TestCase):
    def test_scenario_selection_is_explicit_and_validated(self):
        command = [sys.executable, str(ROOT/'eval/run.py'), 'validate', '--scenario', CASE['id']]
        result = subprocess.run(command, capture_output=True, text=True, timeout=2)
        self.assertEqual(result.returncode, 0)
        self.assertIn('validated 1 scenarios across 1 categories', result.stdout)
        for arguments in (['--scenario','unknown'], ['--scenario',CASE['id']]):
            result = subprocess.run(command+arguments, capture_output=True, text=True, timeout=2)
            self.assertEqual(result.returncode, 2)
            self.assertIn('distinct known', result.stderr)

    def test_malformed_nested_record_fails_and_next_scenario_runs(self):
        from unittest.mock import patch
        calls = []
        def fixture_execution(command, workspace, env, timeout):
            calls.append(command)
            if len(calls) == 1:
                stored = StoredEvidence(workspace.parent)
                stored.ledger['entries'][0]['dispositions'][-1]['reason'] = 7
                stored.save()
            return {'stdout': '42', 'stderr': '', 'exit_code': 0,
                    'timed_out': False, 'output_limited': False,
                    'cleanup_error': None, 'direct_process_reaped': True}
        with tempfile.TemporaryDirectory() as raw, patch.object(runner, 'execute', fixture_execution):
            evidence = Path(raw)/'evidence.json'
            following = {'id': 'after-malformed', 'category': 'data',
                         'prompt': 'following case', 'expect_output': ['42']}
            self.assertEqual(runner.run_live([CASE, following], '/fixture/helm', evidence, 2), 1)
            results = json.loads(evidence.read_text())['results']
            self.assertEqual(len(calls), 2)
            self.assertEqual([result['passed'] for result in results], [False, True])
            self.assertFalse(results[0]['checks']['completion-count-v1'])

    def test_small_output_and_nonzero(self):
        result = execute([sys.executable, '-c', 'import sys; print("42"); sys.exit(2)'], ROOT, os.environ.copy(), 2)
        self.assertEqual(result['exit_code'], 2)
        self.assertTrue(result['direct_process_reaped'])

    def test_flood_and_timeout_are_bounded_failures(self):
        for script, expected in [('import os; os.write(1,b"x"*2000000)', 'output_limited'),
                                 ('import time; print("partial42",flush=True); time.sleep(5)', 'timed_out')]:
            started = time.monotonic()
            result = execute([sys.executable, '-c', script], ROOT, os.environ.copy(), .2)
            self.assertTrue(result[expected], result)
            self.assertTrue(result['direct_process_reaped'])
            self.assertLessEqual(len(result['stdout']), 1024*1024)
            self.assertLess(time.monotonic()-started, 3)

    def test_old_false_positive_and_stderr_only_are_rejected(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw); fake = root/'fake-helm'
            fake.write_text('#!/bin/sh\nprintf 42 > verification.md\nprintf 42 >&2\n'); fake.chmod(0o700)
            cases = [CASE, {'id': 'stderr-only', 'category': 'data', 'prompt': 'irrelevant', 'expect_output': ['42']}]
            self.assertEqual(runner.run_live(cases, str(fake), root/'evidence.json', 2), 1)
            results = json.loads((root/'evidence.json').read_text())['results']
            self.assertEqual([r['passed'] for r in results], [False, False])
            self.assertFalse(results[1]['checks']['output:42'])
            fake.write_text('#!/bin/sh\nprintf incorrect42 > verification.md\nprintf 42\n')
            self.assertEqual(runner.run_live([CASE], str(fake), root/'artifact-only.json', 2), 1)
            result = json.loads((root/'artifact-only.json').read_text())['results'][0]
            self.assertTrue(result['checks']['output:42'])
            self.assertTrue(result['checks']['file:verification.md:incorrect'])
            self.assertFalse(result['checks']['completion-count-v1'])


if __name__ == '__main__':
    unittest.main()
