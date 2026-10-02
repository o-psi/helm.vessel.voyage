"""One bounded actual local-owner fault; observation is separate from acceptance."""
import hashlib
import json
import os
from pathlib import Path
import signal
import time
import uuid
from contextlib import contextmanager

DEADLINE_SECONDS = 1230
# Full reviewed embedded program identity, checked against source by local contracts.
LEGACY_PROGRAM_SHA256 = '367099dcf722459f1097ced8c49708a7901b129e36399bc358cfef505a09a4be'


def bounded(path, limit=65536):
    with path.open('rb') as stream:
        value = stream.read(limit + 1)
    if len(value) > limit:
        raise RuntimeError('fault identity exceeds bound')
    return value


def argv(pid):
    raw = bounded(Path('/proc') / str(pid) / 'cmdline', 131072)
    if not raw:
        return []
    if not raw.endswith(b'\0'):
        raise RuntimeError('process argv is incomplete')
    return raw[:-1].split(b'\0')


def parent(pid):
    raw = bounded(Path('/proc') / str(pid) / 'stat', 8192).decode()
    fields = raw.rsplit(')', 1)[1].split()
    return int(fields[1])


def canonical_uuid(value):
    try:
        return isinstance(value, str) and str(uuid.UUID(value)) == value and uuid.UUID(value).int != 0
    except (ValueError, AttributeError):
        return False


def record_pin(q, path, record, candidate):
    op = record.get('operation_id')
    q.require(canonical_uuid(op) and path.name == op + '.json', 'noncanonical fault operation')
    q.require(record.get('legacy_mode') is True and record.get('channel') == 'local-owner',
              'fault route is not the maintained local-owner updater')
    q.require(type(record.get('created_at')) is int and record['created_at'] > 0
              and all(isinstance(record.get(k), str) and len(record[k]) == 64
                      and all(c in '0123456789abcdef' for c in record[k])
                      for k in ('current_release', 'release_id')), 'release/time identity unavailable')
    stage = q.HOME / '.cache/voyage/upgrades' / ('prepare-local-' + op)
    activation = record.get('supervisor_activation') or {}
    accounts = record.get('legacy_accounts')
    q.require(record.get('staging_root') == str(stage) and
              record.get('bin_dir') == str(stage / 'candidate/bin') and
              activation.get('state') == str(q.STATE) and activation.get('active') is True,
              'local-owner activation/staging identity mismatch')
    q.require(isinstance(accounts, str) and Path(accounts).is_absolute() and
              Path(accounts).is_relative_to(q.HOME) and Path(accounts).resolve(strict=True) == Path(accounts),
              'account namespace outside owned canonical fixture')
    release = json.loads(bounded(candidate.parent.parent / 'release.json'))
    staged = json.loads(bounded(stage / 'candidate/release.json'))
    q.require(release == staged, 'local-owner staged manifest changed')
    return {name: record.get(name) for name in (
        'operation_id', 'channel', 'created_at', 'current_release', 'release_id',
        'bin_dir', 'staging_root', 'legacy_mode', 'legacy_accounts', 'supervisor_activation',
        'contracts_sha256')}


def stable_record(q, path, pin):
    current = json.loads(bounded(path))
    q.require(all(current.get(k) == v for k, v in pin.items()), 'fault operation identity changed')
    return current


def public_witness(witness):
    return {k: v for k, v in witness.items() if k not in ('descriptor', 'image_pin')}


def process_witness(q, pid, image, sha, expected_argv, owner=None):
    category, witness = q.exact_candidate(pid, image, sha)
    if witness is None:
        return category, None
    try:
        if argv(pid) != expected_argv or owner is not None and parent(pid) != owner['pid']:
            return 'not_candidate', None
        if not q.still_candidate(witness, image):
            return 'missing_pending', None
        witness['argv_sha256'] = hashlib.sha256(b'\0'.join(expected_argv)).hexdigest()
        if owner is not None:
            witness['parent_pid'] = owner['pid']
            witness['parent_start_ticks'] = owner['start_ticks']
        result = witness
        witness = None
        return 'verified', result
    except PermissionError:
        return 'permission_pending', None
    except (FileNotFoundError, ProcessLookupError, q.ProcessNotLive):
        return 'missing_pending', None
    finally:
        if witness is not None:
            os.close(witness['descriptor'])


def exact_again(q, witness, image, expected_argv, owner=None):
    return (q.still_candidate(witness, image) and argv(witness['pid']) == expected_argv
            and (owner is None or parent(witness['pid']) == owner['pid']))


