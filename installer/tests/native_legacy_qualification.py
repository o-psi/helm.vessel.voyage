#!/usr/bin/env python3
"""Offline, real user-manager #401 qualification. No implicit reset or effect retry."""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import select
import stat
import sqlite3
import subprocess
import sys
import tarfile
import threading
import time

OLD_SHA = 'f7b078e654e514d23d4ab9021337837b90579babf9f79e21f2a01fdce4fcb4c8'
HOME = Path('/home/qualification')
STATE = HOME / '.local/state/voyage/vessel'
INSTALL = HOME / '.local/share/voyage/install'
UNIT = HOME / '.config/systemd/user/voyage-vessel.service'
WORK = HOME / 'q401'
NAME = 'voyage-vessel.service'


def require(condition, reason):
    if not condition:
        raise RuntimeError(reason)


def command(args, timeout=30, env=None, check=True):
    result = subprocess.run(args, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=timeout)
    if check:
        require(result.returncode == 0, 'bounded command refused; retain private diagnostics')
    return result


def manager(*args):
    return command(['systemctl', '--user', *args]).stdout.decode().strip()


def preflight():
    require(os.getuid() == os.geteuid() == 1000, 'requires disposable qualification UID 1000')
    require(Path.home() == HOME and HOME.resolve() == HOME, 'requires dedicated /home/qualification')
    require(os.environ.get('XDG_RUNTIME_DIR') == '/run/user/1000', 'unexpected runtime namespace')
    for key, fallback in [('XDG_CONFIG_HOME', HOME/'.config'), ('XDG_STATE_HOME', HOME/'.local/state'),
                          ('XDG_DATA_HOME', HOME/'.local/share'), ('XDG_CACHE_HOME', HOME/'.cache')]:
        require(Path(os.environ.get(key, str(fallback))) == fallback, 'nonstandard user namespace refused')
    require(Path('/etc/debian_version').read_text().startswith('13'), 'requires reviewed Debian 13 CT')
    require(not Path('/etc/voyage/system-install.json').exists(), 'system installation is outside scope')
    manager('show-environment')
    paths = manager('show', '--property=UnitPath', '--value').split()
    require(str(UNIT.parent) in paths, 'real manager does not search qualification unit directory')
    require(not manager('show', NAME, '--property=DropInPaths', '--value'), 'service overrides refused')
    os.umask(0o077)
    WORK.mkdir(mode=0o700, exist_ok=True)


