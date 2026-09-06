"""Independent acceptance for the versioned record-counting evaluation."""
from __future__ import annotations
import copy
import csv
import hashlib
import io
import json
import os
from pathlib import Path
import uuid

from voyage_eval_io import EvidenceReader, strict_json

ORACLE = 'completion-count-v1'
COUNTER_COMMAND = 'python3 count_records.py'
TODO_FIELDS = ('id title description status priority order dependencies blockers assignees '
               'notes progress evidence created_at updated_at completed_at archived_at').split()
COUNTER = '''import csv, hashlib, json
from pathlib import Path
names = ['claims.json', 'records.csv', 'count_records.py']
raw = {name: Path(name).read_bytes() for name in names}
claim = json.loads(raw['claims.json'])['records']
with Path('records.csv').open(newline='') as io_open:
    rows = list(csv.DictReader(io_open))
result = {'claimed': claim, 'measured': len(rows), 'claim_matches': claim == len(rows),
          'source_sha256': {name: hashlib.sha256(data).hexdigest() for name, data in raw.items()}}
print(json.dumps(result, sort_keys=True))
'''


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def unsigned(value, maximum=2**64-1):
    require(type(value) is int and 0 <= value <= maximum, 'invalid bounded integer')
    return value


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def encoded(value) -> bytes:
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()


def identity(value) -> str:
    require(isinstance(value, str), 'identity must be a UUID string')
    parsed = uuid.UUID(value)
    require(parsed.int != 0 and str(parsed) == value, 'identity must be canonical and nonnil')
    return value


def expectation(seeds: dict[str, str]) -> dict:
    require(seeds.get('count_records.py') == COUNTER, 'unknown counter source')
    claim = strict_json(seeds['claims.json'])
    require(set(claim) == {'records'}, 'invalid source claim')
    unsigned(claim['records'], 1000000)
    # Independent validation counts actual well-formed rows and distinct IDs,
    # rather than trusting the counter output or the model's number.
    rows = list(csv.reader(io.StringIO(seeds['records.csv'], newline='')))
    require(rows and rows[0] == ['record_id', 'value'], 'invalid record header')
    records = rows[1:]
    require(0 < len(records) <= 1000, 'record count outside scenario bounds')
    require(all(len(row) == 2 and row[0] and row[1] for row in records), 'invalid data row')
    require(len({row[0] for row in records}) == len(records), 'duplicate record identity')
    require(claim['records'] != len(records), 'counting scenario requires an incorrect source claim')
    return {'claimed': claim['records'], 'measured': len(records),
            'claim_matches': claim['records'] == len(records),
            'source_sha256': {name: sha(seeds[name].encode()) for name in
                              ('claims.json', 'records.csv', 'count_records.py')}}


def report_matches(value, expected):
    return (isinstance(value, dict) and value == expected
            and type(value.get('claimed')) is int and type(value.get('measured')) is int
            and type(value.get('claim_matches')) is bool)


def successful_calls(messages):
    pending, results = {}, []
    for message in messages:
        calls = message.get('tool_calls', [])
        if calls:
            require(message['role'] == 'assistant', 'tool calls outside assistant message')
        for call in calls:
            require(isinstance(call['id'], str) and call['id'] and call['id'] not in pending,
                    'ambiguous pending tool call')
            pending[call['id']] = call
        if message['role'] == 'tool':
            call = pending.pop(message.get('tool_call_id'), None)
            require(call is not None, 'tool result has no matching call')
            if message.get('tool_success') is True:
                results.append((call, message['content']))
    require(not pending, 'unfinished tool calls')
    return results


