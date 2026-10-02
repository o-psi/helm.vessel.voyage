"""Offline typed/one-shot reopen contracts; callbacks simulate TUI, never execute."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch
import uuid

spec=importlib.util.spec_from_file_location('native_reopen',Path(__file__).with_name('host_browser_native_reopen.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class Process:
    def poll(self): return None

class Contracts(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='native-reopen-contract-');self.root=Path(self.temp.name)
        self.directory=self.root/'mailbox';self.directory.mkdir(mode=0o700)
        self.program=self.root/'helm';self.program.write_bytes(b'fixture-program-not-executed');self.program.chmod(0o700)
        self.capture=self.root/'capture';self.actions=[];self.detached=False;self.fresh=True
        self.old=self.root/'old'/'open.html';self.old.parent.mkdir(mode=0o700);self.old.write_bytes(b'fixture only');self.old.chmod(0o600)
        self.new=self.root/'new'/'open.html';self.new.parent.mkdir(mode=0o700);self.new.write_bytes(b'fixture only');self.new.chmod(0o600)
        self.capture.write_text(str(self.old));self.capture.chmod(0o600)
        self.session=str(uuid.uuid4());self.client_root={'label':'native-a','pid':901,'start_ticks':500,'descendants':True}
        self.patch=patch.object(m,'process_identity',return_value=(500,str(self.program)));self.patch.start()
        def screen(_): return 'Voyage: '+self.session+('\nViewer detached; host browser remains owned by Voyage' if self.detached else '')
        def paste(_,text):
            self.assertTrue((self.directory/'pending.json').exists());self.assertEqual(text,'/browser detach');self.actions.append(('paste',text))
        def send(_,value):
            self.actions.append(('send',value))
            if value=='\r': self.detached=True
            if value=='\x1b[17~' and self.fresh: self.capture.write_text(str(self.new))
        def wait(predicate,_): self.assertTrue(predicate(),'bounded synthetic stage unobserved')
        self.handler=m.Reopen(self.directory,self.session,'a',self.client_root,{'path':str(self.program),'sha256':hashlib.sha256(self.program.read_bytes()).hexdigest()},
                              {'process':Process(),'pid':901},self.capture,self.root,screen,paste,send,wait)
    def tearDown(self): self.handler.close();self.patch.stop();self.temp.cleanup()
    def request(self,**extra):
        value={'schema':1,'id':str(uuid.uuid4()),'action':'native_reopen','session_id':self.session,'label':'a','client':self.client_root,'expires_at_ms':int(time.time()*1000)+45000,**extra}
        value['digest']=hashlib.sha256(m.encode(value)).hexdigest()
        (self.directory/'request.json').write_bytes(m.encode(value));(self.directory/'request.json').chmod(0o600)
        return value
    def test_verified_fixed_action_has_durable_pending_before_keys_and_fresh_one_use_result(self):
        request=self.request();self.handler.poll();reply=json.loads((self.directory/'response.json').read_bytes())
        self.assertEqual(reply['id'],request['id']);self.assertEqual(reply['digest'],request['digest']);self.assertEqual(reply['status'],'observed');self.assertIs(reply['outcome_unknown'],False)
        self.assertEqual(reply['client'],self.client_root);self.assertEqual(reply['launcher'],str(self.new));self.assertNotEqual(reply['launcher'],str(self.old))
        self.assertEqual(self.actions,[('paste','/browser detach'),('send','\r'),('send','\x1b[17~')]);self.handler.poll();self.assertEqual(len(self.actions),3)
    def test_foreign_fixture_process_action_path_and_expiry_refuse_before_any_key(self):
        for fields in ({'session_id':str(uuid.uuid4())},{'label':'b'},{'client':{**self.client_root,'start_ticks':501}},{'command':'arbitrary'}, {'launcher':'/private/credential'},{'expires_at_ms':0}):
            with self.subTest(fields=list(fields)):
                self.request(**fields)
                with self.assertRaises(AssertionError): self.handler.poll()
                self.assertEqual(self.actions,[]);self.assertFalse((self.directory/'pending.json').exists())
    def test_unknown_f6_result_is_retained_without_second_detach_or_key_attempt(self):
        self.request();self.fresh=False
        with self.assertRaises(AssertionError): self.handler.poll()
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertEqual(reply['status'],'unknown');self.assertIs(reply['outcome_unknown'],True)
        self.assertTrue((self.directory/'pending.json').exists());self.assertTrue(self.handler.attempted)
        before=list(self.actions);self.handler.poll();self.assertEqual(self.actions,before)
    def test_changed_live_identity_after_detach_cannot_send_f6_or_replay(self):
        self.request();original=self.handler.verify;calls=0
        def changed():
            nonlocal calls
            calls+=1
            if calls>=3: raise AssertionError('fixed identity changed')
            original()
        self.handler.verify=changed
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertNotIn(('send','\x1b[17~'),self.actions)
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
        self.handler.poll();self.assertEqual(self.actions,[('paste','/browser detach'),('send','\r')])

if __name__=='__main__': unittest.main()
