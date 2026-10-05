from pathlib import Path
import json
import sys
import tempfile
import unittest
from coverage_compile_record import run


class RecorderTests(unittest.TestCase):
    def test_forward_failure_and_preserve_intent(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(run([sys.executable, '-c', 'raise SystemExit(7)'], directory), 7)
            records = [json.loads(p.read_text()) for p in Path(directory).glob('*.json')]
            self.assertEqual({r['phase'] for r in records}, {'pending', 'completed'})
            self.assertEqual(next(r for r in records if r['phase'] == 'completed')['exit_status'], 7)
            self.assertTrue(all('env' not in r for r in records))

    def test_absent_directory_refuses_before_effect(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                run([sys.executable, '-c', 'raise SystemExit(0)'], Path(directory) / 'missing')
