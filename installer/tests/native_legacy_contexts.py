"""One source-bound ordinary local-owner context change; never restore or retry."""
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import time

from native_legacy_faults import (DEADLINE_SECONDS, argv, bounded, exact_again,
    process_witness, public_witness, record_pin, stable_record, witness_scan)


def private_bytes(q, path, limit=1024*1024):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        before = os.fstat(descriptor)
        q.require(stat.S_ISREG(before.st_mode) and before.st_uid == 1000
                  and before.st_nlink == 1 and before.st_mode & 0o077 == 0
                  and 0 <= before.st_size <= limit, 'private context evidence shape refused')
        chunks, remaining = [], before.st_size
        while remaining:
            chunk = os.read(descriptor, min(1024*1024, remaining))
            if not chunk:break
            chunks.append(chunk);remaining -= len(chunk)
        data = b''.join(chunks)
        after = os.fstat(descriptor)
        named = path.lstat()
        identity = lambda value: (value.st_dev, value.st_ino, value.st_uid,
            value.st_mode, value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        q.require(len(data) == before.st_size and identity(before) == identity(after) == identity(named),
                  'private context evidence changed during read')
        return data, identity(before)
    finally:
        os.close(descriptor)


def manager_environment(q):
    # Save the real presence/value, including absence; never assume the old default.
    result = {}
    for line in q.manager('show-environment').splitlines():
        key, separator, value = line.partition('=')
        if key in ('HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_STATE_HOME',
                   'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR'):
            q.require(separator and key not in result, 'ambiguous manager namespace')
            result[key] = value
    q.require(result.get('HOME', str(q.HOME)) == str(q.HOME)
              and result.get('XDG_RUNTIME_DIR') == '/run/user/1000', 'manager account namespace changed')
    for key, fallback in (('XDG_DATA_HOME', q.HOME/'.local/share'),
                          ('XDG_CONFIG_HOME', q.HOME/'.config'),
                          ('XDG_STATE_HOME', q.HOME/'.local/state'),
                          ('XDG_CACHE_HOME', q.HOME/'.cache')):
        q.require(result.get(key, str(fallback)) == str(fallback), 'manager namespace differs from fixture review')
    return result


def owner_namespace(q, owner):
    raw = bounded(Path('/proc')/str(owner['pid'])/'environ',131072)
    result = {}
    keys = ('HOME','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME','XDG_RUNTIME_DIR')
    for entry in raw.split(b'\0'):
        key, separator, value = entry.partition(b'=')
        name = os.fsdecode(key)
        if name in keys:
            q.require(separator and name not in result, 'ambiguous updater namespace')
            result[name] = os.fsdecode(value)
    q.require(result.get('HOME') == str(q.HOME) and result.get('XDG_RUNTIME_DIR') == '/run/user/1000',
              'updater account namespace changed')
    for key, path in (('XDG_DATA_HOME',q.HOME/'.local/share'),('XDG_CONFIG_HOME',q.HOME/'.config'),
                      ('XDG_STATE_HOME',q.HOME/'.local/state'),('XDG_CACHE_HOME',q.HOME/'.cache')):
        q.require(result.get(key,str(path)) == str(path), 'updater namespace differs from fixture')
    return result


def stopped(q, owner):
    raw = bounded(Path('/proc')/str(owner['pid'])/'stat', 8192).decode()
    fields = raw.rsplit(')', 1)[1].split()
    q.require(len(fields) >= 20 and int(fields[19]) == owner['start_ticks'], 'paused owner identity changed')
    return fields[0] in ('T', 't')


def no_children(q, owner):
    tasks = list((Path('/proc')/str(owner['pid'])/'task').iterdir())
    q.require(0 < len(tasks) <= 256, 'owner thread inventory unavailable')
    for task in tasks:
        q.require(not bounded(task/'children', 8192).split(), 'owned updater helper still present; context window refused')


def snapshot_pin(q, record):
    proof = record.get('legacy_proof')
    q.require(isinstance(proof, dict) and proof.get('state') == str(q.STATE)
              and proof.get('accounts') == record['legacy_accounts']
              and proof.get('stage') == record['staging_root'], 'durable snapshot namespace unavailable')
    stage = Path(proof['stage'])
    q.require(stage.resolve(strict=True) == stage and stage.is_relative_to(q.HOME), 'snapshot stage changed')
    q.require(record.get('phase') == 'applying' and q.installed_bin().parent.name == record['current_release']
              and q.sqlite_observe(q.STATE/'catalogue.sqlite3', 'SELECT version FROM schema_version WHERE id=1') == [(1,)],
              'prepublication old-schema snapshot window missed')
    result = {}
    values = {}
    for label, path in (('proof', stage/'legacy-proof.json'),
                        ('backup', stage/'legacy-catalogue.sqlite3'),
                        ('quarantine', q.STATE/'update-quarantine.json')):
        data, identity = private_bytes(q, path, 64*1024*1024 if label == 'backup' else 1024*1024)
        result[label] = {'sha256': hashlib.sha256(data).hexdigest(), 'identity': identity}
        if label != 'backup':values[label] = json.loads(data)
    saved = values['proof']
    q.require(isinstance(saved, dict) and isinstance(proof.get('evidence'), dict)
              and {key:value for key,value in saved.items() if key != 'sessions'} == proof['evidence']
              and saved.get('backup_sha256') == result['backup']['sha256'], 'durable snapshot differs from operation proof')
    q.require(values['quarantine'] == {'schema_version':1,'operation_id':record['operation_id'],
              'previous_release':record['current_release'],'candidate_release':record['release_id']},
              'snapshot quarantine is not bound to fresh operation')
    return result


def change(q, context, environment, unit, unit_identity, enabled, name):
    if context == 'account-namespace':
        changed = q.WORK/'independent-data'
        q.require(not changed.exists(), 'independent manager namespace already exists')
        changed.mkdir(mode=0o700)
        q.manager('set-environment', 'XDG_DATA_HOME='+str(changed))
        actual = manager_environment_unchecked(q)
        q.require(actual.get('XDG_DATA_HOME') == str(changed)
                  and {k:v for k,v in actual.items() if k != 'XDG_DATA_HOME'}
                  == {k:v for k,v in environment.items() if k != 'XDG_DATA_HOME'}, 'manager change not positively observed')
        return {'key': 'XDG_DATA_HOME', 'original_present': 'XDG_DATA_HOME' in environment,
                'original_value': environment.get('XDG_DATA_HOME'), 'changed_value': str(changed)}
    if context == 'unit':
        # A full private saved copy permits explicit later operator restoration;
        # this monitor never restores or replaces the reviewed unit.
        saved = q.WORK/(name+'-original.service')
        with saved.open('xb') as stream:
            stream.write(unit);stream.flush();os.fsync(stream.fileno())
        saved.chmod(0o600)
        descriptor = os.open(q.UNIT, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            current = os.fstat(descriptor)
            actual = (current.st_dev,current.st_ino,current.st_uid,current.st_mode,current.st_nlink,
                      current.st_size,current.st_mtime_ns,current.st_ctime_ns)
            q.require(actual == unit_identity, 'reviewed unit changed before context effect')
            comment = b'\n# Independent qualification operator edit\n'
            q.require(os.write(descriptor, comment) == len(comment), 'unit context write incomplete')
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        q.manager('daemon-reload')
        observed, _ = private_bytes(q, q.UNIT)
        q.require(observed == unit+comment, 'unit change not positively observed')
        return {'original_sha256':hashlib.sha256(unit).hexdigest(),
                'changed_sha256':hashlib.sha256(observed).hexdigest(), 'saved_original':str(saved)}
    q.require(context == 'enablement' and enabled == 'enabled', 'enablement context is not a real enabled-to-disabled change')
    q.manager('disable', q.NAME)
    q.require(q.manager('show',q.NAME,'--property=UnitFileState','--value') == 'disabled',
              'persistent disablement not positively observed')
    return {'original_unit_file_state':enabled, 'changed_unit_file_state':'disabled'}


def manager_environment_unchecked(q):
    result = {}
    for line in q.manager('show-environment').splitlines():
        key, separator, value = line.partition('=')
        if key in ('HOME','XDG_DATA_HOME','XDG_CONFIG_HOME','XDG_STATE_HOME','XDG_CACHE_HOME','XDG_RUNTIME_DIR'):
            q.require(separator and key not in result, 'ambiguous manager namespace')
            result[key] = value
    return result


def run(q, args):
    """Arm before one upgrade, bind one fresh updater, pause once, change once."""
    name = 'context-' + args.context
    baseline = {path.name for path in (q.INSTALL/'updates').glob('*.json')}
    q.write(name+'-attempt.json', {'status':'armed','context':args.context,
        'role':'local-owner-updater','independent_remote_worker':False,
        'monitor_pid':os.getpid(),'monitor_start_ticks':q.process_start(os.getpid()),
        'baseline_records':sorted(baseline),'deadline_seconds':DEADLINE_SECONDS,
        'maximum_stop_requests':1,'maximum_continue_requests':1,'maximum_context_changes':1})
    armed_start, armed_wall = q.process_start(os.getpid()), int(time.time())
    deadline = time.monotonic()+DEADLINE_SECONDS
    owner = image = owner_args = pin = record_path = None
    pause_requested = paused = continue_requested = continued = mutation_requested = changed = False
    snapshot = None
    mutation = None
    pending = {'permission_pending':0,'missing_pending':0,'not_candidate':0}
    try:
        environment = manager_environment(q)
        unit, unit_identity = private_bytes(q, q.UNIT)
        enabled = q.manager('show',q.NAME,'--property=UnitFileState','--value')
        q.require(enabled == 'enabled', 'context qualification requires originally persistently enabled service')
        while time.monotonic() < deadline:
            try:
                if image is None and (q.WORK/'candidate-bin.json').is_file():
                    image = Path(json.loads(bounded(q.WORK/'candidate-bin.json')))
                    q.require(image.is_relative_to(q.WORK/'candidate') and image.resolve(strict=True) == image,
                              'candidate image outside owned staging')
                    release = json.loads(bounded(image.parent.parent/'release.json'))
                    sha = release['binaries']['voyage-installer']['sha256']
                    q.require(q.digest(image) == sha, 'qualified installer image changed')
                    owner_args = [os.fsencode(image),b'--bin-dir',os.fsencode(image.parent),b'upgrade',b'--no-start']
                fresh = sorted(path for path in (q.INSTALL/'updates').glob('*.json') if path.name not in baseline)
                q.require(len(fresh) <= 1, 'ambiguous fresh local-owner operation')
                if image is not None and fresh and record_path is None:
                    record_path = fresh[0]
                    pin = record_pin(q, record_path, json.loads(bounded(record_path)), image)
                    q.require(pin['created_at'] >= armed_wall, 'operation predates armed context monitor')
                    activation = pin['supervisor_activation']
                    q.require(activation.get('enabled') is True and activation.get('unit_file_state') == enabled
                              and activation.get('definition') == unit.decode(), 'fresh review differs from armed unit/enablement')
                if record_path is None:
                    time.sleep(.002);continue
                record = stable_record(q, record_path, pin)
                if owner is None:
                    pids = [args.installer_pid] if args.installer_pid else [int(path.name) for path in Path('/proc').iterdir() if path.name.isdecimal()]
                    q.require(len(pids) <= 65536, 'process inventory exceeds bound')
                    with witness_scan() as matches:
                        for pid in pids:
                            try:
                                if argv(pid) != owner_args:continue
                                category, witness = process_witness(q,pid,image,sha,owner_args)
                                if witness is None:pending[category] += 1
                                else:matches.append(witness)
                            except PermissionError:pending['permission_pending'] += 1
                            except (FileNotFoundError,ProcessLookupError):pending['missing_pending'] += 1
                        if len(matches) != 1:
                            q.require(not matches, 'ambiguous exact local-owner updater')
                            time.sleep(.002);continue
                        owner = matches.pop()
                    q.require(owner['start_ticks'] >= armed_start, 'updater predates armed context monitor')
                    owner_environment = owner_namespace(q,owner)
                    q.write(name+'-owner.json', {**public_witness(owner),'record_pin':pin,
                            'updater_namespace':owner_environment})
                q.require(exact_again(q,owner,image,owner_args) and owner_namespace(q,owner) == owner_environment,
                          'retained updater identity/namespace changed before context boundary')
                if record.get('legacy_proof') is None:
                    time.sleep(.002);continue
                snapshot = snapshot_pin(q,record)
                q.require(manager_environment(q) == environment and private_bytes(q,q.UNIT) == (unit,unit_identity)
                          and q.manager('show',q.NAME,'--property=UnitFileState','--value') == enabled,
                          'operator context changed before qualification effect')
                no_children(q,owner)
                frozen = json.dumps(record,sort_keys=True,separators=(',',':'))
                q.write(name+'-pre-stop.json', {'owner':public_witness(owner),'record_pin':pin,
                        'snapshot':snapshot,'original_manager_namespace':environment,
                        'unit_sha256':hashlib.sha256(unit).hexdigest(),'unit_file_state':enabled})
                q.require(json.dumps(stable_record(q,record_path,pin),sort_keys=True,separators=(',',':')) == frozen
                          and exact_again(q,owner,image,owner_args) and owner_namespace(q,owner) == owner_environment,
                          'durable context window changed before pause')
                pause_requested = True
                signal.pidfd_send_signal(owner['descriptor'],signal.SIGSTOP)
                until = time.monotonic()+5
                while time.monotonic() < until and not stopped(q,owner):time.sleep(.002)
                paused = stopped(q,owner)
                q.require(paused and exact_again(q,owner,image,owner_args)
                          and owner_namespace(q,owner) == owner_environment, 'same-owner stopped state unconfirmed')
                no_children(q,owner)
                q.require(json.dumps(stable_record(q,record_path,pin),sort_keys=True,separators=(',',':')) == frozen
                          and snapshot_pin(q,record) == snapshot and manager_environment(q) == environment
                          and private_bytes(q,q.UNIT) == (unit,unit_identity)
                          and q.manager('show',q.NAME,'--property=UnitFileState','--value') == enabled,
                          'snapshot or reviewed context changed while pausing; no mutation')
                q.write(name+'-mutation-request.json', {'context':args.context,'operation_id':pin['operation_id'],
                        'same_owner_stopped_observed':True,'snapshot':snapshot})
                q.require(stopped(q,owner) and exact_again(q,owner,image,owner_args)
                          and owner_namespace(q,owner) == owner_environment, 'paused owner changed before mutation')
                mutation_requested = True
                mutation = change(q,args.context,environment,unit,unit_identity,enabled,name)
                changed = True
                q.write(name+'-changed.json', {'operation_id':pin['operation_id'],'context':args.context,
                        'original_manager_namespace':environment,'change':mutation,
                        'independent_change_applied':True,'implicit_restoration':False})
                return
            except PermissionError:
                pending['permission_pending'] += 1
                if owner is not None:raise RuntimeError('retained owner permission unavailable; no replacement pause') from None
            except (FileNotFoundError,ProcessLookupError,q.ProcessNotLive):
                pending['missing_pending'] += 1
                if owner is not None:raise RuntimeError('retained owner changed; no replacement pause') from None
            time.sleep(.002)
        raise RuntimeError('durable snapshot context window not observed within 1230 seconds')
    finally:
        resume_error = None
        if pause_requested and owner is not None:
            try:
                q.require(exact_again(q,owner,image,owner_args) and owner_namespace(q,owner) == owner_environment,
                          'retained owner unavailable; continue remains unconfirmed')
                continue_requested = True
                signal.pidfd_send_signal(owner['descriptor'],signal.SIGCONT)
                until = time.monotonic()+5
                while time.monotonic() < until:
                    q.require(exact_again(q,owner,image,owner_args), 'continued owner identity unavailable')
                    if not stopped(q,owner):continued = True;break
                    time.sleep(.002)
                q.require(continued, 'same-owner continuation not positively observed')
            except (OSError,RuntimeError,ValueError):resume_error = 'continue_unconfirmed'
        witness = public_witness(owner) if owner is not None else None
        if owner is not None:os.close(owner['descriptor'])
        q.write(name+'-result.json', {'context':args.context,'operation_id':pin['operation_id'] if pin else None,
            'status':'context_changed_and_owner_continued' if changed and continued else 'unqualified',
            'role':'local-owner-updater','independent_remote_worker':False,'owner':witness,
            'pause_requested':pause_requested,'same_owner_stopped_observed':paused,
            'mutation_requested':mutation_requested,'mutation_observed':changed,'change':mutation,
            'continue_requested':continue_requested,'same_owner_continued_observed':continued,
            'resume_error':resume_error,'pending_observations':pending,
            'implicit_restoration':False,'rollback_or_cleanup_acceptance':False})
        if resume_error is not None:raise RuntimeError('original owner continuation unconfirmed; retain context and operation') from None
