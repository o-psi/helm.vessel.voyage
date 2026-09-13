"""Offline Linux owner-death recovery; no operator attestation or inference replay."""
import argparse
import json
import os
from pathlib import Path
import signal
import time
import uuid

from provider_attempts import Fixture, Handler, history, session, wait_for


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    fixture = Fixture(args.bin_dir.resolve())
    fixture.provider.RequestHandlerClass = Handler
    fixture.provider.errors = []
    fixture.provider.times = []
    fixture.provider.scenario = ('responses', 'cancel')  # 503 with a two-second backoff.
    try:
        fixture.start()
        sid = session(fixture, 'responses', 'cancel')
        receipt = fixture.command(sid, fixture.submit(sid, 'Remember lighthouse during owner recovery.'))
        def pending():
            snapshot = fixture.command(sid, {'op': 'snapshot'})
            rows = snapshot['run']['provider_attempts']
            return snapshot if rows and rows[-1]['decision'] == 'retry_scheduled' else None
        before = wait_for(pending)
        incarnation = fixture.request({'op': 'inspect', 'session_id': sid})['incarnation']
        owned = fixture.owned_processes()
        assert len(owned) == 1, owned
        pid, argv = next(iter(owned.items()))
        assert Path(os.fsdecode(argv[3])) == fixture.directory / 'sessions' / sid
        descriptor = os.pidfd_open(pid)
        try:
            assert fixture.owned_processes().get(pid) == argv
            signal.pidfd_send_signal(descriptor, signal.SIGKILL)
        finally:
            os.close(descriptor)
        wait_for(lambda: pid not in fixture.owned_processes())
        # Use the existing exclusive recovery command, with no fabricated cleanup
        # observations, resource attestations or tool reconciliation decisions.
        recovered = fixture.request({'op': 'recover', 'command_id': str(uuid.uuid4()),
            'session_id': sid, 'incarnation': incarnation, 'acknowledge_cleanup': None,
            'reconcile_tools': None, 'expected_revision': None, 'acknowledge_resources': []})
        assert recovered['restart_permitted'] is True, recovered
        assert recovered['cleanup_disposition'] in ('observed', 'unresolved_retained'), recovered
        restarted = fixture.request({'op': 'restart', 'command_id': str(uuid.uuid4()),
            'session_id': sid, 'incarnation': incarnation})
        assert restarted['incarnation'] != incarnation, restarted
        # Outlast the original scheduled delay: the replacement owner must not
        # consume its saved scheduling observation as permission to dispatch.
        time.sleep(2.2)
        rows = history(fixture, sid, receipt['run_id'])
        assert len(rows) == 1 and rows[0]['attempt']['decision'] == 'recovery_interrupted', rows
        assert rows[0]['attempt']['retry']['eligible'] is False
        assert len(fixture.provider.bodies) == 1
        snapshot = fixture.command(sid, {'op': 'snapshot'})
        assert snapshot['messages'] == before['messages']
        fixture.record('owner-death-no-inference-replay', {'recovery': recovered, 'attempts': rows})
        fixture.provider.scenario = ('responses', 'success')
        next_receipt = fixture.command(sid, fixture.submit(sid, 'Continue the retained lighthouse task.'))
        assert next_receipt['run_id'] != receipt['run_id']
        final = fixture.finished(sid)
        assert final['run']['state'] == 'completed', final['run']
        assert len(fixture.provider.bodies) == 2
        assert 'lighthouse' in json.dumps(fixture.provider.bodies[-1])
        fixture.suspended(sid)
        assert not fixture.provider.errors, fixture.provider.errors
        fixture.record('explicit-turn-after-owner-recovery', {'run': final['run'], 'requests': 2})
    finally:
        fixture.close()


if __name__ == '__main__':
    main()
