"""Exact published v1.0.2 public-message projection; no reads, effects or retries.

Source: v1.0.2 bba5f4b1d8d7561d30030c90a4244deb73e27702,
voyage/src/attachment/runtime/process/projection.rs full/bounded/recent.
The caller separately requires complete raw DB before/after equality.
"""
import hashlib
import json
import uuid
from pathlib import Path
import os
import threading
import time

PUBLIC_FIELDS = frozenset(('interrupted_attempt', 'coordination', 'role', 'content',
    'parts', 'tool_output', 'created_at', 'operator_name', 'tool_calls',
    'tool_call_id', 'tool_outcome', 'tool_success', 'steering'))
CANONICAL_FIELDS = PUBLIC_FIELDS | {'provider_state'}
NULL_FIELDS = PUBLIC_FIELDS - {'role', 'content', 'parts', 'tool_calls'}


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def canonical_bytes(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False,
                         separators=(',', ':'), allow_nan=False).encode()


def digest(value):
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def full_public_message(message):
    require(isinstance(message, dict) and set(message) <= CANONICAL_FIELDS,
            'unknown canonical legacy message shape')
    require(message.get('role') in ('system', 'user', 'assistant', 'tool')
            and isinstance(message.get('content'), str), 'legacy message text or role unavailable')
    result = {key: message.get(key) for key in NULL_FIELDS}
    result.update(role=message['role'], content=message['content'],
                  parts=message.get('parts', []), tool_calls=message.get('tool_calls', []))
    require(isinstance(result['parts'], list) and isinstance(result['tool_calls'], list),
            'legacy content/tool references unavailable')
    # Old public projection deliberately exposes interruption identity only for
    # assistant history, with the same typed UUID/non-nil rule as the old reader.
    interrupted = message.get('interrupted_attempt')
    if interrupted is not None:
        parsed = uuid.UUID(interrupted)
        result['interrupted_attempt'] = str(parsed) if message['role'] == 'assistant' and parsed.int else None
    return result


def verify_snapshot(snapshot, session, canonical_messages, expected_revision=None):
    require(isinstance(canonical_messages, list), 'canonical legacy history unavailable')
    require(isinstance(snapshot, dict) and snapshot.get('projection') == 'public-v1'
            and snapshot.get('observation') == 'snapshot'
            and snapshot.get('session_id') == session
            and type(snapshot.get('revision')) is int and snapshot['revision'] >= 0
            and (expected_revision is None or snapshot['revision'] == expected_revision), 'old Snapshot projection/session mismatch')
    require(type(snapshot.get('total_messages')) is int
            and snapshot['total_messages'] == len(canonical_messages)
            and type(snapshot.get('message_offset')) is int and snapshot['message_offset'] == 0
            and snapshot.get('history_truncated') is False,
            'old Snapshot is not complete untruncated history')
    messages = snapshot.get('messages')
    require(isinstance(messages, list) and len(messages) == len(canonical_messages),
            'old Snapshot message range mismatch')
    expected = []
    for index, message in enumerate(canonical_messages):
        value = full_public_message(message)
        value.update(message_index=index, projection_truncated=False)
        expected.append(value)
    require(canonical_bytes(messages) == canonical_bytes(expected),
            'old public history text/order/identity/metadata mismatch')
    # This is only a source-bound public-read witness, not a DB preservation or
    # rollback claim. Opaque provider replay state is absent from public messages;
    # raw before/after equality remains mandatory in the qualification caller.
    return {'projection': 'public-v1', 'session_id': session, 'messages': len(messages),
            'canonical_messages_sha256': digest(canonical_messages),
            'public_messages_sha256': digest(messages),
            'full_text_order_identity_verified': True, 'history_truncated': False}


def read_json(path, limit=1024*1024):
    with path.open('rb') as stream:
        value = stream.read(limit + 1)
    require(len(value) <= limit, 'retained qualification evidence exceeds bound')
    return json.loads(value)