def write(name, value):
    path = WORK / name
    require(not path.exists(), 'evidence already exists; no implicit rerun')
    # Exclusive creation is the one-attempt fence even if two operators race.
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    path.chmod(0o600)
    directory = os.open(WORK, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def archive(path, expected, label):
    path = path.resolve(strict=True)
    require(digest(path) == expected, 'archive checksum mismatch')
    dest = WORK / label
    require(not dest.exists(), 'archive staging already exists')
    dest.mkdir(mode=0o700)
    with tarfile.open(path) as source:
        entries = source.getmembers()
        require(len(entries) <= 16384 and sum(e.size for e in entries) <= 512*1024*1024,
                'archive bounds refused')
        for entry in entries:
            relative = Path(entry.name)
            require(not relative.is_absolute() and '..' not in relative.parts,
                    'nonlocal archive member')
            require(entry.isfile() or entry.isdir(), 'archive links/devices refused')
        source.extractall(dest, filter='data')
    manifests = list(dest.rglob('release.json'))
    require(len(manifests) == 1, 'ambiguous archive manifest')
    release = manifests[0].parent
    for binary in ('vessel', 'voyage', 'helm', 'voyage-installer'):
        require((release/'bin'/binary).is_file(), 'archive binary missing')
    return release


def installed_bin():
    return (INSTALL/'current').resolve(strict=True)/'bin'


def service_identity():
    pid = int(manager('show', NAME, '--property=MainPID', '--value'))
    require(manager('show', NAME, '--property=ActiveState', '--value') == 'active' and pid > 0,
            'real service is not ready')
    proc = Path('/proc')/str(pid)
    require(proc.stat().st_uid == 1000, 'unexpected service UID')
    executable = (proc/'exe').resolve(strict=True)
    require(executable == installed_bin()/'vessel', 'live executable does not match managed pointer')
    argv = (proc/'cmdline').read_bytes().split(b'\0')
    require(b'--directory' in argv and argv[argv.index(b'--directory')+1] == os.fsencode(STATE),
            'live service directory mismatch')
    selected = {}
    for entry in (proc/'environ').read_bytes().split(b'\0'):
        for key in (b'HOME=', b'XDG_DATA_HOME='):
            if entry.startswith(key):
                selected[key[:-1].decode()] = os.fsdecode(entry[len(key):])
    require(selected.get('HOME') == str(HOME), 'live account HOME mismatch')
    require(selected.get('XDG_DATA_HOME', str(HOME/'.local/share')) == str(HOME/'.local/share'),
            'live account namespace mismatch')
    return {'pid': pid, 'executable_sha256': digest(executable), 'unit_sha256': digest(UNIT),
            'unit_file_state': manager('show', NAME, '--property=UnitFileState', '--value')}


def sqlite_observe(path, query, parameters=()):
    connection = sqlite3.connect(path.as_uri()+'?mode=ro', uri=True, timeout=2)
    try:
        connection.execute('PRAGMA query_only=ON')
        return connection.execute(query, parameters).fetchall()
    finally:
        connection.close()


def observation():
    seed = json.loads((WORK/'seed.json').read_text())
    service = service_identity()
    catalogue = sqlite_observe(STATE/'catalogue.sqlite3', 'SELECT version FROM schema_version WHERE id=1')[0][0]
    sessions = {}
    for session in seed['sessions']:
        journal = STATE/'sessions'/session/'journal/journal.sqlite3'
        schema = sqlite_observe(journal, 'SELECT version FROM attachment_schema WHERE id=1')[0][0]
        saved = sqlite_observe(journal, 'SELECT state FROM sessions WHERE id=?', (session,))
        # Canonical contents, not SQLite file headers or materialized projections.
        require(len(saved) == 1, 'canonical legacy session missing')
        canonical = json.loads(saved[0][0])
        sessions[session] = {'schema': schema, 'messages': canonical['messages']}
    return {'service': service, 'catalogue_schema': catalogue, 'sessions': sessions,
            'quarantine': (STATE/'update-quarantine.json').exists()}


def ready_catalogue(fixture):
    """Only authenticated reads are repeated; no session effect is retried."""
    from delivery_recovery import wait_for
    def ready():
        try:
            before = service_identity()
            catalogue = fixture.request({'op': 'catalogue'})
            after = service_identity()
            return catalogue is not None and before == after
        except (OSError, ValueError, RuntimeError):
            return False
    wait_for(ready, timeout=20)


def empty_old_seed(old_archive):
    """One explicit continuation of an installed old, never-admitted seed."""
    require(digest(old_archive.resolve(strict=True)) == OLD_SHA, 'old archive checksum mismatch')
    require((WORK/'old-install.log').is_file(), 'retained original install evidence required')
    for name in ('seed.json', 'before.json', 'candidate', 'candidate-bin.json',
                 'seed-existing-attempt.json', 'upgrade-result.json'):
        require(not (WORK/name).exists(), 'seed continuation already attempted or admitted')
    require(not (STATE/'update-quarantine.json').exists(), 'quarantined state is outside seed continuation')
    for name in ('updates', 'recoveries'):
        directory = INSTALL/name
        require(not directory.exists() or not any(directory.iterdir()), 'update/recovery evidence retained; seed refused')
    require(sqlite_observe(STATE/'catalogue.sqlite3', 'SELECT version FROM schema_version WHERE id=1') == [(1,)],
            'requires actual old schema1')
    for table in ('voyages', 'incarnations', 'lifecycle_commands', 'creation_receipts',
                  'catalogue', 'catalogue_events'):
        require(sqlite_observe(STATE/'catalogue.sqlite3', 'SELECT COUNT(*) FROM '+table) == [(0,)],
                'accepted or uncertain canonical creation evidence refuses seed continuation')
    imports = sqlite_observe(STATE/'catalogue.sqlite3', 'SELECT source,digest FROM legacy_imports ORDER BY source')
    # Published v1.0.2 initialization always records this all-zero completion sentinel,
    # even in an empty catalogue. It is not an imported registration or command.
    require(imports in ([], [('complete-v1', bytes(32))]),
            'any actual legacy import evidence refuses seed continuation')
    for relative in ('commands', 'start-resolution/intent/commands', 'start-resolution/not-admitted/commands'):
        directory = STATE/relative
        require(not directory.exists() or not any(directory.iterdir()),
                'retained original command evidence refuses seed continuation')
    sessions = STATE/'sessions'
    require(not sessions.exists() or not any(sessions.iterdir()), 'retained session evidence refuses continuation')
    expected = {
        'helm': 'bf33bbea0fddcd23b6aca233b7c881aef6f8795633f1a2d240460fab0b02b592',
        'vessel': '9f19b52a97e38576089f9df3a1f70ec56ddcf66b97fd301fdd83fa2c91a262e4',
        'voyage': '4cc2e01776d269a5430645846d67fc37483975671c35de2be7f7b53c96aefd61',
        'voyage-installer': 'cd6f56cf195bf45196b9539103ff7fb56495dbe0ebb34b715a1d9ea1d50dfed3'}
    for name, value in expected.items():
        require(digest(installed_bin()/name) == value, 'installed binary differs from published old artifact')
    service = service_identity()
    workspace = WORK/'workspace'
    require(workspace.is_dir() and not any(workspace.iterdir()), 'nonempty original workspace refused')
    drafts = {}
    import uuid
    for path in WORK.glob('*.json'):
        if path.name == 'seed-attempt.json':
            continue
        try:
            uuid.UUID(path.stem)
        except ValueError:
            raise RuntimeError('unrecognized retained seed evidence refuses continuation') from None
        value = json.loads(path.read_text())
        require(value.get('version') == 1 and value.get('workspace') == str(workspace)
                and value.get('selection') is None and value.get('confirmation') is None,
                'partial configuration is not the retained synthetic draft')
        config = value.get('config', {})
        require(config.get('provider') == 'openai-responses' and config.get('model') == 'fixture-model'
                and config.get('api_key_required') is False and config.get('access') == 'read-only'
                and config.get('base_url', '').startswith('http://127.0.0.1:'),
                'partial configuration is outside synthetic seed scope')
        drafts[path.name] = digest(path)
    require(drafts, 'requires retained pre-admission synthetic configuration evidence')
    return {'old_sha256': OLD_SHA, 'service': service, 'partial_configuration_sha256': drafts,
            'catalogue_schema': 1, 'voyages': 0,
            'action': 'continue once with distinct new identities; preserve completed private account effects'}


def seed(old_archive, existing=False):
    if existing:
        retained = empty_old_seed(old_archive)
        write('seed-existing-attempt.json', retained)
    else:
        require(not UNIT.exists() and not INSTALL.exists() and not STATE.exists(),
                'qualification namespace must be fresh; no cleanup of existing state')
        require(digest(old_archive.resolve(strict=True)) == OLD_SHA, 'old archive checksum mismatch')
        write('seed-attempt.json', {'old_sha256': OLD_SHA, 'action': 'one fresh old install and seed'})
        old = archive(old_archive, OLD_SHA, 'old')
        result = command([str(old/'bin/voyage-installer'), 'install', '--bin-dir', str(old/'bin'), '--start'], 120)
        (WORK/'old-install.log').write_bytes(result.stdout+result.stderr)
        service_identity()
    # Reuse maintained public command/provider helpers, without their unmanaged launcher or cleanup.
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'voyage/tests'))
    from delivery_recovery import Fixture, Provider
    fixture = object.__new__(Fixture)
    fixture.binaries = installed_bin()
    fixture.root = WORK
    fixture.directory = STATE
    fixture.workspace = WORK/'workspace'
    fixture.workspace.mkdir(mode=0o700, exist_ok=existing)
    fixture.env = dict(os.environ, RECOVERY_FIXTURE_KEY='synthetic-fixture-key')
    fixture.sessions = []
    fixture.provider = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    fixture.provider.bodies = []
    thread = threading.Thread(target=fixture.provider.serve_forever, daemon=True)
    thread.start()
    manager('set-environment', 'RECOVERY_FIXTURE_KEY=synthetic-fixture-key')
    # The already-running supervisor must inherit the declared synthetic binding.
    try:
        manager('restart', NAME)
        ready_catalogue(fixture)
        for index in range(2):
            session = fixture.session()
            exact = fixture.submit(session, 'ordinary legacy qualification '+str(index))
            fixture.command(session, exact)  # One submission only; uncertainty is retained.
            finished = fixture.finished(session)
            require(finished['run']['state'] == 'completed', 'synthetic run did not complete')
            fixture.suspended(session)
        require(len(fixture.provider.bodies) == 2, 'unexpected provider effects')
        write('seed.json', {'sessions': fixture.sessions, 'provider_requests': 2, 'old_sha256': OLD_SHA})
        before = observation()
        require(before['catalogue_schema'] == 1 and all(s['schema'] <= 12 for s in before['sessions'].values()),
                'seed must come from actual published legacy writers')
        write('before.json', before)
    finally:
        fixture.provider.shutdown()
        fixture.provider.server_close()
        manager('unset-environment', 'RECOVERY_FIXTURE_KEY')


