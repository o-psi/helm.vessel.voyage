#!/usr/bin/env python3
"""Opt-in forwarding rustc recorder; isolated evidence files, no secrets/env capture.

Use outside RUSTC_WORKSPACE_WRAPPER so checkout namespace remains unchanged.
This records raw compiler evidence, not inferred Cargo fingerprint bindings.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid


def run(argv, directory):
    require = lambda ok, msg: None if ok else (_ for _ in ()).throw(ValueError(msg))
    require(argv, 'missing compiler command')
    directory = Path(directory).resolve()
    require(directory.is_dir(), 'evidence directory must already exist')
    identity = str(uuid.uuid4())
    record = directory / (identity + '.json')
    source = [a for a in argv[1:] if a.endswith('.rs') and Path(a).is_file()]
    started = {'schema': 1, 'cwd': os.getcwd(), 'argv': argv,
               'source_inputs': {str(Path(p).resolve()): hashlib.sha256(Path(p).read_bytes()).hexdigest()
                                 for p in source}}
    # Intent written before invoking compiler; interrupted compiler stays pending.
    with record.open('x') as output:
        json.dump({'phase': 'pending', **started}, output, sort_keys=True)
    result = subprocess.run(argv, check=False)
    completion = directory / (identity + '.completed.json')
    with completion.open('x') as output:
        json.dump({'phase': 'completed', 'exit_status': result.returncode, **started}, output, sort_keys=True)
    return result.returncode


if __name__ == '__main__':
    directory = os.environ.get('VOYAGE_COVERAGE_COMPILER_EVIDENCE')
    if not directory:
        raise SystemExit('VOYAGE_COVERAGE_COMPILER_EVIDENCE is required')
    raise SystemExit(run(sys.argv[1:], directory))