def boundary_seen(q, boundary, record):
    proof = record.get('legacy_proof')
    if proof is not None:
        q.require(proof.get('state') == str(q.STATE) and proof.get('accounts') == record['legacy_accounts']
                  and proof.get('stage') == record['staging_root'], 'helper proof namespace changed')
    if boundary == 'snapshot':
        return proof is not None
    if boundary == 'committing':
        return record.get('phase') == 'committing'
    if boundary == 'restored':
        return bool(proof and (Path(proof['stage']) / 'legacy-restored.json').is_file())
    return False


def qualified_helper_argv(actual, action, state, accounts, stage):
    return (len(actual) == 8 and actual[:3] == [b'/usr/bin/python3', b'-I', b'-c']
            and hashlib.sha256(actual[3]).hexdigest() == LEGACY_PROGRAM_SHA256
            and actual[4:] == [action.encode(), os.fsencode(state),
                              os.fsencode(accounts), os.fsencode(stage)])


@contextmanager
def witness_scan():
    """The scan owns all admitted pidfds until exactly one witness transfers."""
    matches = []
    try:
        yield matches
    finally:
        failure = None
        for item in matches:
            witness = item[0] if isinstance(item, tuple) else item
            try:
                os.close(witness['descriptor'])
            except OSError as error:
                failure = failure or error
        if failure is not None:
            raise failure