def upgrade(args):
    candidate = archive(args.candidate_archive, args.candidate_sha256, 'candidate')
    write('candidate-bin.json', str(candidate/'bin/voyage-installer'))
    script = args.install_sh.resolve(strict=True)
    require(digest(script) == args.install_sh_sha256, 'bootstrap source identity mismatch')
    # Normal current public bootstrap's trusted, already-downloaded release route.
    env = dict(os.environ, VOYAGE_RELEASE_DIR=str(candidate))
    result = command(['/bin/sh', str(script), 'upgrade', '--no-start'], 180, env, check=False)
    (WORK/'current-upgrade.log').write_bytes(result.stdout+result.stderr)
    write('upgrade-result.json', {'exit_status': result.returncode,
          'candidate_archive_sha256': args.candidate_sha256, 'bootstrap_sha256': args.install_sh_sha256})
    require(result.returncode == 0, 'upgrade is refused/unconfirmed; preserve namespace for diagnosis')
    after = observation()
    before = json.loads((WORK/'before.json').read_text())
    require(after['sessions'] == before['sessions'], 'canonical history or idle journal changed')
    require(after['catalogue_schema'] == 2 and not after['quarantine'], 'migration commit not positively observed')
    write('after.json', after)


def live_owner(args):
    """Hold one genuine provider response; upgrade refusal must not cancel its owner."""
    sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'voyage/tests'))
    from delivery_recovery import Fixture
    received, release = threading.Event(), threading.Event()
    class Held(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args): pass
        def do_POST(self):
            length = int(self.headers['Content-Length'])
            require(length <= 1024*1024, 'synthetic request exceeds bound')
            self.rfile.read(length)
            self.server.requests += 1
            received.set()
            if not release.wait(90):
                self.send_error(503)
                return
            payload = ('data: '+json.dumps({'type':'response.completed', 'response':{
                'id':'qualification-held', 'status':'completed', 'output':[{
                    'type':'message','role':'assistant','content':[{
                        'type':'output_text','text':'Held fixture finished.'}]}],
                'usage':{'input_tokens':1,'output_tokens':1}}})+'\n\n').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
    before = observation()
    require(before['catalogue_schema'] == 1, 'live refusal requires untouched legacy source')
    candidate = archive(args.candidate_archive, args.candidate_sha256, 'candidate')
    script = args.install_sh.resolve(strict=True)
    require(digest(script) == args.install_sh_sha256, 'bootstrap source identity mismatch')
    fixture = object.__new__(Fixture)
    fixture.binaries, fixture.root, fixture.directory = installed_bin(), WORK, STATE
    fixture.workspace, fixture.sessions = WORK/'workspace', []
    fixture.env = dict(os.environ, RECOVERY_FIXTURE_KEY='synthetic-fixture-key')
    fixture.provider = http.server.ThreadingHTTPServer(('127.0.0.1',0), Held)
    fixture.provider.requests = 0
    threading.Thread(target=fixture.provider.serve_forever, daemon=True).start()
    manager('set-environment', 'RECOVERY_FIXTURE_KEY=synthetic-fixture-key')
    try:
        manager('restart', NAME)
        ready_catalogue(fixture)
        session = fixture.session()
        exact = fixture.submit(session, 'held ordinary legacy owner qualification')
        fixture.command(session, exact)
        require(received.wait(20), 'actual provider request not observed')
        owner = fixture.request({'op':'inspect','session_id':session})
        active = fixture.command(session, {'op':'snapshot'})
        pid_before = service_identity()['pid']
        result = command(['/bin/sh',str(script),'upgrade','--no-start'], 60,
                         dict(os.environ, VOYAGE_RELEASE_DIR=str(candidate)), check=False)
        (WORK/'live-refusal.log').write_bytes(result.stdout+result.stderr)
        require(result.returncode != 0, 'active legacy upgrade unexpectedly admitted')
        require(service_identity()['pid'] == pid_before and installed_bin() == fixture.binaries,
                'refused update changed service or pointer')
        require(not (STATE/'update-quarantine.json').exists(), 'active refusal created quarantine')
        require(not list((INSTALL/'updates').glob('*.json')), 'active refusal created operation receipt')
        require(fixture.request({'op':'inspect','session_id':session}) == owner,
                'active execution owner changed')
        current = fixture.command(session, {'op':'snapshot'})
        require(current['run'] == active['run'] and current['revision'] == active['revision'],
                'active run was cancelled or changed by upgrade refusal')
        release.set()
        finished = fixture.finished(session)
        require(finished['run']['state'] == 'completed' and fixture.provider.requests == 1,
                'held original inference did not complete exactly once')
        fixture.suspended(session)
        write('live-owner-refusal.json', {'session':session, 'command_id':exact['command_id'],
              'installer_exit':result.returncode, 'provider_requests':1, 'completed':True})
    finally:
        release.set()
        fixture.provider.shutdown()
        fixture.provider.server_close()
        manager('unset-environment','RECOVERY_FIXTURE_KEY')


