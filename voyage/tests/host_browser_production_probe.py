"""Read-only, private qualification resource proof on the actual Linux host.

JSON stdin supplies two exact fixture roots and capacity directory, never a
credential. No launch, signal, lock acquisition, deletion or receipt repair.
Before returns private process identities; after returns only fixed booleans.
"""
import argparse
import json
import os
from pathlib import Path
import stat
import sys
import uuid

from host_browser_cost import identity, inventory


def private(path, limit=8192):
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
    return data


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
    assert not (root/'guardian-cleanup.json').exists(), 'prior cleanup marker cannot qualify this browser'
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['before','after'])
    args = parser.parse_args()
    assert __debug__ and sys.platform == 'linux'
    data = sys.stdin.buffer.read(65537); assert len(data) <= 65536
    request = json.loads(data); assert request['schema'] == 1 and len(request['sessions']) == 2
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
