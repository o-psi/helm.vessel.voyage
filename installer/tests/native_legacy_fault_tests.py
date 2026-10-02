"""Offline admission/fault-witness contracts; no native upgrade or manager call."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import signal
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import native_legacy_faults as f
spec = importlib.util.spec_from_file_location('native_fault_qualification', Path(__file__).with_name('native_legacy_qualification.py'))
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)


class AdmissionContracts(unittest.TestCase):
    def test_full_embedded_helper_source_matches_reviewed_rust_input(self):
        program = (Path(__file__).resolve().parents[1] / 'src/legacy_update.py').read_bytes()
        self.assertEqual(hashlib.sha256(program).hexdigest(), f.LEGACY_PROGRAM_SHA256)
        actual = [b'/usr/bin/python3', b'-I', b'-c', program, b'snapshot', b'/fixed/state', b'/fixed/accounts', b'/fixed/stage']
        self.assertTrue(f.qualified_helper_argv(actual, 'snapshot', '/fixed/state', '/fixed/accounts', '/fixed/stage'))
        for index, replacement in [(0,b'/tmp/python3'),(1,b'-E'),(3,program+b'\n'),(4,b'verify'),(5,b'/other/state'),(6,b'/other/accounts'),(7,b'/other/stage')]:
            with self.subTest(index=index):
                changed = actual.copy(); changed[index] = replacement
                self.assertFalse(f.qualified_helper_argv(changed, 'snapshot', '/fixed/state', '/fixed/accounts', '/fixed/stage'))
        self.assertFalse(f.qualified_helper_argv(actual+[b'extra'], 'snapshot', '/fixed/state', '/fixed/accounts', '/fixed/stage'))

    def test_noncanonical_uuid_and_nil_are_refused(self):
        valid = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
        self.assertTrue(f.canonical_uuid(valid))
        for invalid in [valid.upper(), '0'*32, '00000000-0000-0000-0000-000000000000', '../'+valid, None, 42]:
            self.assertFalse(f.canonical_uuid(invalid))

    def test_exact_role_argv_and_direct_parent_cannot_be_substituted(self):
        expected = [b'/fixed/installer',b'--bin-dir',b'/fixed/bin',b'upgrade',b'--no-start']
        for actual in [[b'/bin/sh',b'/fixed/install.sh',b'upgrade'],[b'/fixed/installer',b'remote-update',b'worker',b'apply',b'op'],expected+[b'--start']]:
            witness={'descriptor':41,'pid':99}
            with patch.object(q,'exact_candidate',return_value=('verified',witness)),patch.object(f,'argv',return_value=actual),patch.object(q.os,'close') as close,patch.object(q.signal,'pidfd_send_signal') as signal:
                self.assertEqual(f.process_witness(q,99,Path('/fixed/installer'),'a'*64,expected),('not_candidate',None))
                close.assert_called_once_with(41);signal.assert_not_called()
        witness={'descriptor':41,'pid':99}
        with patch.object(q,'exact_candidate',return_value=('verified',witness)),patch.object(f,'argv',return_value=expected),patch.object(f,'parent',return_value=777),patch.object(q.os,'close') as close:
            self.assertEqual(f.process_witness(q,99,Path('/fixed/installer'),'a'*64,expected,{'pid':10,'start_ticks':123}),('not_candidate',None));close.assert_called_once_with(41)

    def test_retain_exact_pidfd_and_no_signal_during_admission(self):
        expected=[b'/fixed/installer',b'upgrade'];witness={'descriptor':41,'pid':99}
        with patch.object(q,'exact_candidate',return_value=('verified',witness)),patch.object(f,'argv',return_value=expected),patch.object(q,'still_candidate',return_value=True),patch.object(q.os,'close') as close,patch.object(q.signal,'pidfd_send_signal') as signal:
            category, retained=f.process_witness(q,99,Path('/fixed/installer'),'a'*64,expected)
            self.assertEqual(category,'verified');self.assertIs(retained,witness);close.assert_not_called();signal.assert_not_called()
            self.assertEqual(retained['argv_sha256'],hashlib.sha256(b'\0'.join(expected)).hexdigest())

    def test_changed_start_or_permission_never_admits_and_closes_fd(self):
        expected=[b'/fixed/installer',b'upgrade']
        for effect in [False,PermissionError(13,'synthetic')]:
            with patch.object(q,'exact_candidate',return_value=('verified',{'descriptor':41,'pid':99})),patch.object(f,'argv',return_value=expected),patch.object(q,'still_candidate',side_effect=effect if isinstance(effect,Exception) else None,return_value=False),patch.object(q.os,'close') as close:
                category,witness=f.process_witness(q,99,Path('/fixed/installer'),'a'*64,expected)
                self.assertIn(category,['missing_pending','permission_pending']);self.assertIsNone(witness);close.assert_called_once_with(41)

    def test_record_identity_changes_do_not_replay(self):
        with tempfile.TemporaryDirectory() as root:
            path=Path(root)/'record.json';path.write_text(json.dumps({'operation_id':'fixed','channel':'local-owner','phase':'applying'}))
            pin={'operation_id':'fixed','channel':'local-owner'}
            self.assertEqual(f.stable_record(q,path,pin)['phase'],'applying')
            path.write_text(json.dumps({'operation_id':'foreign','channel':'local-owner','phase':'applying'}))
            with self.assertRaisesRegex(RuntimeError,'identity changed'):f.stable_record(q,path,pin)

    def test_proof_namespace_and_restored_marker_must_match(self):
        record={'legacy_accounts':'/fixed/accounts','staging_root':'/fixed/stage','legacy_proof':{'state':str(q.STATE),'accounts':'/fixed/accounts','stage':'/foreign/stage'}}
        with self.assertRaisesRegex(RuntimeError,'namespace changed'):f.boundary_seen(q,'restored',record)
        self.assertFalse(f.boundary_seen(q,'snapshot',{'legacy_proof':None}))
        self.assertTrue(f.boundary_seen(q,'committing',{'phase':'committing','legacy_proof':None}))

    def test_one_fresh_actual_role_signal_is_observed_and_uncertain_signal_is_never_retried(self):
        for scenario in ['normal','uncertain','terminal-cleanup','terminal-removed','changed-before-signal']:
            uncertain=scenario=='uncertain';changed_before=scenario=='changed-before-signal'
            with self.subTest(scenario=scenario),tempfile.TemporaryDirectory() as directory:
                home=Path(directory);work=home/'q401';work.mkdir();install=home/'install';(install/'updates').mkdir(parents=True)
                state=home/'state';state.mkdir();accounts=home/'accounts';accounts.mkdir()
                op='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';stage=home/'.cache/voyage/upgrades'/('prepare-local-'+op)
                (stage/'candidate/bin').mkdir(parents=True);(work/'candidate/release/bin').mkdir(parents=True)
                image=work/'candidate/release/bin/voyage-installer';image.write_bytes(b'qualified synthetic image')
                sha=q.digest(image);manifest={'binaries':{'voyage-installer':{'sha256':sha}}}
                (work/'candidate/release/release.json').write_text(json.dumps(manifest));(stage/'candidate/release.json').write_text(json.dumps(manifest))
                (work/'candidate-bin.json').write_text(json.dumps(str(image)))
                record={'operation_id':op,'channel':'local-owner','created_at':int(f.time.time()),'current_release':'a'*64,'release_id':'b'*64,'legacy_mode':True,'staging_root':str(stage),'bin_dir':str(stage/'candidate/bin'),'legacy_accounts':str(accounts),'supervisor_activation':{'state':str(state),'active':True},'legacy_proof':{'state':str(state),'accounts':str(accounts),'stage':str(stage)}}
                written={};signalled=[]
                def retain(name,value):
                    written[name]=value
                    if changed_before and name.endswith('-pre-signal.json'):
                        changed={**record,'staging_root':None}
                        (install/'updates'/(op+'.json')).write_text(json.dumps(changed))
                shim=SimpleNamespace(HOME=home,WORK=work,INSTALL=install,STATE=state,require=q.require,digest=q.digest,ProcessNotLive=q.ProcessNotLive,
                    process_start=lambda _pid:123,write=retain,
                    exact_candidate=lambda *_args:('verified',{'descriptor':41,'pid':99,'start_ticks':123,'uid':1000,'image_pin':(1,2)}),
                    still_candidate=lambda *_args:True,pidfd_exited=lambda _fd:bool(signalled),retired=lambda _w:bool(signalled))
                def fresh_record(_delay):
                    path=install/'updates'/(op+'.json')
                    if not path.exists():path.write_text(json.dumps(record))
                def one_signal(*_args):
                    signalled.append('one')
                    if uncertain:raise PermissionError(13,'synthetic uncertain signal')
                    if scenario=='terminal-cleanup':
                        cleaned={**record,'phase':'complete','bin_dir':None,'staging_root':None,'supervisor_activation':None}
                        (install/'updates'/(op+'.json')).write_text(json.dumps(cleaned))
                    if scenario=='terminal-removed':(install/'updates'/(op+'.json')).unlink()
                expected=[os.fsencode(image),b'--bin-dir',os.fsencode(image.parent),b'upgrade',b'--no-start']
                with patch.object(f.time,'sleep',side_effect=fresh_record),patch.object(f,'argv',return_value=expected),patch.object(f.signal,'pidfd_send_signal',side_effect=one_signal),patch.object(f.os,'close') as close:
                    if uncertain:
                        with self.assertRaisesRegex(RuntimeError,'signal unconfirmed'):f.run(shim,SimpleNamespace(boundary='snapshot',installer_pid=99))
                    elif changed_before:
                        with self.assertRaisesRegex(RuntimeError,'identity changed'):f.run(shim,SimpleNamespace(boundary='snapshot',installer_pid=99))
                    else:f.run(shim,SimpleNamespace(boundary='snapshot',installer_pid=99))
                    close.assert_called_once_with(41)
                self.assertEqual(signalled,[] if changed_before else ['one']);self.assertIn('fault-snapshot-pre-signal.json',written)
                result=written['fault-snapshot-result.json']
                self.assertEqual(result['status'],'unqualified' if uncertain or changed_before else 'fault_target_retired')
                self.assertEqual(result['signal_delivered'],not (uncertain or changed_before));self.assertFalse(result['rollback_or_cleanup_acceptance'])
                self.assertEqual(result['identities']['updater']['exit_and_reaping_observed'],not changed_before)
                self.assertEqual(written['fault-snapshot-owner.json']['record_pin']['staging_root'],str(stage))
                self.assertFalse(result['independent_remote_worker'])

    def test_multi_candidate_or_helper_scan_exception_closes_every_owned_pidfd(self):
        for items in [[{'descriptor':41},{'descriptor':42}], [({'descriptor':41},[b'full argv']),({'descriptor':42},[b'other argv'])]]:
            with self.subTest(items=items),patch.object(f.os,'close') as close,patch.object(f.signal,'pidfd_send_signal') as signal:
                with self.assertRaises(PermissionError):
                    with f.witness_scan() as accepted:
                        accepted.extend(items)
                        raise PermissionError(13,'synthetic later candidate/child denial')
                self.assertEqual([call.args[0] for call in close.call_args_list],[41,42]);signal.assert_not_called()

    def test_scan_attempts_all_closes_even_if_first_close_raises(self):
        with patch.object(f.os,'close',side_effect=[OSError('synthetic close failure'),None]) as close:
            with self.assertRaises(OSError):
                with f.witness_scan() as accepted:accepted.extend([{'descriptor':41},{'descriptor':42}])
            self.assertEqual([call.args[0] for call in close.call_args_list],[41,42])

    def test_actual_owned_child_full_argv_parent_pidfd_exit_and_reaping(self):
        self.assertEqual(os.getuid(),1000,'ordinary fixture UID required')
        image=Path(sys.executable).resolve(strict=True)
        arguments=[str(image),'-I','-c','import time;time.sleep(30)']
        child=subprocess.Popen(arguments,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        witness=None
        try:
            owner={'pid':os.getpid(),'start_ticks':q.process_start(os.getpid())}
            category,witness=f.process_witness(q,child.pid,image,q.digest(image),[os.fsencode(a) for a in arguments],owner)
            self.assertEqual(category,'verified');self.assertIsNotNone(witness)
            self.assertEqual(witness['parent_pid'],os.getpid());self.assertEqual(witness['parent_start_ticks'],owner['start_ticks'])
            self.assertTrue(f.exact_again(q,witness,image,[os.fsencode(a) for a in arguments],owner))
            self.assertFalse(q.pidfd_exited(witness['descriptor']));self.assertFalse(q.retired(witness))
            signal.pidfd_send_signal(witness['descriptor'],signal.SIGKILL)
            self.assertEqual(child.wait(timeout=3),-signal.SIGKILL)
            self.assertTrue(q.pidfd_exited(witness['descriptor']));self.assertTrue(q.retired(witness))
        finally:
            if witness is not None:os.close(witness['descriptor'])
            if child.poll() is None:child.kill();child.wait(timeout=3)

    def test_expired_unobserved_window_retains_unqualified_result_without_signal(self):
        with tempfile.TemporaryDirectory() as root:
            private=Path(root);(private/'updates').mkdir();written={}
            shim=SimpleNamespace(INSTALL=private,WORK=private,process_start=lambda _pid:123,
                                 write=lambda name,value:written.__setitem__(name,value),digest=lambda _p:'a'*64)
            with patch.object(f.time,'monotonic',side_effect=[0,1231]),patch.object(f.signal,'pidfd_send_signal') as signal:
                with self.assertRaisesRegex(RuntimeError,'1230 seconds'):f.run(shim,SimpleNamespace(boundary='snapshot',installer_pid=None))
                signal.assert_not_called()
            result=written['fault-snapshot-result.json']
            self.assertEqual(result['status'],'unqualified');self.assertFalse(result['signal_delivered']);self.assertFalse(result['rollback_or_cleanup_acceptance']);self.assertEqual(result['deadline_seconds'],1230)
            self.assertFalse(result['independent_remote_worker'])


if __name__ == '__main__':unittest.main()