def kill_at(args):
    """Arm in another terminal before ONE explicit upgrade. Missed windows never pass."""
    proc = Path('/proc')/str(args.installer_pid)
    require(proc.stat().st_uid == 1000, 'fault target UID mismatch')
    executable = (proc/'exe').resolve(strict=True)
    require(executable == Path(json.loads((WORK/'candidate-bin.json').read_text())), 'fault target executable not pinned')
    descriptor = os.pidfd_open(args.installer_pid)
    try:
        deadline = time.monotonic()+120
        while time.monotonic() < deadline:
            records = list((INSTALL/'updates').glob('*.json'))
            for path in records:
                record = json.loads(path.read_text())
                if not record.get('legacy_mode'):
                    continue
                proof = record.get('legacy_proof')
                seen = (args.boundary == 'snapshot' and proof is not None or
                        args.boundary == 'committing' and record.get('phase') == 'committing' or
                        args.boundary == 'restored' and proof and
                        (Path(proof['stage'])/'legacy-restored.json').exists())
                if args.boundary.startswith('helper-'):
                    seen = False
                    children = (proc/'task'/str(args.installer_pid)/'children').read_text().split()
                    for child in children:
                        child_proc = Path('/proc')/child
                        try:
                            argv = (child_proc/'cmdline').read_bytes().split(b'\0')
                            if (child_proc.stat().st_uid == 1000 and len(argv) > 5 and
                                    argv[1:3] == [b'-I', b'-c'] and
                                    argv[3].startswith(b'\"\"\"Quiescent ordinary schema-1 snapshot/proof.') and
                                    argv[4].decode() == args.boundary.removeprefix('helper-')):
                                helper = os.pidfd_open(int(child))
                                try: signal.pidfd_send_signal(helper, signal.SIGKILL)
                                finally: os.close(helper)
                                write('death-'+args.boundary+'.json', {'operation_id': record['operation_id'],
                                      'boundary': args.boundary, 'helper_signal_requested': True})
                                return
                        except (FileNotFoundError, ProcessLookupError):
                            pass
                if seen:
                    if getattr(args, 'context', None):
                        signal.pidfd_send_signal(descriptor, signal.SIGSTOP)
                        try:
                            require((STATE/'update-quarantine.json').exists(), 'reviewed fault window missed')
                            if args.context == 'account-namespace':
                                changed = WORK/'independent-data'
                                changed.mkdir(mode=0o700)
                                manager('set-environment', 'XDG_DATA_HOME='+str(changed))
                                original = {'XDG_DATA_HOME': str(HOME/'.local/share')}
                            elif args.context == 'unit':
                                original = {'unit_sha256': digest(UNIT)}
                                (WORK/'independent-original.service').write_bytes(UNIT.read_bytes())
                                with UNIT.open('ab') as unit: unit.write(b'\n# Independent qualification operator edit\n')
                                manager('daemon-reload')
                            else:
                                original = {'unit_file_state': manager('show',NAME,'--property=UnitFileState','--value')}
                                manager('disable', NAME)
                            write('changed-'+args.context+'.json', {'operation_id':record['operation_id'],
                                  'original':original, 'independent_change_applied':True})
                        finally:
                            signal.pidfd_send_signal(descriptor, signal.SIGCONT)
                        return
                    signal.pidfd_send_signal(descriptor, signal.SIGKILL)
                    write('death-'+args.boundary+'.json', {'operation_id': record['operation_id'],
                          'boundary': args.boundary, 'signal_requested': True})
                    return
            time.sleep(.002)
        raise RuntimeError('boundary not reached; death case remains unqualified')
    finally:
        os.close(descriptor)