def release_identity(manifest):
    # Exact published Manifest::id serialization: BTreeMap keys sorted, struct
    # field order fixed, compatibility removed and empty assets omitted.
    value = {key: manifest[key] for key in ('schema_version', 'version', 'target')}
    value['binaries'] = {key: {'sha256': manifest['binaries'][key]['sha256']}
                         for key in sorted(manifest['binaries'])}
    if manifest.get('assets'):
        value['assets'] = {key: {'sha256': manifest['assets'][key]['sha256']}
                           for key in sorted(manifest['assets'])}
    encoded = json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()
    return hashlib.sha256(encoded).hexdigest()


def original_target(q, operation_id=None):
    installer = Path(read_json(q.WORK / 'candidate-bin.json'))
    require(installer.is_relative_to(q.WORK / 'candidate')
            and installer.resolve(strict=True) == installer, 'qualified candidate staging identity changed')
    manifest = read_json(installer.parent.parent / 'release.json')
    target = release_identity(manifest)
    require(q.digest(installer) == manifest['binaries']['voyage-installer']['sha256']
            and q.digest(installer.with_name('vessel')) == manifest['binaries']['vessel']['sha256'],
            'qualified original candidate image changed')
    paths = list((q.INSTALL / 'updates').glob('*.json'))
    require(len(paths) <= 16, 'original operation inventory exceeds bound')
    matches = []
    for path in paths:
        record = read_json(path, 65536)
        if operation_id is not None and record.get('operation_id') != operation_id:
            continue
        if record.get('legacy_mode') is True and record.get('channel') == 'local-owner':
            op = record.get('operation_id')
            require(isinstance(op, str) and str(uuid.UUID(op)) == op and uuid.UUID(op).int
                    and path.name == op + '.json', 'original operation identity malformed')
            matches.append(record)
    require(len(matches) == 1, 'original local-owner operation is ambiguous or missing')
    record = matches[0]
    require(record.get('release_id') == target
            and record.get('current_release') == q.installed_bin().parent.name,
            'original operation/qualified target/restored pointer conflict')
    return record, installer, manifest


def original_identity_retired(q, witness):
    require(isinstance(witness, dict) and type(witness.get('pid')) is int
            and witness['pid'] > 1 and type(witness.get('start_ticks')) is int
            and witness['start_ticks'] > 0 and witness.get('uid') == 1000,
            'original owned process witness unavailable')
    try:
        current = q.process_start(witness['pid'], allow_exited=True)
    except (FileNotFoundError, ProcessLookupError):
        return True
    # A live or unreaped zombie with the same start remains an obligation. A
    # replacement PID belongs to another identity and is never signalled here.
    return current != witness['start_ticks']