def count_acceptance(case: dict, workspace: Path, data_root: Path) -> dict:
    expected = expectation(case['seed_files'])
    with EvidenceReader(workspace) as artifacts, EvidenceReader(data_root) as data:
        for name, original in case['seed_files'].items():
            require(artifacts.read(name) == original.encode(), 'seed source was changed')
        report_bytes = artifacts.read('verification.json', 65536)
        require(report_matches(strict_json(report_bytes), expected), 'report differs from independently counted evidence')
        prose = artifacts.read('verification.md', 65536).decode('utf-8')
        require(all(term in prose for term in ['claims.json', 'records.csv', str(expected['claimed']),
                                              str(expected['measured'])]) and 'incorrect' in prose.lower(),
                'human report lacks source attribution or incorrect-claim explanation')
        sessions = data.files('helm/sessions', '.json')
        require(len(sessions) == 1, 'one saved evaluation session required')
        saved_bytes = data.read(sessions[0])
        saved = strict_json(saved_bytes)
        session_id = identity(saved['id'])
        require(sessions[0] == f'helm/sessions/{session_id}.json', 'session filename differs from identity')
        require(saved['workspace'] == str(workspace.resolve()), 'session belongs to another workspace')
        refs, summaries = saved['completion_runs'], saved['run_summaries']
        require(len(refs) == len(summaries) == 1, 'one current run and summary required')
        reference, summary = refs[0], summaries[0]
        run_id = identity(reference['run_id'])
        require(reference['session_id'] == session_id and summary['run_id'] == run_id,
                'session/run summary identity mismatch')
        require(summary['phase'] == 'completed' and not summary.get('partial_output'),
                'canonical run is not accepted completed')
        messages = saved['messages']
        require([m['content'] for m in messages if m['role'] == 'user'] == [case['prompt']],
                'canonical prompt differs from evaluation')
        require(not any(m['role'] == 'system' for m in messages), 'runtime guidance persisted as history')
        require(unsigned(summary['message_start']) == 0 and unsigned(summary['message_end']) == len(messages),
                'accepted canonical range is missing or ambiguous')
        message_hashes = [sha(json.dumps({'role': message['role'], 'content': message['content'],
                            'tool_calls': message.get('tool_calls', []),
                            'tool_call_id': message.get('tool_call_id')}, sort_keys=True,
                            ensure_ascii=False, separators=(',', ':')).encode()) for message in messages]
        require(summary['message_fingerprints'] == message_hashes, 'accepted canonical messages changed')
        calls = successful_calls(messages)
        allowed = {'read_file', 'write_file', 'shell', 'todo', 'completion'}
        for message in messages:
            for call in message.get('tool_calls', []):
                require(call['name'] in allowed, 'unexpected tool in counting evaluation')
                if call['name'] == 'shell':
                    require(call['arguments'] == {'command': COUNTER_COMMAND}, 'unapproved counting command')
                if call['name'] == 'write_file':
                    require(call['arguments'].get('path') in ('verification.json', 'verification.md'),
                            'unexpected artifact write')
        reads = {call['arguments'].get('path') for call, output in calls if call['name'] == 'read_file'
                 and call['arguments'].get('path') in case['seed_files']
                 and output == 'sha256: ' + expected['source_sha256'][call['arguments']['path']] + '\n'
                     + case['seed_files'][call['arguments']['path']]}
        require({'claims.json', 'records.csv'} <= reads, 'source inspection was not successful')
        counts = []
        for call, output in calls:
            if call['name'] == 'shell':
                require(output.startswith('exit: 0\nstdout:\n') and '\nstderr:\n' in output,
                        'counter did not exit successfully')
                stdout, stderr = output[len('exit: 0\nstdout:\n'):].split('\nstderr:\n', 1)
                require(not stderr.strip(), 'counter diagnostics are not counting evidence')
                require(report_matches(strict_json(stdout), expected), 'counter result differs from independent count')
                counts.append(call['id'])
        require(len(counts) == 1, 'one successful counter execution required')
        writes = {call['arguments']['path']: call['arguments'].get('content') for call, _ in calls
                  if call['name'] == 'write_file'}
        require(writes.get('verification.json') == report_bytes.decode('utf-8')
                and writes.get('verification.md') == prose, 'reports lack matching successful persisted writes')
        key = sha(os.fsencode(workspace.resolve()))
        ledger_name = f'helm/completion/{key}/ledgers/{session_id}-{run_id}.json'
        ledger_bytes = data.read(ledger_name)
        envelope = strict_json(ledger_bytes)
        require(unsigned(envelope['version']) == 1 and envelope['scope'] == {
            'workspace': str(workspace.resolve()), 'session_id': session_id}, 'wrong ledger scope/version')
        ledger = strict_json(envelope['ledger'])
        require(unsigned(ledger['version']) == 2 and ledger['run_id'] == run_id, 'wrong ledger identity/version')
        require(ledger['state']['status'] == 'sealed', 'completion ledger remains open')
        decision = ledger['state']['decision']
        require(decision['outcome'] == 'completed' and decision['reason'] is None, 'ledger did not accept completion')
        readiness = decision['readiness']
        require(readiness['run_id'] == run_id and unsigned(readiness['revision']) + 1 == unsigned(ledger['revision']),
                'decision does not match current ledger revision')
        require(all(unsigned(readiness[key], 1024) == 1 for key in ('total', 'accounted', 'completed'))
                and all(unsigned(readiness[key], 1024) == 0 for key in ('incomplete', 'omitted_unresolved'))
                and readiness['unresolved'] == readiness['incomplete_obligations'] == [],
                'completion has missing or unfinished obligations')
        require(len(ledger['entries']) == 1, 'one owned verification todo required')
        entry = ledger['entries'][0]
        require(set(entry['obligation']) == {'todo'}, 'unexpected owned obligation')
        todo_id = identity(entry['obligation']['todo'])
        todo_bytes = data.read(f'helm/todos/{key}.json')
        todos = strict_json(todo_bytes)
        require(unsigned(todos['version']) == 1, 'unknown todo version')
        unsigned(todos['revision'])
        require(todos['scope'] == {'workspace': str(workspace.resolve()), 'session_id': None}, 'todo scope differs from evaluation')
        require(set(todos['items']) == {todo_id}, 'missing or unrelated todo evidence')
        item = todos['items'][todo_id]
        require(set(item) == set(TODO_FIELDS) and item['id'] == todo_id, 'unknown todo record shape')
        require(item['status'] == 'completed' and item['completed_at'] and item['archived_at'] is None
                and not item['blockers'], 'verification todo remains unfinished or hidden')
        evidence = []
        for value in item['evidence']:
            try:
                evidence.append(strict_json(value['text']))
            except (ValueError, TypeError):
                continue
        require(any(report_matches(value, expected) for value in evidence), 'todo lacks exact measured structured evidence')
        counter_index = next(i for i, (call, _) in enumerate(calls) if call['id'] == counts[0])
        evidence_calls = [i for i, (call, _) in enumerate(calls) if call['name'] == 'todo'
                          and call['arguments'].get('action') == 'evidence'
                          and call['arguments'].get('id') == todo_id
                          and report_matches(strict_json(call['arguments']['text']), expected)]
        require(evidence_calls and min(evidence_calls) > counter_index,
                'measured evidence was not recorded after successful counting')
        current = sha(encoded({field: item[field] for field in TODO_FIELDS}))
        require(entry['dispositions'] and len(entry['dispositions']) <= 16, 'missing accounting')
        disposition = entry['dispositions'][-1]
        require(disposition['kind'] == 'completed_with_evidence' and disposition['reviewed'] == current
                and disposition['reason'].strip(), 'accounting is stale or does not verify this todo')
        require(any(call['name'] == 'completion' and call['arguments'].get('action') == 'account'
                    and call['arguments'].get('id') == todo_id
                    and call['arguments'].get('disposition') == 'completed_with_evidence'
                    and strict_json(output) == {'accounted': todo_id}
                    for call, output in calls), 'no successful completion accounting call')
        # Independently reconstruct the exact pre-seal snapshot fingerprint. The
        # final decision must refer to these records, not a prior accepted run.
        open_ledger = {field: copy.deepcopy(ledger[field]) for field in
                       ('version', 'run_id', 'revision', 'entries', 'state')}
        open_ledger['revision'] -= 1
        open_ledger['state'] = {'status': 'open'}
        fingerprint = sha(encoded([open_ledger, [[{'todo': todo_id}, current]]]))
        require(readiness['fingerprint'] == fingerprint, 'sealed decision fingerprint is stale')
        return {'oracle': ORACLE, 'session_id': session_id, 'run_id': run_id, 'todo_id': todo_id,
                'measured': expected['measured'], 'claimed': expected['claimed'],
                'source_sha256': expected['source_sha256'], 'report_sha256': sha(report_bytes),
                'session_sha256': sha(saved_bytes), 'ledger_sha256': sha(ledger_bytes),
                'todo_store_sha256': sha(todo_bytes), 'reviewed_fingerprint': current,
                'counter_call_id': counts[0]}