class ProcessNotLive(RuntimeError):
    pass


def process_start(pid, allow_exited=False):
    raw = (Path('/proc')/str(pid)/'stat').read_text()
    require(len(raw) <= 8192 and ')' in raw, 'bounded process identity unavailable')
    fields = raw.rsplit(')', 1)[1].split()
    require(len(fields) >= 20, 'bounded process identity unavailable')
    if not allow_exited and fields[0] in ('Z', 'X', 'x'):
        raise ProcessNotLive('process is not live')
    return int(fields[19])


def pidfd_exited(descriptor):
    poll = select.poll()
    poll.register(descriptor, select.POLLIN)
    return any(events & select.POLLIN for _, events in poll.poll(0))


def exact_candidate(pid, candidate, expected_sha):
    """Read-only ordinary witness; startup denial/missing remains pending."""
    descriptor = None
    try:
        descriptor = os.pidfd_open(pid)
        if pidfd_exited(descriptor):
            return 'missing_pending', None
        proc = Path('/proc')/str(pid)
        if proc.stat().st_uid != 1000:
            return 'not_candidate', None
        status = (proc/'status').read_text()
        require(len(status) <= 65536, 'process status exceeds bound')
        uids = next((line.split()[1:] for line in status.splitlines() if line.startswith('Uid:')), [])
        if uids != ['1000'] * 4:
            return 'not_candidate', None
        started = process_start(pid)
        if os.readlink(proc/'exe') != str(candidate):
            return 'not_candidate', None
        image = os.open(proc/'exe', os.O_RDONLY | os.O_CLOEXEC)
        with os.fdopen(image, 'rb') as source:
            meta = os.fstat(source.fileno())
            require(stat.S_ISREG(meta.st_mode) and 0 < meta.st_size <= 128*1024*1024,
                    'candidate image bound refused')
            named = candidate.stat()
            pin = (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns)
            require(pin == (named.st_dev, named.st_ino, named.st_size, named.st_mtime_ns, named.st_ctime_ns),
                    'candidate named image changed')
            digest_value = hashlib.sha256()
            total = 0
            while chunk := source.read(1024*1024):
                total += len(chunk)
                require(total <= meta.st_size, 'candidate image grew during observation')
                digest_value.update(chunk)
            after = os.fstat(source.fileno())
            require(total == meta.st_size and pin == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
                    'candidate image changed during observation')
            require(digest_value.hexdigest() == expected_sha, 'candidate qualified image hash mismatch')
        if pidfd_exited(descriptor) or process_start(pid) != started:
            return 'missing_pending', None
        # Retain only nonsecret identity. The open pidfd binds the actual process.
        result = {'descriptor':descriptor, 'pid':pid, 'start_ticks':started, 'uid':1000,
                  'executable_sha256':expected_sha, 'image_pin':pin}
        descriptor = None
        return 'verified', result
    except PermissionError:
        return 'permission_pending', None
    except (FileNotFoundError, ProcessLookupError, ProcessNotLive):
        return 'missing_pending', None
    finally:
        if descriptor is not None:
            os.close(descriptor)


