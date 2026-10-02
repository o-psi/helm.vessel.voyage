#!/usr/bin/env python3
"""Reconcile existing audited LLVM JSON/LCOV, without invoking LLVM or Cargo.

Supported input is the established Rust code-region-only mapping. Refuse newer
mapping kinds instead of approximating them. Outputs are accounting evidence,
not source exclusions, reachability classifications or a coverage measurement.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def function_lines(function):
    regions = function['regions']
    require(len(function['filenames']) == 1 and regions,
            'requires single-file, nonempty function mappings')
    require(all(r[5] == 0 and r[7] == 0 and tuple(r[:2]) < tuple(r[2:4])
                for r in regions), 'unsupported region file/kind/empty span')
    entries = collections.defaultdict(list)
    for region in regions:
        entries[region[0]].append(region)
    counts = {}
    for line in range(min(r[0] for r in regions), max(r[2] for r in regions)+1):
        # The preceding line's last segment wraps into this line, even when a
        # region ends at column one. The innermost region supplies that counter.
        wrapped = [r for r in regions if tuple(r[:2]) < (line, 0)
                   and tuple(r[2:4]) >= (line, 1)]
        count = (max(wrapped, key=lambda r: (r[0], r[1], -r[2], -r[3]))[4]
                 if wrapped else 0)
        if entries[line]:
            count = max(count, max(r[4] for r in entries[line]))
        if wrapped or entries[line]:
            counts[line] = count
    return counts


def read_lcov(path):
    files = {}
    for block in path.read_text().split('end_of_record'):
        source = next((v[3:] for v in block.splitlines() if v.startswith('SF:')), None)
        if source is None:
            continue
        require(source not in files, 'duplicate LCOV source record')
        counts = {}
        for value in block.splitlines():
            if value.startswith('DA:'):
                line, count, *_ = value[3:].split(',')
                require(int(line) not in counts, 'duplicate LCOV line address')
                counts[int(line)] = int(count)
        files[source] = counts
    return files


def file_lines(segments):
    """LLVM LineCoverageStats applied to the already-exported file segments."""
    by_line = collections.defaultdict(list)
    for segment in segments:
        by_line[segment[0]].append(segment)
    counts, wrapped = {}, None
    for line in range(1, max(by_line, default=0)+1):
        current = by_line[line]
        starts = [s for s in current if s[3] and s[4] and not s[5]]
        skipped = bool(current and not current[0][3] and current[0][4])
        mapped = ((not skipped and (bool(wrapped and wrapped[3]) or bool(starts)))
                  or any(s[3] and s[4] for s in current))
        if mapped:
            counts[line] = max([wrapped[2] if wrapped else 0]+
                               [s[2] for s in starts])
        if current:
            wrapped = current[-1]
    return counts


def reconcile(detail, lcov, source_root):
    require(len(detail['data']) == 1, 'requires one audited export dataset')
    data = detail['data'][0]
    files = {f['filename']: f for f in data['files']}
    require(set(files) == set(lcov), 'JSON/LCOV source inventory differs')
    require(all(not f['expansions'] for f in files.values()), 'unsupported expansions')
    groups = collections.defaultdict(list)
    for function in data['functions']:
        if function['filenames'][0] not in files:
            continue  # Existing exported file selection; not a new exclusion.
        counts = function_lines(function)
        first = function['regions'][0]
        groups[(function['filenames'][0], first[0], first[1])].append(
            (counts, function['name']))
    by_file = collections.defaultdict(list)
    for (source, line, column), instances in groups.items():
        by_file[source].append((line, column, instances))
    reports, group_witnesses, address_witnesses = [], [], []
    for source, metadata in files.items():
        display = source.removeprefix(str(source_root).rstrip('/')+'/')
        require(file_lines(metadata['segments']) == lcov[source],
                'JSON/LCOV exact file counters differ: '+display)
        mapped = covered = 0
        union = {}
        owners = collections.defaultdict(list)
        group_union_gap = 0
        for line, column, instances in by_file[source]:
            total = max(len(counts) for counts, _ in instances)
            hits = max(sum(c > 0 for c in counts.values()) for counts, _ in instances)
            mapped += total
            covered += hits
            merged = {}
            for counts, name in instances:
                for address, count in counts.items():
                    merged[address] = max(merged.get(address, 0), count)
            zeros = sum(c == 0 for c in merged.values())
            group_union_gap += zeros
            maximum_delta = total-hits-zeros
            if maximum_delta:
                group_witnesses.append({'file': display, 'definition': [line, column],
                    'maximum_gap': total-hits, 'union_gap': zeros,
                    'delta': maximum_delta, 'instances': [
                        {'name': name, 'mapped_lines': len(counts),
                         'covered_lines': sum(c > 0 for c in counts.values()),
                         'zero_lines': [n for n, c in counts.items() if not c]}
                        for counts, name in instances]})
            for address, count in merged.items():
                union[address] = max(union.get(address, 0), count)
                owners[address].append({'definition': [line, column], 'count': count,
                                        'instances': [name for _, name in instances]})
        summary = metadata['summary']['lines']
        require([mapped, covered] == [summary['count'], summary['covered']],
                'function reconstruction differs from LLVM summary: '+display)
        require(len(by_file[source]) == metadata['summary']['functions']['count'],
                'instantiation grouping differs: '+display)
        require(set(union) == set(lcov[source]), 'address inventory differs: '+display)
        union_zero = sum(c == 0 for c in union.values())
        lcov_zero = sum(c == 0 for c in lcov[source].values())
        row = {'file': display, 'summary_gap': mapped-covered, 'lcov_zero': lcov_zero,
               'maximum_vs_union': mapped-covered-group_union_gap,
               'overlapping_group_gaps': group_union_gap-union_zero,
               'file_counter_shadow': union_zero-lcov_zero}
        row['difference'] = row['summary_gap']-row['lcov_zero']
        require(row['difference'] == row['maximum_vs_union']+
                row['overlapping_group_gaps']+row['file_counter_shadow'],
                'unreconciled file accounting')
        reports.append(row)
        for address, own in owners.items():
            zeros = sum(o['count'] == 0 for o in own)
            shadow = (union[address] > 0) != (lcov[source][address] > 0)
            if zeros > 1 or (zeros and union[address] > 0) or shadow:
                address_witnesses.append({'file': display, 'line': address,
                    'file_count': lcov[source][address], 'function_union_count': union[address],
                    'zero_group_count': zeros, 'owners': own})
    totals = {key: sum(r[key] for r in reports) for key in reports[0] if key != 'file'}
    require(totals['summary_gap'] == data['totals']['lines']['count']-
            data['totals']['lines']['covered'], 'dataset summary differs')
    return {'schema': 1, 'scope_changed': False, 'reachability_claim': False,
            'files': len(files), 'function_groups': len(groups), 'totals': totals,
            'per_file': reports, 'instantiation_witnesses': group_witnesses,
            'address_witnesses': address_witnesses}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--detailed', type=Path, required=True)
    parser.add_argument('--lcov', type=Path, required=True)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    require(not args.output.exists(), 'evidence output already exists')
    raw = args.detailed.read_bytes()
    result = reconcile(json.loads(raw), read_lcov(args.lcov), args.source_root)
    result['input_sha256'] = {'detailed': hashlib.sha256(raw).hexdigest(),
                             'lcov': hashlib.sha256(args.lcov.read_bytes()).hexdigest()}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as output:
        json.dump(result, output, indent=2)
        output.write('\n')
    print(json.dumps({k: v for k, v in result.items() if k not in
                      ('per_file', 'instantiation_witnesses', 'address_witnesses')}))


if __name__ == '__main__':
    main()