class ObserverWatch:
    """Read-only witness of direct children of the exact restored supervisor."""
    def __init__(self, q, supervisor, started, sessions):
        self.q, self.supervisor, self.started = q, supervisor, started
        self.binary = q.installed_bin() / 'voyage'
        self.expected_sha = q.digest(self.binary)
        self.image = self.binary.stat()
        self.directories = {os.fsencode(q.STATE / 'sessions' / session) for session in sessions}
        self.witnesses, self.errors = {}, []
        self.active_read, self.reads = None, []
        self.stop = threading.Event()
        self.deadline = time.monotonic() + 35
        self.closed, self.outcome = False, None
        try:
            self.baseline = self.scan()
            self.thread = threading.Thread(target=self.observe, daemon=True)
            self.thread.start()
        except BaseException:
            for witness in self.witnesses.values():os.close(witness['descriptor'])
            self.witnesses.clear()
            raise

    def children(self):
        require(self.q.process_start(self.supervisor) == self.started, 'restored supervisor replaced during child observation')
        tasks = list((Path('/proc') / str(self.supervisor) / 'task').iterdir())
        require(len(tasks) <= 256, 'supervisor task inventory exceeds bound')
        children = set()
        for task in tasks:
            try:
                raw = (task / 'children').read_bytes()
            except FileNotFoundError:
                continue
            require(len(raw) <= 8192, 'supervisor child inventory exceeds bound')
            children.update(int(pid) for pid in raw.split())
        require(len(children) <= 256, 'supervisor child count exceeds bound')
        return children

    def scan(self):
        identities = set()
        for pid in self.children():
            descriptor = None
            try:
                started = self.q.process_start(pid, allow_exited=True)
                identities.add((pid, started))
                proc = Path('/proc') / str(pid)
                raw = (proc / 'cmdline').read_bytes()
                require(len(raw) <= 65536, 'owned child argv exceeds bound')
                arguments = raw.rstrip(b'\0').split(b'\0')
                if (len(arguments) != 4 or arguments[:3] != [os.fsencode(self.binary), b'observe-suspended', b'--directory']
                        or arguments[3] not in self.directories):
                    continue
                key = (pid, started)
                if key in self.witnesses:
                    continue
                require(len(self.witnesses) < 64, 'observer witness count exceeds bound')
                descriptor = os.pidfd_open(pid)
                status = (proc / 'status').read_text()
                require(len(status) <= 65536 and proc.stat().st_uid == 1000
                        and next((line.split()[1:] for line in status.splitlines() if line.startswith('Uid:')), []) == ['1000']*4,
                        'owned observer UID changed')
                fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
                require(int(fields[1]) == self.supervisor and self.q.process_start(pid) == started,
                        'observer direct parent/start identity changed')
                image = (proc / 'exe').stat()
                require(os.readlink(proc / 'exe') == str(self.binary)
                        and (image.st_dev,image.st_ino,image.st_uid,image.st_mode,image.st_size,image.st_mtime_ns,image.st_ctime_ns)
                        == (self.image.st_dev,self.image.st_ino,self.image.st_uid,self.image.st_mode,self.image.st_size,self.image.st_mtime_ns,self.image.st_ctime_ns)
                        and (proc / 'cmdline').read_bytes() == raw,
                        'observer does not execute exact qualified old image/argv')
                interval = self.active_read
                attribution = interval[0] if interval is not None and arguments[3] == os.fsencode(self.q.STATE / 'sessions' / interval[1]) else None
                self.witnesses[key] = {'descriptor':descriptor,'pid':pid,'start_ticks':started,'uid':1000,
                    'parent_pid':self.supervisor,'parent_start_ticks':self.started,
                    'executable_sha256':self.expected_sha,'argv_sha256':hashlib.sha256(raw).hexdigest(),
                    'snapshot_read_interval':attribution, 'session_id':Path(os.fsdecode(arguments[3])).name}
                descriptor = None
            except (OSError, RuntimeError, ValueError):
                self.errors.append('child_identity_observation_unconfirmed')
            finally:
                if descriptor is not None:os.close(descriptor)
        return identities

    def begin_read(self, session):
        require(self.active_read is None, 'overlapping old public reads refused')
        # Capture already-running metadata children before opening this interval;
        # they cannot serve as a newly observed Snapshot helper witness.
        self.scan()
        index = len(self.reads)
        self.reads.append({'index':index,'session_id':session})
        self.active_read = (index, session)

    def end_read(self):
        self.active_read = None

    def observe(self):
        while not self.stop.wait(.001):
            try:
                if time.monotonic() >= self.deadline:
                    self.errors.append('observer_witness_deadline');return
                self.scan()
            except (OSError, RuntimeError, ValueError):
                self.errors.append('supervisor_child_observation_unconfirmed');return

    def finish(self):
        if self.closed:return self.outcome
        self.stop.set();self.thread.join(timeout=3)
        require(not self.thread.is_alive(), 'observer watcher retirement unconfirmed')
        records = []
        try:
            final = self.scan()
            for witness in self.witnesses.values():
                retired = self.q.retired(witness)
                records.append({**{k:v for k,v in witness.items() if k!='descriptor'},
                                'pidfd_exit_observed':self.q.pidfd_exited(witness['descriptor']),
                                'exit_and_reaping_observed':retired})
            read_witnesses = [{'index':read['index'],'session_id':read['session_id'],
                               'observed_helpers':[(row['pid'],row['start_ticks']) for row in records
                                    if row.get('snapshot_read_interval') == read['index']
                                    and row.get('session_id') == read['session_id']]} for read in self.reads]
            known = (bool(read_witnesses) and all(read['observed_helpers'] for read in read_witnesses)
                     and all(row['exit_and_reaping_observed'] for row in records))
            # No sampled helper is not a fabricated witness of a transient helper.
            self.outcome = {'supervisor_pid':self.supervisor,'supervisor_start_ticks':self.started,
                    'direct_children_baseline':sorted(self.baseline),'direct_children_end':sorted(final),
                    'direct_children_unchanged':self.baseline==final,'observed_helpers':records,
                    'snapshot_read_intervals':read_witnesses,
                    'cleanup_observation':'observed' if known and self.baseline==final and not self.errors else 'unknown',
                    'pending_observation_categories':sorted(set(self.errors)), 'no_signals_or_effects':True}
            return self.outcome
        finally:
            close_error = None
            for witness in self.witnesses.values():
                try:os.close(witness['descriptor'])
                except OSError as error:close_error = close_error or error
            self.witnesses.clear();self.closed = True
            if self.outcome is None:self.outcome = {'cleanup_observation':'unknown','no_signals_or_effects':True}
            if close_error is not None:raise close_error