def still_candidate(witness, candidate):
    """Last identity gate immediately before the one pidfd signal."""
    if pidfd_exited(witness['descriptor']):
        return False
    proc = Path('/proc')/str(witness['pid'])
    if proc.stat().st_uid != 1000 or process_start(witness['pid']) != witness['start_ticks']:
        return False
    status = (proc/'status').read_text()
    require(len(status) <= 65536, 'process status exceeds bound')
    if next((line.split()[1:] for line in status.splitlines() if line.startswith('Uid:')), []) != ['1000']*4:
        return False
    if os.readlink(proc/'exe') != str(candidate):
        return False
    meta = (proc/'exe').stat()
    return witness['image_pin'] == (meta.st_dev,meta.st_ino,meta.st_size,meta.st_mtime_ns,meta.st_ctime_ns)


def retired(witness):
    if not pidfd_exited(witness['descriptor']):
        return False
    try:
        return process_start(witness['pid'], allow_exited=True) != witness['start_ticks']
    except (FileNotFoundError, ProcessLookupError):
        return True
    except RuntimeError:
        # A zombie is exited but has not been positively reaped yet.
        return False


def fail_startup():
    """Only positive ordinary candidate witnesses receive the bounded fault."""
    write('fail-startup-attempt.json', {'monitor_pid':os.getpid(), 'monitor_uid':os.getuid(),
          'monitor_start_ticks':process_start(os.getpid()), 'status':'armed', 'maximum_signals':5})
    deadline = time.monotonic()+120
    witnesses, pending = [], {'permission_pending':0,'missing_pending':0,'not_candidate':0}
    target_fence = None
    expected_sha = None
    passed = False
    try:
        while time.monotonic() < deadline:
            if (STATE/'update-quarantine.json').exists():
                fence = json.loads((STATE/'update-quarantine.json').read_text())
                require(set(fence) == {'schema_version','operation_id','previous_release','candidate_release'}
                        and type(fence['schema_version']) is int and fence['schema_version'] == 1,
                        'unknown quarantine identity refused')
                release = fence['candidate_release']
                require(isinstance(release,str) and 0 < len(release) <= 256 and Path(release).name == release
                        and release not in ('.','..'), 'candidate release outside fixed installation')
                candidate = INSTALL/'releases'/release/'bin/vessel'
                if target_fence is None:
                    target_fence = fence
                    staged_installer = Path(json.loads((WORK/'candidate-bin.json').read_text()))
                    require(staged_installer.is_relative_to(WORK/'candidate'), 'candidate staging outside owned fixture')
                    expected_sha = digest(staged_installer.with_name('vessel'))
                require(fence == target_fence, 'fault operation/fence changed; no further signals')
                # Exhausted injection budget waits for observed rollback; it does
                # not declare success or signal a sixth/replacement process.
                if len(witnesses) < 5:
                    pid = int(manager('show', NAME, '--property=MainPID', '--value'))
                    if pid > 0 and not any(w['pid'] == pid and not pidfd_exited(w['descriptor']) for w in witnesses):
                        category, witness = exact_candidate(pid, candidate, expected_sha)
                        if witness is None:
                            pending[category] += 1
                        else:
                            try:
                                if (sqlite_observe(STATE/'catalogue.sqlite3', 'SELECT version FROM schema_version WHERE id=1') == [(2,)]
                                    and json.loads((STATE/'update-quarantine.json').read_text()) == target_fence
                                    and int(manager('show', NAME, '--property=MainPID', '--value')) == witness['pid']
                                    and still_candidate(witness, candidate)):
                                    signal.pidfd_send_signal(witness['descriptor'], signal.SIGKILL)
                                    witness['signal_delivered'] = True
                                    witnesses.append(witness)
                                    witness = None
                            except PermissionError:
                                pending['permission_pending'] += 1
                            except (FileNotFoundError, ProcessLookupError, ProcessNotLive):
                                pending['missing_pending'] += 1
                            finally:
                                if witness is not None:
                                    os.close(witness['descriptor'])
            elif witnesses:
                try:
                    after = observation()
                except PermissionError:
                    pending['permission_pending'] += 1
                    time.sleep(.01)
                    continue
                except (FileNotFoundError, ProcessLookupError):
                    pending['missing_pending'] += 1
                    time.sleep(.01)
                    continue
                before = json.loads((WORK/'before.json').read_text())
                require(after['catalogue_schema'] == 1 and after['sessions'] == before['sessions'],
                        'old schema/history rollback unconfirmed; retain all state')
                require(after['service']['executable_sha256'] == before['service']['executable_sha256'],
                        'previous supervisor was not actually reactivated')
                try:
                    all_retired = all(retired(w) for w in witnesses)
                except PermissionError:
                    pending['permission_pending'] += 1
                    all_retired = False
                if not all_retired:
                    time.sleep(.01)
                    continue
                # Use the actual old saved reader only after positive rollback.
                sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'voyage/tests'))
                from delivery_recovery import Fixture
                reader = object.__new__(Fixture)
                reader.directory = STATE
                for session, saved in before['sessions'].items():
                    snapshot = reader.command(session, {'op':'snapshot'})
                    require(snapshot['messages'] == saved['messages'], 'old helper cannot read canonical history')
                signals = [{k:v for k,v in w.items() if k not in ('descriptor','image_pin')} for w in witnesses]
                require(signals and all(w['signal_delivered'] for w in signals), 'fault delivery unconfirmed')
                write('forced-startup-rollback.json', {'candidate_pids_signalled':[w['pid'] for w in witnesses],
                      'signals':signals, 'all_signalled_candidates_exited_and_reaped':True,
                      'pending_observations':pending, 'observed':after})
                passed = True
                return
            time.sleep(.01)
        raise RuntimeError('migrated startup fault/rollback window not positively observed; case remains unqualified')
    finally:
        records = []
        for witness in witnesses:
            try:
                retirement = retired(witness)
            except (OSError, RuntimeError, ValueError):
                retirement = None
            records.append({**{k:v for k,v in witness.items() if k not in ('descriptor','image_pin')},
                            'exit_and_reaping_observed':retirement})
            os.close(witness['descriptor'])
        write('fail-startup-monitor-result.json', {'status':'passed' if passed else 'unqualified',
              'pending_observations':pending, 'signals':records, 'maximum_signals':5})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ack-disposable-ct119', action='store_true', required=True)
    commands = parser.add_subparsers(dest='action', required=True)
    commands.add_parser('preflight')
    for name in ('seed', 'seed-existing'):
        initial = commands.add_parser(name); initial.add_argument('--old-archive', type=Path, required=True)
    for name in ('upgrade', 'live-owner'):
        current = commands.add_parser(name)
        current.add_argument('--candidate-archive', type=Path, required=True)
        current.add_argument('--candidate-sha256', required=True)
        current.add_argument('--install-sh', type=Path, required=True)
        current.add_argument('--install-sh-sha256', required=True)
    commands.add_parser('observe')
    commands.add_parser('fail-startup')
    fault = commands.add_parser('kill-at')
    selection = fault.add_mutually_exclusive_group(required=True)
    selection.add_argument('--installer-pid', type=int)
    selection.add_argument('--auto-local-owner', action='store_true')
    fault.add_argument('--boundary', choices=['snapshot','restored','committing','helper-snapshot','helper-restore','helper-verify'], required=True)
    context = commands.add_parser('change-at')
    context.add_argument('--installer-pid',type=int,required=True)
    context.add_argument('--context',choices=['account-namespace','unit','enablement'],required=True)
    context.set_defaults(boundary='snapshot')
    args = parser.parse_args()
    preflight()
    if args.action in ('seed', 'seed-existing'): seed(args.old_archive, existing=args.action == 'seed-existing')
    elif args.action == 'upgrade': upgrade(args)
    elif args.action == 'live-owner': live_owner(args)
    elif args.action == 'observe': print(json.dumps(observation(), indent=2))
    elif args.action == 'kill-at':
        from native_legacy_faults import run
        run(sys.modules[__name__], args)
    elif args.action == 'change-at': kill_at(args)
    elif args.action == 'fail-startup': fail_startup()
    else: print('Native prerequisites observed; no release qualification claimed.')

if __name__ == '__main__':
    main()
