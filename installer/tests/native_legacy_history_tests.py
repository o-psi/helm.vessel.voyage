"""Pure historical projection contracts; no process, provider or Native effect."""
import copy
import importlib.util
from pathlib import Path
import unittest
import uuid
import hashlib
import os
import json
import sys
import tempfile
import types
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('native_legacy_history',Path(__file__).with_name('native_legacy_history.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
SESSION='11111111-1111-4111-8111-111111111111'
INTERRUPTED='22222222-2222-4222-8222-222222222222'
NULLS={'interrupted_attempt':None,'coordination':None,'tool_output':None,'created_at':None,
       'operator_name':None,'tool_call_id':None,'tool_outcome':None,'tool_success':None,'steering':None}


def pair():
    canonical=[{'role':'user','content':'Exact unsent Δ\nline 2'},
        {'role':'assistant','content':'Exact canonical answer α','created_at':'2026-10-02T06:00:00Z',
         'operator_name':'fixture author','interrupted_attempt':INTERRUPTED,
         'coordination':{'source_session':SESSION,'message_id':'source-1'},
         'tool_calls':[{'id':'call-α','name':'fixture tool','arguments':{'value':'Δ'}}],
         'tool_call_id':'call-α','tool_success':True,'steering':{'id':SESSION,'status':'applied'},
         'provider_state':{'opaque':'synthetic-private-replay-state'}}]
    # Explicit source-derived expected payload, not calling the implementation
    # to manufacture its oracle. Every public field and bounded-message marker
    # occurs, even when canonical serde omitted an empty or optional value.
    public=[{**NULLS,'role':'user','content':'Exact unsent Δ\nline 2','parts':[],'tool_calls':[],
             'message_index':0,'projection_truncated':False},
        {**NULLS,'role':'assistant','content':'Exact canonical answer α','created_at':'2026-10-02T06:00:00Z',
         'operator_name':'fixture author','interrupted_attempt':INTERRUPTED,
         'coordination':{'source_session':SESSION,'message_id':'source-1'},'parts':[],
         'tool_calls':[{'id':'call-α','name':'fixture tool','arguments':{'value':'Δ'}}],
         'tool_call_id':'call-α','tool_success':True,'steering':{'id':SESSION,'status':'applied'},
         'message_index':1,'projection_truncated':False}]
    return canonical,{'session_id':SESSION,'projection':'public-v1','observation':'snapshot','revision':17,'total_messages':2,
                      'message_offset':0,'history_truncated':False,'messages':public}


class Contracts(unittest.TestCase):
    def test_exact_public_v1_has_full_text_and_identity_despite_different_raw_json_shape(self):
        before,snapshot=pair();self.assertNotEqual(before,snapshot['messages'])
        witness=m.verify_snapshot(snapshot,SESSION,before)
        self.assertTrue(witness['full_text_order_identity_verified']);self.assertEqual(witness['messages'],2)
        self.assertEqual(before[1]['provider_state'],{'opaque':'synthetic-private-replay-state'})
        self.assertNotIn('provider_state',snapshot['messages'][1]);self.assertEqual(len(snapshot['messages'][1]),15)

    def test_same_count_changed_text_role_order_or_index_is_refused(self):
        before,snapshot=pair()
        for field,value in [('content','different'),('role','tool'),('message_index',0)]:
            changed=copy.deepcopy(snapshot);changed['messages'][1][field]=value
            with self.subTest(field=field),self.assertRaisesRegex(RuntimeError,'text/order/identity'):m.verify_snapshot(changed,SESSION,before)
        changed=copy.deepcopy(snapshot);changed['messages'].reverse()
        for i,value in enumerate(changed['messages']):value['message_index']=i
        with self.assertRaisesRegex(RuntimeError,'text/order/identity'):m.verify_snapshot(changed,SESSION,before)

    def test_exact_all_public_metadata_and_tool_ids_are_preserved(self):
        before,snapshot=pair()
        changes={'created_at':'2026-10-02T06:00:01Z','operator_name':'other author',
            'interrupted_attempt':str(uuid.uuid4()),'coordination':{'source_session':SESSION,'message_id':'other'},
            'tool_calls':[{'id':'other','name':'fixture tool','arguments':{'value':'Δ'}}],
            'tool_call_id':'other','tool_success':False,'tool_outcome':{'incomplete':'different'},
            'tool_output':{'other':'output'},'parts':[{'type':'text','text':'different'}],
            'steering':{'id':str(uuid.uuid4()),'status':'applied'}}
        for field,value in changes.items():
            changed=copy.deepcopy(snapshot);changed['messages'][1][field]=value
            with self.subTest(field=field),self.assertRaisesRegex(RuntimeError,'text/order/identity'):m.verify_snapshot(changed,SESSION,before)

    def test_partial_ranges_truncation_and_foreign_session_or_projection_refuse(self):
        before,snapshot=pair()
        for field,value in [('session_id',str(uuid.uuid4())),('projection','other'),('observation','other'),('revision',True),
                            ('total_messages',3),('total_messages',True),('message_offset',1),
                            ('message_offset',False),('history_truncated',True)]:
            changed=copy.deepcopy(snapshot);changed[field]=value
            with self.subTest(field=field),self.assertRaises(RuntimeError):m.verify_snapshot(changed,SESSION,before)
        changed=copy.deepcopy(snapshot);changed['messages'][1]['projection_truncated']=True
        with self.assertRaises(RuntimeError):m.verify_snapshot(changed,SESSION,before)
        changed=copy.deepcopy(snapshot);changed['messages'].pop()
        with self.assertRaises(RuntimeError):m.verify_snapshot(changed,SESSION,before)

    def test_unknown_or_omitted_public_fields_do_not_silently_normalize(self):
        before,snapshot=pair()
        for added in [True,False]:
            changed=copy.deepcopy(snapshot)
            if added:changed['messages'][0]['unapproved_extra']='unknown'
            else:del changed['messages'][0]['tool_calls']
            with self.subTest(added=added),self.assertRaises(RuntimeError):m.verify_snapshot(changed,SESSION,before)
        changed=copy.deepcopy(before);changed[0]['unapproved_extra']='unknown'
        with self.assertRaisesRegex(RuntimeError,'unknown canonical'):m.verify_snapshot(snapshot,SESSION,changed)

    def test_private_replay_bytes_remain_in_complete_raw_preservation_witness(self):
        before,snapshot=pair();after=copy.deepcopy(before);after[1]['provider_state']['opaque']='different private state'
        self.assertNotEqual(before,after,'caller complete DB before/after comparison must reject this drift')
        first=m.verify_snapshot(snapshot,SESSION,before);second=m.verify_snapshot(snapshot,SESSION,after)
        self.assertEqual(first['public_messages_sha256'],second['public_messages_sha256'])
        self.assertNotEqual(first['canonical_messages_sha256'],second['canonical_messages_sha256'])

    def test_numeric_boolean_metadata_and_index_types_do_not_compare_equal(self):
        before,snapshot=pair()
        for field,value in [('tool_success',1),('message_index',True),('projection_truncated',0)]:
            changed=copy.deepcopy(snapshot);changed['messages'][1][field]=value
            with self.subTest(field=field),self.assertRaises(RuntimeError):m.verify_snapshot(changed,SESSION,before)

    def test_old_interruption_projection_only_exposes_nonnil_assistant_identity(self):
        for role,identity in [('user',INTERRUPTED),('assistant','00000000-0000-0000-0000-000000000000')]:
            message={'role':role,'content':'exact text','interrupted_attempt':identity}
            public={**NULLS,'role':role,'content':'exact text','parts':[],'tool_calls':[],
                    'message_index':0,'projection_truncated':False}
            snapshot={'session_id':SESSION,'projection':'public-v1','observation':'snapshot','revision':17,'total_messages':1,'message_offset':0,
                      'history_truncated':False,'messages':[public]}
            self.assertTrue(m.verify_snapshot(snapshot,SESSION,[message])['full_text_order_identity_verified'])
        with self.assertRaises(ValueError):m.full_public_message({'role':'assistant','content':'x','interrupted_attempt':'not-uuid'})


class ReadOnlyCheckpointContracts(unittest.TestCase):
    def test_raw_private_drift_or_supervisor_start_reuse_refuses_without_replaying_effects(self):
        for drift in ['none','provider_state','supervisor_start','public_text']:
            with self.subTest(drift=drift),tempfile.TemporaryDirectory() as directory:
                home=Path(directory);work=home/'q401';work.mkdir();state=home/'state';state.mkdir()
                install=home/'install';(install/'updates').mkdir(parents=True)
                binary=work/'candidate/release/bin';binary.mkdir(parents=True)
                for name in ['vessel','voyage-installer']:(binary/name).write_bytes(name.encode())
                def file_digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
                manifest={'schema_version':1,'version':'fixture-only','target':'fixture-target',
                          'binaries':{name:{'sha256':file_digest(binary/name)} for name in ['vessel','voyage-installer']}}
                (binary.parent/'release.json').write_text(json.dumps(manifest))
                (work/'candidate-bin.json').write_text(json.dumps(str(binary/'voyage-installer')))
                op='bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';old='a'*64
                record={'operation_id':op,'legacy_mode':True,'channel':'local-owner','created_at':2000000000,
                        'current_release':old,'release_id':m.release_identity(manifest),'legacy_proof':None}
                (install/'updates'/(op+'.json')).write_text(json.dumps(record))
                (work/'fail-startup-attempt.json').write_text('{}');os.utime(work/'fail-startup-attempt.json',(1999999999,1999999999))
                original={'signals':[{'pid':739,'start_ticks':100,'uid':1000,'signal_delivered':True,
                                     'executable_sha256':manifest['binaries']['vessel']['sha256']}]}
                failed=work/'fail-startup-monitor-result.json';failed.write_text(json.dumps(original));original_bytes=failed.read_bytes()
                canonical,snapshot=pair();service={'pid':1037,'executable_sha256':'old-image','unit_sha256':'old-unit','unit_file_state':'enabled'}
                before={'service':{**service,'pid':777},'sessions':{SESSION:{'schema':12,'messages':canonical}}}
                (work/'before.json').write_text(json.dumps(before))
                observations=[{'service':service,'catalogue_schema':1,'quarantine':False,'sessions':copy.deepcopy(before['sessions'])} for _ in range(2)]
                if drift=='provider_state':observations[1]['sessions'][SESSION]['messages'][1]['provider_state']['opaque']='changed private state'
                if drift=='public_text':snapshot['messages'][1]['content']='changed public text'
                writes={};calls=[];starts=iter([500,501] if drift=='supervisor_start' else [500,500])
                def process_start(pid,allow_exited=False):
                    if pid==739:raise FileNotFoundError()
                    self.assertEqual(pid,1037);return next(starts)
                def write(name,value):
                    with (work/name).open('x') as stream:json.dump(value,stream)
                    writes[name]=value
                shim=types.SimpleNamespace(WORK=work,STATE=state,INSTALL=install,__file__=str(Path(__file__).with_name('native_legacy_qualification.py')),
                      write=write,digest=file_digest,installed_bin=lambda:install/'releases'/old/'bin',
                      observation=lambda:observations.pop(0),process_start=process_start,
                      sqlite_observe=lambda *_args:[(17,)])
                class Reader:
                    def command(_self,session,command):
                        self.assertEqual(session,SESSION);self.assertEqual(command,{'op':'snapshot'})
                        calls.append((session,command));return snapshot
                fake=types.ModuleType('delivery_recovery');fake.Fixture=Reader
                class Observer:
                    def __init__(_self,*_args):pass
                    def finish(_self):return {'cleanup_observation':'observed','direct_children_unchanged':True,'observed_helpers':[{'pid':900,'start_ticks':600,'exit_and_reaping_observed':True}]}
                with patch.dict(sys.modules,{'delivery_recovery':fake}),patch.object(m,'ObserverWatch',Observer):
                    if drift=='none':m.qualify_restored(shim,types.SimpleNamespace(case='startup'))
                    else:
                        with self.assertRaises(RuntimeError):m.qualify_restored(shim,types.SimpleNamespace(case='startup'))
                self.assertEqual(calls,[(SESSION,{'op':'snapshot'})]);self.assertEqual(failed.read_bytes(),original_bytes)
                self.assertEqual(writes['readonly-restored-startup-result.json']['status'],'qualified' if drift=='none' else 'unqualified')
                self.assertFalse(writes['readonly-restored-startup-result.json']['upgrade_or_signal_replayed'])
                self.assertEqual((work/'readonly-restored-startup-qualified.json').exists(),drift=='none')
                if drift=='none':
                    self.assertEqual(writes['readonly-restored-startup-qualified.json']['restored_supervisor_start_ticks'],500)
                    self.assertFalse(writes['readonly-restored-startup-qualified.json']['original_legacy_proof_present'])
                with self.assertRaises(FileExistsError):m.qualify_restored(shim,types.SimpleNamespace(case='startup'))

    def test_positive_original_retirement_never_signals_or_accepts_same_start_zombie(self):
        witness={'pid':739,'start_ticks':100,'uid':1000}
        with patch.object(m,'original_identity_retired',wraps=m.original_identity_retired):
            for current,expected in [(100,False),(101,True)]:
                shim=types.SimpleNamespace(process_start=lambda *_args,**_kw:current)
                self.assertEqual(m.original_identity_retired(shim,witness),expected)
            shim=types.SimpleNamespace(process_start=lambda *_args,**_kw:(_ for _ in ()).throw(FileNotFoundError()))
            self.assertTrue(m.original_identity_retired(shim,witness))
            with self.assertRaises(RuntimeError):m.original_identity_retired(shim,{**witness,'uid':0})

    def test_public_revision_or_missing_observation_marker_refuses(self):
        canonical,snapshot=pair()
        with self.assertRaises(RuntimeError):m.verify_snapshot(snapshot,SESSION,canonical,18)
        del snapshot['observation']
        with self.assertRaises(RuntimeError):m.verify_snapshot(snapshot,SESSION,canonical,17)


class ObserverCleanupContracts(unittest.TestCase):
    def test_no_captured_helper_never_fabricates_a_cleanup_witness(self):
        watcher=object.__new__(m.ObserverWatch);watcher.closed=False;watcher.outcome=None
        watcher.stop=types.SimpleNamespace(set=lambda:None)
        watcher.thread=types.SimpleNamespace(join=lambda timeout:None,is_alive=lambda:False)
        watcher.supervisor=1037;watcher.started=500;watcher.baseline=set();watcher.errors=[];watcher.witnesses={}
        watcher.scan=lambda:set()
        result=watcher.finish()
        self.assertEqual(result['cleanup_observation'],'unknown');self.assertEqual(result['observed_helpers'],[])
        self.assertTrue(result['direct_children_unchanged']);self.assertTrue(result['no_signals_or_effects'])
        self.assertIs(watcher.finish(),result)

    def test_exact_observed_helper_requires_pidfd_exit_and_positive_reaping(self):
        for retired in [False,True]:
            watcher=object.__new__(m.ObserverWatch);watcher.closed=False;watcher.outcome=None
            watcher.stop=types.SimpleNamespace(set=lambda:None)
            watcher.thread=types.SimpleNamespace(join=lambda timeout:None,is_alive=lambda:False)
            watcher.supervisor=1037;watcher.started=500;watcher.baseline=set();watcher.errors=[]
            watcher.witnesses={(900,600):{'descriptor':41,'pid':900,'start_ticks':600,'uid':1000}}
            watcher.scan=lambda:set();watcher.q=types.SimpleNamespace(retired=lambda _w:retired,pidfd_exited=lambda _fd:True)
            with patch.object(m.os,'close') as close:
                result=watcher.finish();self.assertEqual(result['cleanup_observation'],'observed' if retired else 'unknown')
                self.assertEqual(result['observed_helpers'][0]['exit_and_reaping_observed'],retired)
                close.assert_called_once_with(41);self.assertIs(watcher.finish(),result);close.assert_called_once_with(41)


if __name__=='__main__':unittest.main()
