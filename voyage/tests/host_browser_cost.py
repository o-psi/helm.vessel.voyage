"""Bounded Linux process-cost observation from a private, explicitly owned PID ledger.

No process launch/signalling, service control, provider or network effects.
Run on the measured host. CPU is the observed counter delta, not power usage.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import re
import stat
import time


def identity(pid):
    try:
        fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        return {'pid': pid, 'state': fields[0], 'ppid': int(fields[1]),
                'cpu_ticks': int(fields[11])+int(fields[12]), 'start_ticks': int(fields[19])}
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def memory(pid):
    try:
        values = {}
        for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
            key, _, rest = line.partition(':')
            if key in ('Rss', 'Pss'):
                values[key.lower()+'_kib'] = int(rest.split()[0])
        return values if len(values) == 2 else None
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def inventory(roots):
    processes = {}
    entries = list(Path('/proc').iterdir())
    assert len(entries) <= 20000, 'process inventory exceeds fixture bound'
    for entry in entries:
        if entry.name.isdecimal() and (seen := identity(int(entry.name))):
            processes[seen['pid']] = seen
    found = {}
    for root in roots:
        current = processes.get(root['pid'])
        if not current or current['start_ticks'] != root['start_ticks']:
            raise RuntimeError('owned root exited or identity changed during measurement')
        own = {current['pid']: current}
        if root['descendants']:
            for _ in range(64):
                added = {pid: item for pid, item in processes.items() if pid not in own
                         and item['ppid'] in own
                         and own[item['ppid']]['start_ticks'] <= item['start_ticks']}
                if not added:
                    break
                own.update(added)
                assert len(own) <= 4096, 'owned descendant inventory exceeds fixture bound'
            else:
                raise RuntimeError('owned descendant depth exceeds fixture bound')
        assert not set(found).intersection(own), 'overlapping owned roots would double count'
        found.update(own)
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ledger', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--seconds', type=float, default=10)
    parser.add_argument('--interval', type=float, default=.25)
    args = parser.parse_args()
    assert platform.system() == 'Linux', 'Linux process evidence only'
    assert 1 <= args.seconds <= 60 and .1 <= args.interval <= 1
    assert args.seconds / args.interval <= 600
    meta = args.ledger.lstat()
    assert stat.S_ISREG(meta.st_mode) and meta.st_uid == os.getuid()
    assert meta.st_nlink == 1 and not meta.st_mode & 0o077 and meta.st_size <= 8192
    assert args.ledger.resolve() == args.ledger.absolute()
    ledger = json.loads(args.ledger.read_text())
    assert set(ledger) == {'schema', 'roots'} and ledger['schema'] == 1
    roots = ledger['roots']
    assert isinstance(roots, list) and 1 <= len(roots) <= 16
    for root in roots:
        assert set(root) == {'label', 'pid', 'start_ticks', 'descendants'}
        assert re.fullmatch(r'[A-Za-z0-9_.-]{1,48}', root['label'])
        assert type(root['pid']) is int and root['pid'] > 1
        assert type(root['start_ticks']) is int and root['start_ticks'] > 0
        assert type(root['descendants']) is bool
    os.umask(0o077)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    initial, last = {}, {}
    samples = []
    result = {'schema': 1, 'captured_at': datetime.now(timezone.utc).isoformat(),
              'platform': platform.platform(), 'clock_ticks_per_second': os.sysconf('SC_CLK_TCK'),
              'labels': [r['label'] for r in roots], 'scope': 'explicit owned roots and selected descendants',
              'cpu_scope': 'observed deltas only; short-lived unseen descendants can be missed',
              'rss_scope': 'sum of RSS includes shared pages; PSS apportions shared pages', 'status': 'pending'}
    try:
        while True:
            owned = inventory(roots)
            complete = []
            unavailable = 0
            for item in owned.values():
                key = (item['pid'], item['start_ticks'])
                initial.setdefault(key, item['cpu_ticks'])
                last[key] = item['cpu_ticks']
                value = memory(item['pid'])
                if value is None:
                    unavailable += 1
                else:
                    complete.append(value)
            sample = {'elapsed_seconds': time.monotonic()-started, 'processes': len(owned),
                      'zombies': sum(item['state'] == 'Z' for item in owned.values()),
                      'memory_unavailable': unavailable,
                      'rss_kib': None if unavailable else sum(v['rss_kib'] for v in complete),
                      'pss_kib': None if unavailable else sum(v['pss_kib'] for v in complete)}
            samples.append(sample)
            if sample['elapsed_seconds'] >= args.seconds:
                break
            time.sleep(min(args.interval, args.seconds-sample['elapsed_seconds']))
        result.update(status='observed', elapsed_seconds=time.monotonic()-started,
                      observed_cpu_seconds=sum(max(0, last[k]-first) for k, first in initial.items())/result['clock_ticks_per_second'],
                      observed_process_identities=len(initial), samples=samples)
    except Exception as error:
        result.update(status='failed', category=type(error).__name__, samples=samples)
        raise
    finally:
        # Never publish PID/starttime/path/command/credentials in the summary.
        with args.output.open('x') as output:
            json.dump(result, output, indent=2)


if __name__ == '__main__':
    main()
