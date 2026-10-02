"""Local witness contracts; one owned-child pidfd test is not native rollback proof."""
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('legacy_native', Path(__file__).with_name('native_legacy_qualification.py'))
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)


class PendingWitnessContracts(unittest.TestCase):
    def test_permission_missing_and_already_exited_never_supply_signal_witness(self):
        for error, category in ((PermissionError(13,'denied'),'permission_pending'),(FileNotFoundError(),'missing_pending'),(ProcessLookupError(),'missing_pending')):
            with self.subTest(category=category), patch.object(q.os,'pidfd_open',return_value=41), patch.object(q,'pidfd_exited',return_value=False), patch.object(q.Path,'stat',side_effect=error), patch.object(q.os,'close') as close, patch.object(q.signal,'pidfd_send_signal') as sent:
                result,witness=q.exact_candidate(999,Path('/fixed/vessel'),'a'*64)
                self.assertEqual(result,category);self.assertIsNone(witness);close.assert_called_once_with(41);sent.assert_not_called()
        with patch.object(q.os,'pidfd_open',return_value=41),patch.object(q,'pidfd_exited',return_value=True),patch.object(q.os,'close') as close:
            self.assertEqual(q.exact_candidate(999,Path('/fixed/vessel'),'a'*64),('missing_pending',None));close.assert_called_once_with(41)

    def test_foreign_uid_and_changed_start_never_pass(self):
        with patch.object(q.os,'pidfd_open',return_value=41),patch.object(q,'pidfd_exited',return_value=False),patch.object(q.Path,'stat',return_value=SimpleNamespace(st_uid=0)),patch.object(q.os,'close'):
            self.assertEqual(q.exact_candidate(999,Path('/fixed/vessel'),'a'*64),('not_candidate',None))
        witness={'descriptor':41,'pid':999,'start_ticks':1,'image_pin':(1,2,3,4,5)}
        with patch.object(q,'pidfd_exited',return_value=False),patch.object(q.Path,'stat',return_value=SimpleNamespace(st_uid=1000)),patch.object(q,'process_start',return_value=2),patch.object(q.signal,'pidfd_send_signal') as sent:
            self.assertFalse(q.still_candidate(witness,Path('/fixed/vessel')));sent.assert_not_called()

    def test_exit_without_reaping_and_permission_denial_are_not_retirement(self):
        witness={'descriptor':41,'pid':999,'start_ticks':1}
        with patch.object(q,'pidfd_exited',return_value=False): self.assertFalse(q.retired(witness))
        with patch.object(q,'pidfd_exited',return_value=True),patch.object(q,'process_start',return_value=1): self.assertFalse(q.retired(witness))
        with patch.object(q,'pidfd_exited',return_value=True),patch.object(q,'process_start',side_effect=PermissionError(13,'denied')):
            with self.assertRaises(PermissionError): q.retired(witness)
        with patch.object(q,'pidfd_exited',return_value=True),patch.object(q,'process_start',side_effect=FileNotFoundError()): self.assertTrue(q.retired(witness))


@unittest.skipUnless(sys.platform=='linux' and os.getuid()==1000 and hasattr(os,'pidfd_open') and hasattr(signal,'pidfd_send_signal'),'ordinary Linux UID1000 pidfds required')
class OwnedLiveWitness(unittest.TestCase):
    def test_transient_denial_then_exact_process_pidfd_signal_exit_and_reaping(self):
        child=subprocess.Popen([sys.executable,'-I','-c','import time;time.sleep(30)'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        witness=None
        try:
            executable=Path(sys.executable).resolve(strict=True);expected=q.digest(executable)
            with patch.object(q.os,'readlink',side_effect=PermissionError(13,'synthetic transient denial')):
                category,missing=q.exact_candidate(child.pid,executable,expected)
                self.assertEqual(category,'permission_pending');self.assertIsNone(missing)
            deadline=time.monotonic()+3
            while time.monotonic()<deadline and witness is None:
                category,witness=q.exact_candidate(child.pid,executable,expected)
                if witness is None: time.sleep(.01)
            self.assertEqual(category,'verified');self.assertIsNotNone(witness)
            self.assertEqual(witness['uid'],1000);self.assertEqual(witness['pid'],child.pid)
            self.assertTrue(q.still_candidate(witness,executable))
            self.assertFalse(q.pidfd_exited(witness['descriptor']))
            signal.pidfd_send_signal(witness['descriptor'],signal.SIGKILL)
            self.assertEqual(child.wait(timeout=3),-signal.SIGKILL)
            self.assertTrue(q.pidfd_exited(witness['descriptor']));self.assertTrue(q.retired(witness))
            with patch.object(q.os,'readlink',side_effect=PermissionError(13,'synthetic late denial')):
                category,missing=q.exact_candidate(child.pid,executable,expected)
                self.assertEqual(category,'missing_pending');self.assertIsNone(missing)
        finally:
            if witness is not None: os.close(witness['descriptor'])
            if child.poll() is None: child.kill();child.wait(timeout=3)


if __name__=='__main__': unittest.main()
