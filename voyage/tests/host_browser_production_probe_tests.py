"""Offline ledger initializer contracts; mocked process data is not native proof."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
import uuid

spec = importlib.util.spec_from_file_location('qualification_probe', Path(__file__).with_name('host_browser_production_probe.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class LedgerContracts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='owned-ledger-contract-')
        self.root = Path(self.temp.name)
        self.capacity = self.root/'capacity'
        self.capacity.mkdir(mode=0o700)
        self.home = self.root/'browser'
        self.home.mkdir(mode=0o700)
        self.session, self.browser, self.incarnation = (str(uuid.uuid4()) for _ in range(3))
        self.lock = self.home/'worker.lock'
        self.lock.write_text(json.dumps({'pid': 201, 'browser': self.browser}))
        self.lock.chmod(0o600)
        slot = self.capacity/'browser-slot-fixture'
        slot.write_text(self.session)
        slot.chmod(0o600)
        programs = {name: {'path': '/fixture/'+name, 'sha256': 'a'*64} for name in ('helm','node','worker','guardian','python','voyage')}
        programs['worker']['path'] = '/fixture/browser/worker.mjs'
        programs['guardian']['path'] = '/fixture/browser/guardian.py'
        self.request = {'schema': 1, 'capacity': str(self.capacity),
                        'sessions': [{'label': 'a', 'session_id': self.session, 'incarnation': self.incarnation, 'browser_id': self.browser, 'root': str(self.home)}],
                        'clients': [{'label': 'native-a', 'pid': 101, 'start_ticks': 1101, 'descendants': True}, {'label': 'native-b', 'pid': 102, 'start_ticks': 1102, 'descendants': True}], 'programs': programs}
        self.processes = {101: {'pid':101,'start_ticks':1101,'ppid':50,'state':'S'}, 102: {'pid':102,'start_ticks':1102,'ppid':50,'state':'S'},
                          201: {'pid':201,'start_ticks':1201,'ppid':202,'state':'S'}, 202: {'pid':202,'start_ticks':1200,'ppid':203,'state':'S'}, 203: {'pid':203,'start_ticks':1199,'ppid':17,'state':'S'}}
        self.words = {201:['/fixture/node','/fixture/browser/worker.mjs'],202:['/fixture/python','/fixture/browser/guardian.py','/fixture/node','/fixture/browser/worker.mjs','/tmp/vhb-fixture']}
        self.proof = {'scratch':'/tmp/vhb-fixture','slots':['browser-slot-fixture'], 'processes':[self.processes[202],self.processes[201]]}
        self.actual_stat = Path.stat
        def stat_path(path, *args, **kwargs):
            if str(path).startswith('/proc/'):
                return SimpleNamespace(st_uid=os.getuid())
            return self.actual_stat(path, *args, **kwargs)
        self.patches = [patch.object(probe,'identity',side_effect=lambda pid: copy.deepcopy(self.processes.get(pid))),
                        patch.object(probe.Path,'stat',stat_path),
                        patch.object(probe,'program',side_effect=lambda value,name:{'path':value['path'],'inode':(1,2,3,4,5)}),
                        patch.object(probe,'executable'), patch.object(probe,'words',side_effect=lambda seen:list(self.words[seen['pid']])),
                        patch.object(probe,'before',side_effect=lambda *_:copy.deepcopy(self.proof))]
        self.mocks = [p.start() for p in self.patches]

    def tearDown(self):
        for p in reversed(self.patches):
            p.stop()
        self.temp.cleanup()

    def test_current_local_binding_derives_voyage_leaf_and_guardian_tree_without_overlap(self):
        value = probe.initialize_ledger(self.request)
        self.assertEqual(value['ledger'], {'schema':1,'roots':[{'label':'voyage-a','pid':203,'start_ticks':1199,'descendants':False},{'label':'browser-a','pid':202,'start_ticks':1200,'descendants':True}]})
        self.assertEqual(value['clients'],self.request['clients'])
        self.assertEqual(value['bindings'][0]['browser_id'],self.browser)
        self.assertEqual(value['bindings'][0]['incarnation'],self.incarnation)
        self.assertEqual(len(self.mocks[3].call_args_list),5)

    def test_stopped_suspended_reused_native_and_missing_worker_refuse(self):
        for state in ('Z','T','t','X'):
            self.processes[101]['state']=state
            with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
        self.processes[101]['state']='S'
        self.processes[101]['start_ticks']+=1
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
        self.processes[101]['start_ticks']-=1
        self.processes.pop(201)
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)

    def test_unknown_schema_foreign_client_label_and_wrong_browser_lock_refuse(self):
        for mutate in (lambda r:r.update(extra='unknown'),lambda r:r.update(schema=True),lambda r:r['clients'][0].update(label='native-foreign'),lambda r:r['sessions'][0].update(browser_id=str(uuid.uuid4()))):
            request=copy.deepcopy(self.request);mutate(request)
            with self.assertRaises(AssertionError): probe.initialize_ledger(request)

    def test_unknown_or_pending_cleanup_marker_including_dangling_symlink_refuses(self):
        for name in ('guardian-cleanup.json','.guardian-cleanup.json.new'):
            marker=self.home/name;marker.write_text('{}')
            with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
            marker.unlink();marker.symlink_to(self.home/'absent')
            with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
            marker.unlink()

    def test_replaced_lock_and_worker_parent_or_program_mismatch_refuse(self):
        def replace(*_):
            self.lock.rename(self.home/'old.lock')
            self.lock.write_text(json.dumps({'pid':201,'browser':self.browser}));self.lock.chmod(0o600)
            return copy.deepcopy(self.proof)
        self.mocks[-1].side_effect=replace
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
        self.mocks[-1].side_effect=lambda *_:copy.deepcopy(self.proof)
        self.processes[201]['ppid']=999
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
        self.processes[201]['ppid']=202
        self.mocks[3].side_effect=AssertionError('fixed program mismatch')
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)

    def test_argument_shape_and_duplicate_cross_fixture_identity_refuse(self):
        self.words[201].append('not-approved')
        with self.assertRaises(AssertionError): probe.initialize_ledger(self.request)
        self.words[201].pop()
        other={**self.request['sessions'][0],'label':'b','session_id':str(uuid.uuid4())}
        request={**self.request,'sessions':[self.request['sessions'][0],other]}
        with self.assertRaises(AssertionError): probe.initialize_ledger(request)

    def test_foreign_uid_rejected_before_private_state_or_program_read(self):
        with patch.object(probe.Path,'stat',return_value=SimpleNamespace(st_uid=os.getuid()+1)):
            with self.assertRaises(AssertionError): probe.live(101,1101)


class ProgramContracts(unittest.TestCase):
    def test_approved_file_hash_and_no_follow_permissions_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix='owned-program-contract-') as folder:
            path=Path(folder)/'node';path.write_bytes(b'\x7fELFbounded fixture only');path.chmod(0o700)
            spec={'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
            value=probe.program(spec,'node');self.assertEqual(value['path'],str(path))
            for bad in ({**spec,'sha256':'0'*64},{**spec,'extra':'unknown'}):
                with self.assertRaises(AssertionError): probe.program(bad,'node')
            path.chmod(0o777)
            with self.assertRaises(AssertionError): probe.program(spec,'node')
            path.chmod(0o700);alias=Path(folder)/'alias';alias.symlink_to(path)
            with self.assertRaises(AssertionError): probe.program({**spec,'path':str(alias)},'node')
            path.unlink();os.mkfifo(path,0o600)
            with self.assertRaises(AssertionError): probe.program(spec,'node')


class IdleMetadataContracts(unittest.TestCase):
    def test_exact_running_zero_viewer_stop_ack_only(self):
        value={'schema':1,'pid':321,'browser':str(uuid.uuid4()),'sequence':4,'observed_at_ms':1000,
               'browser_running':True,'zero_viewers':True,'recorder_stop_observed':True,'source_sha256':'a'*64}
        probe.validate_idle_metadata(value,321,value['browser'],'a'*64)
        for field in ['browser_running','zero_viewers','recorder_stop_observed']:
            changed=copy.deepcopy(value);changed[field]=False
            with self.assertRaises(AssertionError):probe.validate_idle_metadata(changed,321,value['browser'],'a'*64)
        for field in ['schema','pid','sequence','observed_at_ms']:
            changed=copy.deepcopy(value);changed[field]=True
            with self.assertRaises(AssertionError):probe.validate_idle_metadata(changed,321,value['browser'],'a'*64)
        changed=copy.deepcopy(value);changed['page']='private topology'
        with self.assertRaises(AssertionError):probe.validate_idle_metadata(changed,321,value['browser'],'a'*64)
        with self.assertRaises(AssertionError):probe.validate_idle_metadata(value,322,value['browser'],'a'*64)
        with self.assertRaises(AssertionError):probe.validate_idle_metadata(value,321,str(uuid.uuid4()),'a'*64)


if __name__=='__main__':
    unittest.main()
