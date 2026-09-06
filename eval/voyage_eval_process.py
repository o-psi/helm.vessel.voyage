"""POSIX bounded output collection; no escaped-descendant cleanup guarantee."""
from __future__ import annotations
import os
import selectors
import signal
import subprocess
import time

MAX_OUTPUT = 1024 * 1024  # per stream, retained and counted in bytes


def execute(command, cwd, env, timeout):
    if os.name != 'posix':
        raise ValueError('bounded evaluation execution requires POSIX')
    process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               start_new_session=True)
    outputs = {'stdout': bytearray(), 'stderr': bytearray()}
    deadline = time.monotonic() + timeout
    timed_out = limited = False
    cleanup_error = None
    try:
        with selectors.DefaultSelector() as selector:
            for name in outputs:
                stream = getattr(process, name)
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, name)
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                for key, _ in selector.select(min(remaining, .1)):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    target = outputs[key.data]
                    room = MAX_OUTPUT - len(target)
                    target.extend(chunk[:room])
                    if len(chunk) > room:
                        limited = True
                        break
                if limited:
                    break
            if not timed_out and not limited:
                try:
                    process.wait(timeout=max(.001, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    timed_out = True
    finally:
        if timed_out or limited or process.poll() is None:
            # Signal only this invocation's process group. An escaped descendant
            # is outside this proof; return failure without claiming all effects stopped.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            cleanup_error = 'direct Helm process was not observed reaped'
        process.stdout.close()
        process.stderr.close()
    return {'stdout': outputs['stdout'].decode('utf-8', errors='replace'),
            'stderr': outputs['stderr'].decode('utf-8', errors='replace'),
            'exit_code': None if timed_out else process.returncode,
            'timed_out': timed_out, 'output_limited': limited,
            'cleanup_error': cleanup_error, 'direct_process_reaped': process.returncode is not None}