def qualify_restored(q, args):
    """Separate explicit read-only checkpoint; never replace a failed monitor."""
    prefix = 'readonly-restored-' + args.case
    q.write(prefix + '-attempt.json', {'case': args.case, 'action': 'read-only-reconcile',
                                     'upgrade_or_signal_replayed': False})
    passed = False
    history_verified = False
    records = []
    public_reads = []
    observer = None
    observer_cleanup = None
    try:
        before = read_json(q.WORK / 'before.json')
        observed = q.observation()
        require(observed['catalogue_schema'] == 1 and not observed['quarantine']
                and all(observed['service'][key] == before['service'][key]
                        for key in ('executable_sha256', 'unit_sha256', 'unit_file_state')),
                'original reader/pointer/schema/quarantine not positively restored')
        require(canonical_bytes(observed['sessions']) == canonical_bytes(before['sessions']),
                'complete canonical DB history differs from original seed')
        restored_start = q.process_start(observed['service']['pid'])
        if args.case == 'startup':
            result = read_json(q.WORK / 'fail-startup-monitor-result.json')
            witnesses = result.get('signals')
            require(isinstance(witnesses, list) and 1 <= len(witnesses) <= 5
                    and all(w.get('signal_delivered') is True for w in witnesses),
                    'original bounded candidate signal evidence unavailable')
            original, installer, manifest = original_target(q)
            require(type(original.get('created_at')) is int
                    and original['created_at'] >= int((q.WORK / 'fail-startup-attempt.json').stat().st_mtime)
                    and all(w.get('executable_sha256') == manifest['binaries']['vessel']['sha256']
                            for w in witnesses), 'candidate signal does not bind original fresh target')
            role = 'signalled-candidate'
        else:
            result = read_json(q.WORK / 'fault-helper-snapshot-result.json')
            require(result.get('signal_delivered') is True and result.get('boundary') == 'helper-snapshot'
                    and result.get('role') == 'local-owner-updater'
                    and result.get('independent_remote_worker') is False
                    and isinstance(result.get('operation_id'), str),
                    'original local-owner/helper signal evidence unavailable')
            identities = result.get('identities') or {}
            require(set(identities) == {'updater', 'helper'}, 'exact owned helper/updater witnesses unavailable')
            original, installer, manifest = original_target(q, result.get('operation_id'))
            owner = read_json(q.WORK / 'fault-helper-snapshot-owner.json')
            pre = read_json(q.WORK / 'fault-helper-snapshot-pre-signal.json')
            updater, helper = identities['updater'], identities['helper']
            from native_legacy_faults import LEGACY_PROGRAM_SHA256
            updater_args = [str(installer), '--bin-dir', str(installer.parent), 'upgrade', '--no-start']
            expected_argv = hashlib.sha256(b'\0'.join(value.encode() for value in updater_args)).hexdigest()
            pin = owner.get('record_pin') or {}
            require(owner.get('role') == 'local-owner-updater'
                    and owner.get('independent_remote_worker') is False
                    and owner.get('operation_id') == pre.get('operation_id') == original['operation_id']
                    and pin.get('operation_id') == original['operation_id']
                    and pin.get('release_id') == original['release_id']
                    and pin.get('current_release') == original['current_release']
                    and pre.get('boundary') == 'helper-snapshot'
                    and pre.get('helper_program_sha256') == LEGACY_PROGRAM_SHA256
                    and updater.get('argv_sha256') == expected_argv
                    and pre.get('owner') == {k:v for k,v in updater.items()
                                             if k not in ('pidfd_exit_observed','exit_and_reaping_observed')}
                    and pre.get('target') == {k:v for k,v in helper.items()
                                              if k not in ('pidfd_exit_observed','exit_and_reaping_observed')}
                    and updater.get('executable_sha256') == manifest['binaries']['voyage-installer']['sha256']
                    and helper.get('executable_sha256') == q.digest(Path('/usr/bin/python3').resolve(strict=True))
                    and helper.get('parent_pid') == updater.get('pid')
                    and helper.get('parent_start_ticks') == updater.get('start_ticks'),
                    'original helper/parent/operation/target witness conflict')
            witnesses = [updater, helper]
            role = 'owned-updater-and-helper'
        for witness in witnesses:
            require(original_identity_retired(q, witness), 'original signalled/owned identity still not reaped')
            records.append({'pid': witness['pid'], 'start_ticks': witness['start_ticks'],
                            'uid': witness['uid'], 'original_instance_retired': True})
        # This is the maintained actual old saved reader. Only Snapshot commands
        # are issued once per original session; no prepare/apply/stop/signal retry.
        import sys
        sys.path.insert(0, str(Path(q.__file__).resolve().parents[2] / 'voyage/tests'))
        from delivery_recovery import Fixture
        reader = object.__new__(Fixture)
        reader.directory = q.STATE
        observer = ObserverWatch(q, observed['service']['pid'], restored_start, before['sessions'])
        for session, saved in before['sessions'].items():
            revision = q.sqlite_observe(q.STATE / 'sessions' / session / 'journal/journal.sqlite3',
                                        'SELECT revision FROM sessions WHERE id=?', (session,))
            require(len(revision) == 1 and type(revision[0][0]) is int, 'old journal revision unavailable')
            observer.begin_read(session)
            try:
                public_reads.append(verify_snapshot(reader.command(session, {'op': 'snapshot'}),
                                                    session, saved['messages'], revision[0][0]))
            finally:
                observer.end_read()
        observer_cleanup = observer.finish()
        observer = None
        after = q.observation()
        require(after['catalogue_schema'] == 1 and not after['quarantine']
                and after['service'] == observed['service']
                and q.process_start(after['service']['pid']) == restored_start
                and canonical_bytes(after['sessions']) == canonical_bytes(before['sessions']),
                'canonical history or original service identity changed during old read')
        require(all(original_identity_retired(q, witness) for witness in witnesses),
                'original owned identity retirement changed during old read')
        history_verified = True
        passed = observer_cleanup.get('cleanup_observation') == 'observed'
        q.write(prefix + ('-qualified.json' if passed else '-history-verified.json'), {'case': args.case, 'role': role,
                'upgrade_or_signal_replayed': False, 'complete_db_before_after_equal': True,
                'original_operation_id': original['operation_id'],
                'qualified_target_release': original['release_id'],
                'original_record_legacy_proof_present': original.get('legacy_proof') is not None,
                'old_observer_cleanup': observer_cleanup,
                'old_public_history_reads': public_reads, 'original_owned_identities': records,
                'observed': after, 'restored_supervisor_start_ticks': restored_start,
                'original_failed_monitor_preserved': True})
    finally:
        if observer is not None:
            try:observer_cleanup = observer.finish()
            except (OSError, RuntimeError, ValueError):observer_cleanup = {'cleanup_observation':'unknown','no_signals_or_effects':True}
        q.write(prefix + '-result.json', {'case': args.case, 'status': ('qualified' if passed else 'history_verified_cleanup_unknown' if history_verified else 'unqualified'),
                'upgrade_or_signal_replayed': False, 'history_verified': history_verified, 'old_public_history_reads': public_reads,
                'original_owned_identities': records, 'old_observer_cleanup':observer_cleanup,
                'original_failed_monitor_preserved': True})
