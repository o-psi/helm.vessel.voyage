"""Read-only, private qualification resource proof on the actual Linux host.

JSON stdin supplies two exact fixture roots and capacity directory, never a
credential. No launch, signal, lock acquisition, deletion or receipt repair.
Before returns private process identities; after returns only fixed booleans.
"""
import argparse
import hashlib
import importlib.util
import re
import time
import json
import os
from pathlib import Path
import stat
import sys
import uuid
import sqlite3
import base64

# Explicit maintained sibling import supports isolated Python (-I), without
# user-site startup or adding a caller-controlled module directory.
_cost_spec = importlib.util.spec_from_file_location('qualification_cost', Path(__file__).with_name('host_browser_cost.py'))
_cost = importlib.util.module_from_spec(_cost_spec)
_cost_spec.loader.exec_module(_cost)
identity, inventory = _cost.identity, _cost.inventory


def private(path, limit=8192, with_identity=False):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(fd, 'rb') as stream:
        meta = os.fstat(stream.fileno())
        assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid()
        assert meta.st_nlink == 1 and not meta.st_mode & 0o077 and meta.st_size <= limit
        data = stream.read(limit+1); after = os.fstat(stream.fileno())
        assert len(data) <= limit
        assert (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns) == \
               (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
        assert after.st_uid == os.getuid() and after.st_nlink == 1 and not after.st_mode & 0o077
        named = Path(path).lstat()
        pin = (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns)
        assert pin == (named.st_dev, named.st_ino, named.st_size, named.st_mtime_ns, named.st_ctime_ns)
    return (data, pin) if with_identity else data


def directory(value):
    path = Path(value)
    assert path.is_absolute() and path.resolve() == path and str(path) != '/'
    meta = path.lstat()
    assert stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.getuid() and not meta.st_mode & 0o077
    return path


def slots(capacity, session):
    entries = list(capacity.iterdir()); assert len(entries) <= 128
    return [p.name for p in entries if p.name.startswith('browser-slot-')
            and private(p, 128).decode() == session]


def before(item, capacity):
    root = directory(item['root'])
    lock = json.loads(private(root/'worker.lock'))
    assert lock['browser'] == item['browser_id']
    worker = identity(lock['pid']); assert worker and worker['state'] != 'Z'
    guardian = identity(worker['ppid']); assert guardian and guardian['state'] != 'Z'
    assert Path(f'/proc/{worker["pid"]}').stat().st_uid == os.getuid()
    assert Path(f'/proc/{guardian["pid"]}').stat().st_uid == os.getuid()
    # This is the worker's pinned actual parent, not a global command search.
    command = Path(f'/proc/{guardian["pid"]}/cmdline').read_bytes()
    assert len(command) <= 16384
    words = command.rstrip(b'\0').split(b'\0')
    assert len(words) >= 5 and Path(os.fsdecode(words[1])).name == 'guardian.py'
    scratch = directory(os.fsdecode(words[-1]))
    assert scratch.parent == Path('/tmp') and scratch.name.startswith('vhb-')
    root_home = Path(f'/proc/{guardian["pid"]}/environ').read_bytes()
    assert len(root_home) <= 32768 and b'HOME='+os.fsencode(root) in root_home.split(b'\0')
    current = identity(guardian['pid']); current_worker = identity(worker['pid'])
    assert current and current_worker
    assert all(current[k] == guardian[k] for k in ('pid','start_ticks','ppid'))
    assert all(current_worker[k] == worker[k] for k in ('pid','start_ticks','ppid'))
    owned = inventory([dict(label=item['label'], pid=guardian['pid'],
                            start_ticks=guardian['start_ticks'], descendants=True)])
    assert worker['pid'] in owned and len(owned) <= 128
    exact_slots = slots(capacity, item['session_id']); assert len(exact_slots) == 1
    assert not os.path.lexists(root/'guardian-cleanup.json') and not os.path.lexists(root/'.guardian-cleanup.json.new'), 'prior/unknown cleanup cannot qualify this browser'
    return {'label':item['label'], 'session_id':item['session_id'], 'browser_id':item['browser_id'],
            'root':str(root), 'scratch':str(scratch), 'slots':exact_slots,
            'processes':[{'pid':p['pid'], 'start_ticks':p['start_ticks'], 'ppid':p['ppid']}
                         for p in owned.values()]}


def after(item, proof, capacity):
    root = directory(item['root'])
    assert all(item[k] == proof[k] for k in ('label','session_id','browser_id','root'))
    assert len(proof['processes']) <= 128 and len(proof['slots']) == 1
    observed = json.loads(private(root/'guardian-cleanup.json', 4096))
    identities_gone = all(not (seen := identity(p['pid'])) or seen['start_ticks'] != p['start_ticks']
                          for p in proof['processes'])
    # Guardian is the subreaper: its positive observed marker covers descendants
    # born after the bounded pre-close inventory as well as these pinned PIDs.
    result = {'label':item['label'], 'guardian_observed':observed.get('observed') is True,
              'pinned_processes_gone':identities_gone,
              'scratch_removed':not Path(proof['scratch']).exists(),
              'worker_lock_removed':not (root/'worker.lock').exists(),
              'owned_slots_removed':not slots(capacity,item['session_id'])}
    result['complete'] = all(value is True for key,value in result.items() if key != 'label')
    return result



def live(pid, start=None):
    assert type(pid) is int and pid > 1
    observed = identity(pid)
    assert observed and observed['state'] not in ('Z', 'T', 't', 'X', 'x')
    assert Path(f'/proc/{pid}').stat().st_uid == os.getuid()
    if start is not None:
        assert type(start) is int and observed['start_ticks'] == start
    return observed


def unchanged(observed):
    current = live(observed['pid'], observed['start_ticks'])
    assert current['ppid'] == observed['ppid']


def program(spec, name):
    assert isinstance(spec, dict) and set(spec) == {'path', 'sha256'}
    path = Path(spec['path'])
    assert path.is_absolute() and path.resolve() == path
    assert re.fullmatch('[a-f0-9]{64}', spec['sha256'])
    expected = {'worker': 'worker.mjs', 'guardian': 'guardian.py', 'helm': 'helm', 'voyage': 'voyage', 'node': 'node'}
    if name == 'python':
        assert re.fullmatch(r'python3(?:\.\d+)?', path.name)
    else:
        assert path.name == expected[name]
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    with os.fdopen(descriptor, 'rb') as source:
        meta = os.fstat(source.fileno())
        assert stat.S_ISREG(meta.st_mode) and meta.st_uid in (0, os.getuid()) and not meta.st_mode & 0o022
        assert 0 < meta.st_size <= 256 * 1024 * 1024
        digest = hashlib.sha256()
        head = source.read(4)
        digest.update(head)
        total = len(head)
        if name not in ('worker', 'guardian'):
            assert head == b'\x7fELF' and meta.st_mode & 0o111
        while chunk := source.read(1024 * 1024):
            total += len(chunk)
            assert total <= meta.st_size
            digest.update(chunk)
        assert total == meta.st_size
        after, named = os.fstat(source.fileno()), path.lstat()
        pin = (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns)
        for seen in (after, named):
            assert pin == (seen.st_dev, seen.st_ino, seen.st_size, seen.st_mtime_ns, seen.st_ctime_ns)
        assert digest.hexdigest() == spec['sha256']
    return {'path': str(path), 'inode': pin}


def executable(observed, spec):
    unchanged(observed)
    path = Path(f'/proc/{observed["pid"]}/exe')
    assert os.readlink(path) == spec['path']
    meta = path.stat()
    assert (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns) == spec['inode']
    unchanged(observed)


def words(observed):
    unchanged(observed)
    raw = Path(f'/proc/{observed["pid"]}/cmdline').read_bytes()
    assert 0 < len(raw) <= 16384 and raw.endswith(b'\0')
    result = [os.fsdecode(value) for value in raw[:-1].split(b'\0')]
    unchanged(observed)
    return result


def initialize_ledger(request):
    # Only the same-host native bootstrap can use this route. No client PID is
    # interpreted in another host's PID namespace and no process is signalled.
    assert isinstance(request, dict) and set(request) == {'schema', 'capacity', 'sessions', 'clients', 'programs'}
    assert type(request['schema']) is int and request['schema'] == 1 and os.getuid() > 0
    assert isinstance(request['sessions'], list) and 1 <= len(request['sessions']) <= 2
    assert isinstance(request['clients'], list) and len(request['clients']) == 2
    assert len({c['pid'] for c in request['clients']}) == 2 and len({c['label'] for c in request['clients']}) == 2
    assert len({s['session_id'] for s in request['sessions']}) == len(request['sessions'])
    assert isinstance(request['programs'], dict) and set(request['programs']) == {'helm', 'node', 'worker', 'guardian', 'python', 'voyage'}
    pins = {name: program(spec, name) for name, spec in request['programs'].items()}
    assert Path(pins['worker']['path']).parent == Path(pins['guardian']['path']).parent
    capacity = directory(request['capacity'])
    capacity_meta = capacity.lstat()
    capacity_inode = (capacity_meta.st_dev, capacity_meta.st_ino)
    clients = []
    for client in request['clients']:
        assert set(client) == {'label', 'pid', 'start_ticks', 'descendants'} and client['descendants'] is True
        assert re.fullmatch(r'native-[A-Za-z0-9_.-]{1,40}', client['label'])
        observed = live(client['pid'], client['start_ticks'])
        executable(observed, pins['helm'])
        clients.append(observed)
    roots, bindings, all_owned = [], [], set()
    for item in request['sessions']:
        assert set(item) == {'label', 'session_id', 'incarnation', 'browser_id', 'root'}
        assert re.fullmatch(r'[A-Za-z0-9_.-]{1,40}', item['label'])
        assert 'native-' + item['label'] in {c['label'] for c in request['clients']}
        for key in ('session_id', 'incarnation', 'browser_id'):
            assert str(uuid.UUID(item[key])) == item[key]
        root = directory(item['root'])
        root_meta = root.lstat()
        root_inode = (root_meta.st_dev, root_meta.st_ino)
        raw, lock_inode = private(root/'worker.lock', with_identity=True)
        lock = json.loads(raw)
        assert set(lock) == {'pid', 'browser'} and lock['browser'] == item['browser_id']
        worker = live(lock['pid'])
        guardian = live(worker['ppid'])
        voyage = live(guardian['ppid'])
        executable(worker, pins['node'])
        executable(guardian, pins['python'])
        executable(voyage, pins['voyage'])
        worker_words, guardian_words = words(worker), words(guardian)
        assert len(worker_words) == 2 and len(guardian_words) == 5
        assert Path(worker_words[0]).resolve() == Path(pins['node']['path'])
        assert worker_words[1] == pins['worker']['path']
        assert Path(guardian_words[0]).resolve() == Path(pins['python']['path'])
        assert guardian_words[1:4] == [pins['guardian']['path'], pins['node']['path'], pins['worker']['path']]
        proof = before(item, capacity)
        assert proof['scratch'] == guardian_words[4]
        assert slots(capacity, item['session_id']) == proof['slots']
        assert all(p['pid'] not in all_owned for p in proof['processes'])
        assert voyage['pid'] not in all_owned and voyage['pid'] not in {p['pid'] for p in proof['processes']}
        assert not (({p['pid'] for p in proof['processes']} | {voyage['pid']}) & {c['pid'] for c in clients})
        all_owned.update(p['pid'] for p in proof['processes'])
        all_owned.add(voyage['pid'])
        # Parent as a leaf + guardian descendants prevents double-counting the
        # browser while retaining the owning Voyage's CPU/RSS separately.
        roots.extend([{'label': 'voyage-' + item['label'], 'pid': voyage['pid'], 'start_ticks': voyage['start_ticks'], 'descendants': False},
                      {'label': 'browser-' + item['label'], 'pid': guardian['pid'], 'start_ticks': guardian['start_ticks'], 'descendants': True}])
        _, last_lock = private(root/'worker.lock', with_identity=True)
        assert last_lock == lock_inode
        verified_root = directory(item['root']).lstat()
        assert (verified_root.st_dev, verified_root.st_ino) == root_inode
        for seen in (worker, guardian, voyage):
            unchanged(seen)
        assert not os.path.lexists(root/'guardian-cleanup.json') and not os.path.lexists(root/'.guardian-cleanup.json.new')
        bindings.append({'label': item['label'], 'session_id': item['session_id'], 'incarnation': item['incarnation'],
                         'browser_id': item['browser_id'], 'root_inode': list(root_inode), 'lock_inode': list(lock_inode),
                         'worker': {k: worker[k] for k in ('pid', 'start_ticks', 'ppid')},
                         'guardian': {k: guardian[k] for k in ('pid', 'start_ticks', 'ppid')},
                         'voyage': {k: voyage[k] for k in ('pid', 'start_ticks', 'ppid')}})
    for seen in clients:
        unchanged(seen)
    verified_capacity = directory(request['capacity']).lstat()
    assert (verified_capacity.st_dev, verified_capacity.st_ino) == capacity_inode
    return {'schema': 1, 'captured_at_ms': int(time.time() * 1000),
            'scope': 'selected current Voyage leaves and browser guardian descendants; excludes unrelated host services',
            'clients': request['clients'], 'bindings': bindings, 'ledger': {'schema': 1, 'roots': roots}}

def validate_idle_metadata(value, pid, browser, source_sha256):
    assert set(value)=={'schema','pid','browser','sequence','observed_at_ms','browser_running','zero_viewers','recorder_stop_observed','source_sha256'}
    assert type(value['schema']) is int and value['schema']==1 and type(value['pid']) is int and value['pid']==pid and value['browser']==browser
    assert type(value['sequence']) is int and value['sequence']>0 and type(value['observed_at_ms']) is int and value['observed_at_ms']>0
    assert value['source_sha256']==source_sha256 and re.fullmatch('[a-f0-9]{64}',source_sha256)
    assert all(value[key] is True for key in ('browser_running','zero_viewers','recorder_stop_observed'))


def idle_observation(request):
    # Read only the selected owned worker's private source-written metadata.
    # No status/mirror call, lock acquisition, process control or timer-only proof.
    assert set(request) == {'schema','capacity','sessions','proofs','label','worker_program'} and request['schema'] == 1
    assert len(request['sessions']) == len(request['proofs']) == 2
    pairs=[(item,proof) for item,proof in zip(request['sessions'],request['proofs']) if item['label']==request['label']]
    assert len(pairs)==1
    item,proof=pairs[0]
    assert all(item[k] == proof[k] for k in ('label','session_id','browser_id','root'))
    root=directory(item['root']); capacity=directory(request['capacity'])
    worker_program=program(request['worker_program'],'worker')
    root_pin=(root.stat().st_dev,root.stat().st_ino)
    capacity_pin=(capacity.stat().st_dev,capacity.stat().st_ino)
    lock_raw,lock_pin=private(root/'worker.lock',8192,True);lock=json.loads(lock_raw)
    assert lock['browser']==item['browser_id']
    original=next(value for value in proof['processes'] if value['pid']==lock['pid'])
    pid=original['pid']; start=original['start_ticks']
    guardian=identity(original['ppid']);assert guardian and guardian['state']!='Z'
    tree=inventory([{'label':item['label'],'pid':guardian['pid'],'start_ticks':guardian['start_ticks'],'descendants':True}])
    candidates=[]
    for seen in tree.values():
        argv=words(seen)
        profiles=[arg.split('=',1)[1] for arg in argv if arg.startswith('--user-data-dir=')]
        if len(profiles)==1 and not any(arg.startswith('--type=') for arg in argv) and Path(profiles[0]).is_relative_to(Path(proof['scratch'])):
            candidates.append((seen,profiles[0]))
    assert len(candidates)==1
    browser,profile=candidates[0]
    assert directory(profile).is_relative_to(Path(proof['scratch']))
    image=Path(f'/proc/{browser["pid"]}/exe');image_path=os.readlink(image)
    assert Path(image_path).name in ('chromium','chrome','chrome-headless-shell') and not image_path.endswith(' (deleted)')
    image_meta=image.stat();image_pin=(image_meta.st_dev,image_meta.st_ino,image_meta.st_size,image_meta.st_mtime_ns,image_meta.st_ctime_ns)
    assert stat.S_ISREG(image_meta.st_mode) and image_meta.st_mode & 0o111 and not image_meta.st_mode & 0o022
    with image.open('rb') as stream:browser_source_sha256=hashlib.file_digest(stream,'sha256').hexdigest()
    def sample():
        assert (directory(root).stat().st_dev,root.stat().st_ino)==root_pin
        assert (directory(capacity).stat().st_dev,capacity.stat().st_ino)==capacity_pin
        current=identity(pid);assert current and current['start_ticks']==start and current['state']!='Z'
        assert Path(f'/proc/{pid}').stat().st_uid==os.getuid()
        active_browser=identity(browser['pid']);active_guardian=identity(guardian['pid'])
        assert active_browser and active_guardian and active_browser['state']!='Z' and active_guardian['state']!='Z'
        assert all(active_browser[key]==browser[key] for key in ('pid','start_ticks','ppid'))
        assert all(active_guardian[key]==guardian[key] for key in ('pid','start_ticks','ppid'))
        assert Path(f'/proc/{browser["pid"]}').stat().st_uid==os.getuid()
        assert os.readlink(image)==image_path
        named_image=image.stat();assert image_pin==(named_image.st_dev,named_image.st_ino,named_image.st_size,named_image.st_mtime_ns,named_image.st_ctime_ns)
        current_tree=inventory([{'label':item['label'],'pid':guardian['pid'],'start_ticks':guardian['start_ticks'],'descendants':True}])
        assert browser['pid'] in current_tree and current_tree[browser['pid']]['start_ticks']==browser['start_ticks']
        assert any(arg=='--user-data-dir='+profile for arg in words(active_browser))
        current_lock,current_pin=private(root/'worker.lock',8192,True)
        assert current_lock==lock_raw and current_pin==lock_pin
        raw,pin=private(root/'capture-observation.json',4096,True);value=json.loads(raw)
        validate_idle_metadata(value,pid,item['browser_id'],request['worker_program']['sha256'])
        assert words(current)[1]==worker_program['path']
        named=Path(worker_program['path']).lstat()
        assert worker_program['inode']==(named.st_dev,named.st_ino,named.st_size,named.st_mtime_ns,named.st_ctime_ns)
        assert slots(capacity,item['session_id'])==proof['slots']
        return raw,pin,value
    started=int(time.time()*1000);raw,pin,first=sample()
    assert 0<=started-first['observed_at_ms']<=15000
    samples=0
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        later,later_pin,value=sample();assert later==raw and later_pin==pin and value==first
        samples+=1;time.sleep(min(.25,max(0,deadline-time.monotonic())))
    later,later_pin,value=sample();assert later==raw and later_pin==pin and value==first
    ended=int(time.time()*1000)
    return {'schema':1,'phase':'idle','label':item['label'],'browser_running':True,
            'zero_viewers_observed':True,'recorder_stop_ack_observed':True,'worker_identity_unchanged':True,
            'private_metadata_identity_unchanged':True,'slot_retained':True,'browser_root_identity_unchanged':True,'browser_program_sha256':browser_source_sha256,
            'started_at_ms':started,'ended_at_ms':ended,'samples':samples,
            'scope':'owned running browser zero-viewer recorder-stop interval; private source metadata, no page or frame content'}

def local_authority(request):
    assert set(request) == {'schema','directory','vessel_id','sessions'} and type(request['schema']) is int and request['schema'] == 1
    base = directory(request['directory'])
    vessel = str(uuid.UUID(request['vessel_id']));assert vessel == request['vessel_id'] and uuid.UUID(vessel).int
    public_raw, public_pin = private(base/'identity/public.json',4096,True)
    public = json.loads(public_raw)
    assert set(public) == {'vessel_id','public_key'} and public['vessel_id'] == vessel
    assert isinstance(public['public_key'],str) and len(base64.b64decode(public['public_key'],validate=True)) == 32
    assert isinstance(request['sessions'],list) and len(request['sessions']) == 2
    identities = []
    assert len({item['label'] for item in request['sessions']}) == 2
    for item in request['sessions']:
        assert isinstance(item['label'],str) and re.fullmatch(r'[A-Za-z0-9_.-]{1,40}',item['label'])
        assert set(item) == {'label','session_id','socket_id','root','claims'}
        session = str(uuid.UUID(item['session_id']));assert session == item['session_id'] and uuid.UUID(session).int
        socket = str(uuid.UUID(item['socket_id']));assert socket == item['socket_id'] and uuid.UUID(socket).int
        root = directory(item['root']);assert root == base/'sessions'/session/'journal/host-browser'
        actor_raw, actor_pin = private(root.parent.parent/'identity/actor.json',1024,True)
        actor = json.loads(actor_raw)
        assert set(actor) == {'version','actor'} and type(actor['version']) is int and actor['version'] == 1 and set(actor['actor']) == {'installation_id','principal_id'}
        for value in actor['actor'].values():assert str(uuid.UUID(value)) == value and uuid.UUID(value).int
        claims = item['claims'];assert isinstance(claims,list) and 1 <= len(claims) <= 32
        ids, digests = [], []
        for claim in claims:
            assert claim['action'] in ('attach','control','detach')
            assert set(claim) == ({'action','command_id','binding','mode'} if claim['action']=='control' else {'action','command_id','binding'})
            binding = claim['binding'];assert set(binding) == {'incarnation','browser_id','attachment_id','tab_id','document_epoch','viewport_epoch','controller_epoch','capture_epoch'}
            for key in ('incarnation','browser_id','attachment_id','tab_id'):
                assert str(uuid.UUID(binding[key])) == binding[key] and uuid.UUID(binding[key]).int
            for key in ('document_epoch','viewport_epoch','controller_epoch','capture_epoch'):
                assert type(binding[key]) is int and 0 < binding[key] < 2**53
            if claim['action']=='control':assert claim['mode'] in ('agent','human','private')
            value = claim['command_id'];assert str(uuid.UUID(value)) == value and uuid.UUID(value).int
            ids.append(value)
            serialized = json.dumps({'operation':claim,'socket':socket,'principal':actor['actor']['principal_id']},sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()
            digests.append(hashlib.sha256(serialized).hexdigest())
        assert len(set(ids)) == len(ids)
        path = root/'receipts.sqlite3'
        raw, pin = private(path,64*1024*1024,True)
        del raw  # Never expose or retain private receipt contents.
        db = sqlite3.connect(path.as_uri()+'?mode=ro',uri=True,timeout=1)
        try:
            db.execute('PRAGMA query_only=ON');db.execute('BEGIN')
            rows = [db.execute('SELECT principal,digest,state FROM receipts WHERE id=?',(value,)).fetchone() for value in ids]
            assert all(row == (actor['actor']['principal_id'],digest,'completed') for row,digest in zip(rows,digests))
        finally:db.close()
        _, after = private(path,64*1024*1024,True);assert after == pin
        actor_after, actor_identity = private(root.parent.parent/'identity/actor.json',1024,True)
        assert actor_after == actor_raw and actor_identity == actor_pin
        identities.append({'label':item['label'],'session_id':session,'vessel_id':vessel,'socket_id':socket,
            **actor['actor'],'actor_sha256':hashlib.sha256(actor_raw).hexdigest(),
            'completed_native_receipt_ids':ids,'authority':'existing local account owner',
            'scope':'exact local runtime principal and socket-bound completed native receipt; identifiers do not grant authority'})
    assert len({item['session_id'] for item in identities}) == 2
    public_after, after_pin = private(base/'identity/public.json',4096,True)
    assert public_after == public_raw and after_pin == public_pin
    return {'schema':1,'mode':'local','identities':identities,'no_effects':True,
            'physical_vessel_identity_sha256':hashlib.sha256(public_raw).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['before','after','ledger','local-authority','idle'])
    args = parser.parse_args()
    assert __debug__ and sys.platform == 'linux'
    data = sys.stdin.buffer.read(65537); assert len(data) <= 65536
    request = json.loads(data)
    if args.phase == 'idle':
        json.dump(idle_observation(request),sys.stdout);return
    if args.phase == 'local-authority':
        json.dump(local_authority(request),sys.stdout);return
    if args.phase == 'ledger':
        json.dump(initialize_ledger(request), sys.stdout)
        return
    assert request['schema'] == 1 and len(request['sessions']) == 2
    capacity = directory(request['capacity'])
    assert len({item['session_id'] for item in request['sessions']}) == 2
    for item in request['sessions']:
        for field in ('session_id','browser_id'):
            assert str(uuid.UUID(item[field])) == item[field]
    if args.phase == 'before':
        result = {'schema':1,'phase':'before', 'proofs':[before(item,capacity) for item in request['sessions']]}
    else:
        proofs = request['proofs']; assert len(proofs) == 2
        result = {'schema':1,'phase':'after','sessions':[after(item,proof,capacity)
                  for item,proof in zip(request['sessions'],proofs)]}
    json.dump(result,sys.stdout)


if __name__ == '__main__':
    main()