def run(q, args):
    """Arm before ONE upgrade; no pause, effect replay or generic process signal."""
    deadline = time.monotonic() + DEADLINE_SECONDS
    baseline = {p.name for p in (q.INSTALL / 'updates').glob('*.json')}
    name = 'fault-' + args.boundary
    q.write(name + '-attempt.json', {'status': 'armed', 'role': 'local-owner-updater',
            'monitor_pid': os.getpid(), 'monitor_start_ticks': q.process_start(os.getpid()),
            'deadline_seconds': DEADLINE_SECONDS, 'baseline_records': sorted(baseline),
            'maximum_signals': 1})
    owner = helper = None
    target = pin = record_path = image = None
    owner_args = helper_args = None
    delivered = False
    completed = False
    pending = {'permission_pending': 0, 'missing_pending': 0, 'not_candidate': 0}
    signal_error = None
    armed_start = q.process_start(os.getpid())
    armed_wall = int(time.time())
    try:
        python = Path('/usr/bin/python3').resolve(strict=True)
        python_sha = q.digest(python)
        while time.monotonic() < deadline:
            try:
                if delivered:
                    # After delivery, only retained-target retirement is observed. Terminal
                    # receipt cleanup may legitimately change/remove admission fields.
                    if q.retired(target):
                        completed = True
                        return
                    time.sleep(.01)
                    continue
                if image is None and (q.WORK / 'candidate-bin.json').is_file():
                    image = Path(json.loads(bounded(q.WORK / 'candidate-bin.json')))
                    q.require(image.is_relative_to(q.WORK / 'candidate') and image.resolve(strict=True) == image,
                              'candidate executable outside owned staging')
                    release = json.loads(bounded(image.parent.parent / 'release.json'))
                    sha = release['binaries']['voyage-installer']['sha256']
                    q.require(q.digest(image) == sha, 'qualified installer image changed')
                    owner_args = [os.fsencode(image), b'--bin-dir', os.fsencode(image.parent),
                                  b'upgrade', b'--no-start']
                fresh = sorted(p for p in (q.INSTALL / 'updates').glob('*.json') if p.name not in baseline)
                q.require(len(fresh) <= 1, 'ambiguous new update operations; no signal')
                if image is not None and fresh and record_path is None:
                    record_path = fresh[0]
                    record = json.loads(bounded(record_path))
                    pin = record_pin(q, record_path, record, image)
                    q.require(pin['created_at'] >= armed_wall, 'operation predates fresh fault monitor')
                if record_path is None:
                    time.sleep(.002)
                    continue
                record = stable_record(q, record_path, pin)
                if owner is None:
                    pids = [args.installer_pid] if args.installer_pid else [int(p.name) for p in Path('/proc').iterdir() if p.name.isdecimal()]
                    q.require(len(pids) <= 65536, 'process inventory exceeds bound')
                    with witness_scan() as matches:
                        for pid in pids:
                            try:
                                # Cheap exact argv filter before the qualified-image hash read.
                                if argv(pid) != owner_args:
                                    continue
                                category, witness = process_witness(q, pid, image, sha, owner_args)
                                if witness is None:
                                    pending[category] += 1
                                else:
                                    matches.append(witness)
                            except PermissionError:
                                pending['permission_pending'] += 1
                            except (FileNotFoundError, ProcessLookupError):
                                pending['missing_pending'] += 1
                        if len(matches) != 1:
                            q.require(not matches, 'ambiguous exact local-owner updater')
                            time.sleep(.002)
                            continue
                        owner = matches.pop()
                    q.require(owner['start_ticks'] >= armed_start, 'updater predates fresh fault monitor')
                    q.write(name + '-owner.json', {**public_witness(owner), 'operation_id': pin['operation_id'],
                            'role': 'local-owner-updater', 'independent_remote_worker': False,
                            'record_pin': pin})
                q.require(not q.pidfd_exited(owner['descriptor']), 'updater exited before requested boundary')
                if args.boundary.startswith('helper-'):
                    action = args.boundary.removeprefix('helper-')
                    children = bounded(Path('/proc') / str(owner['pid']) / 'task' / str(owner['pid']) / 'children', 8192).split()
                    with witness_scan() as matches:
                        for child in children:
                            actual = argv(int(child))
                            if not qualified_helper_argv(actual, action, q.STATE,
                                                         pin['legacy_accounts'], pin['staging_root']):
                                continue
                            category, witness = process_witness(q, int(child), python, python_sha, actual, owner)
                            if witness is not None:
                                matches.append((witness, actual))
                            else:
                                pending[category] += 1
                        if len(matches) != 1:
                            q.require(not matches, 'ambiguous owned helper')
                            time.sleep(.002)
                            continue
                        helper, helper_args = matches.pop()
                    target = helper
                    target_image, expected = python, helper_args
                elif boundary_seen(q, args.boundary, record):
                    target = owner
                    target_image, expected = image, owner_args
                else:
                    time.sleep(.002)
                    continue
                stable_record(q, record_path, pin)
                q.require(exact_again(q, owner, image, owner_args), 'updater identity changed before signal')
                q.require(exact_again(q, target, target_image, expected, owner if helper else None),
                          'fault target identity changed before signal')
                q.write(name + '-pre-signal.json', {'operation_id': pin['operation_id'],
                        'boundary': args.boundary, 'owner': public_witness(owner),
                        'target': public_witness(target), 'maximum_signals': 1,
                        'helper_program_sha256': LEGACY_PROGRAM_SHA256 if helper else None})
                # The evidence fsync can outlive a short window. Recheck after it too.
                stable_record(q, record_path, pin)
                q.require(exact_again(q, owner, image, owner_args), 'updater changed after pre-effect witness')
                q.require(exact_again(q, target, target_image, expected, owner if helper else None),
                          'target changed after pre-effect witness')
                # No retry after a requested/uncertain signal. The open pidfd remains retained.
                try:
                    signal.pidfd_send_signal(target['descriptor'], signal.SIGKILL)
                    delivered = True
                except OSError:
                    signal_error = 'signal_delivery_unconfirmed'
                    raise RuntimeError('fault signal unconfirmed; retain original operation') from None
            except PermissionError:
                pending['permission_pending'] += 1
                if target is not None and not delivered:
                    raise RuntimeError('retained target permission unavailable; no replacement signal') from None
            except (FileNotFoundError, ProcessLookupError, q.ProcessNotLive):
                pending['missing_pending'] += 1
                if target is not None and not delivered:
                    raise RuntimeError('retained target changed; no replacement signal') from None
            time.sleep(.002)
        raise RuntimeError('fault boundary/retirement not positively observed within 1230 seconds')
    finally:
        identities = {}
        for role, witness in [('updater', owner), ('helper', helper)]:
            if witness is None:
                continue
            try:
                exited, reaped = q.pidfd_exited(witness['descriptor']), q.retired(witness)
            except (OSError, RuntimeError, ValueError):
                exited = reaped = None
            identities[role] = {**public_witness(witness), 'pidfd_exit_observed': exited,
                                'exit_and_reaping_observed': reaped}
            os.close(witness['descriptor'])
        q.write(name + '-result.json', {'status': 'fault_target_retired' if completed else 'unqualified',
                'operation_id': pin['operation_id'] if pin else None, 'boundary': args.boundary,
                'role': 'local-owner-updater', 'independent_remote_worker': False,
                'signal_delivered': delivered, 'signal_error': signal_error,
                'identities': identities, 'pending_observations': pending,
                'deadline_seconds': DEADLINE_SECONDS, 'rollback_or_cleanup_acceptance': False})
