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
        self.panel_open=True;self.dismiss=True;self.draft='';self.discarded=[]
        self.old=self.root/'old'/'open.html';self.old.parent.mkdir(mode=0o700);self.old.write_bytes(b'fixture only');self.old.chmod(0o600)
        self.new=self.root/'new'/'open.html';self.new.parent.mkdir(mode=0o700);self.new.write_bytes(b'fixture only');self.new.chmod(0o600)
        self.capture.write_text(str(self.old));self.capture.chmod(0o600)
        self.session=str(uuid.uuid4());self.title='qualification-333-fixture-a'
        self.display_session=self.session;self.display_title=self.title
        self.client_root={'label':'native-a','pid':901,'start_ticks':500,'descendants':True}
        self.patch=patch.object(m,'process_identity',return_value=(500,str(self.program)));self.patch.start()
        def screen(_):
            return self.display_title+('\nHost browser\nVoyage: '+self.display_session if self.panel_open else '')+('\nViewer detached; host browser remains owned by Voyage' if self.detached else '')
        def paste(_,text):
            self.assertTrue((self.directory/'pending.json').exists());self.assertEqual(text,'/browser detach');self.actions.append(('paste',text))
            if self.panel_open: self.discarded.append(('paste',text))
            else: self.draft+=text
        def send(_,value):
            self.assertTrue((self.directory/'pending.json').exists())
            self.actions.append(('send',value))
            if self.panel_open:
                if value=='\x1b' and self.dismiss: self.panel_open=False
                elif value!='\x1b': self.discarded.append(('send',value))
                return
            if value=='\r' and self.draft=='/browser detach': self.detached=True;self.draft=''
            if value=='\x1b[17~':
                self.panel_open=True
                if self.fresh: self.capture.write_text(str(self.new))
        def wait(predicate,_): self.assertTrue(predicate(),'bounded synthetic stage unobserved')
        self.handler=m.Reopen(self.directory,self.session,'a',self.title,self.client_root,{'path':str(self.program),'sha256':hashlib.sha256(self.program.read_bytes()).hexdigest()},
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
        self.assertEqual(self.actions,[('send','\x1b'),('paste','/browser detach'),('send','\r'),('send','\x1b[17~')])
        self.assertEqual(self.discarded,[]);self.assertTrue(self.panel_open);self.assertTrue(self.detached)
        self.handler.poll();self.assertEqual(len(self.actions),4)
    def test_foreign_fixture_process_action_path_and_expiry_refuse_before_any_key(self):
        for fields in ({'session_id':str(uuid.uuid4())},{'label':'b'},{'client':{**self.client_root,'start_ticks':501}},{'command':'arbitrary'}, {'launcher':'/private/credential'},{'expires_at_ms':0}):
            with self.subTest(fields=list(fields)):
                self.request(**fields)
                with self.assertRaises(AssertionError): self.handler.poll()
                self.assertEqual(self.actions,[]);self.assertFalse((self.directory/'pending.json').exists())
    def test_foreign_current_panel_refuses_before_esc_and_retains_unknown(self):
        self.request();self.display_session=str(uuid.uuid4())
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertEqual(self.actions,[])
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
        self.handler.poll();self.assertEqual(self.actions,[])
    def test_foreign_fixture_title_refuses_before_esc(self):
        self.request();self.display_title='qualification-333-fixture-b'
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertEqual(self.actions,[])
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
    def test_panel_not_dismissed_prevents_discarded_detach_enter_and_f6(self):
        self.request();self.dismiss=False
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertEqual(self.actions,[('send','\x1b')]);self.assertEqual(self.discarded,[])
        self.assertFalse(self.detached);self.assertEqual(self.draft,'')
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
        self.handler.poll();self.assertEqual(self.actions,[('send','\x1b')])
    def test_unknown_f6_result_is_retained_without_second_detach_or_key_attempt(self):
        self.request();self.fresh=False
        with self.assertRaises(AssertionError): self.handler.poll()
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertEqual(reply['status'],'unknown');self.assertIs(reply['outcome_unknown'],True)
        self.assertTrue((self.directory/'pending.json').exists());self.assertTrue(self.handler.attempted)
        before=list(self.actions);self.handler.poll();self.assertEqual(self.actions,before)
    def test_fresh_launcher_with_foreign_returned_panel_retains_unknown_without_replay(self):
        self.request();original=self.handler.send
        def foreign(client,value):
            original(client,value)
            if value=='\x1b[17~': self.display_session=str(uuid.uuid4())
        self.handler.send=foreign
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertEqual(self.capture.read_text(),str(self.new))
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
        before=list(self.actions);self.handler.poll();self.assertEqual(self.actions,before)
    def test_changed_live_identity_after_detach_cannot_send_f6_or_replay(self):
        self.request();original=self.handler.verify;calls=0
        def changed():
            nonlocal calls
            calls+=1
            if calls>=4: raise AssertionError('fixed identity changed')
            original()
        self.handler.verify=changed
        with self.assertRaises(AssertionError): self.handler.poll()
        self.assertNotIn(('send','\x1b[17~'),self.actions)
        self.assertTrue(self.detached);self.assertEqual(self.discarded,[])
        reply=json.loads((self.directory/'response.json').read_bytes());self.assertIs(reply['outcome_unknown'],True)
        self.handler.poll();self.assertEqual(self.actions,[('send','\x1b'),('paste','/browser detach'),('send','\r')])

if __name__=='__main__': unittest.main()
