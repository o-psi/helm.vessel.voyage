"""Offline source contracts for private synthetic failure capture; no browsers launched."""
import ast
import pathlib
import unittest


class FailureCaptureTests(unittest.TestCase):
    def setUp(self):
        self.tree = ast.parse(pathlib.Path(__file__).with_name('transport-probe.py').read_text())
        self.finally_body = next(node.finalbody for node in self.tree.body
                                 if isinstance(node, ast.Try) and node.finalbody)

    def test_capture_precedes_teardown(self):
        statements = [ast.unparse(node) for node in self.finally_body]
        capture = next(n for n, text in enumerate(statements) if 'pre-teardown.json' in text)
        teardown = next(n for n, text in enumerate(statements) if 'send_signal' in text)
        self.assertLess(capture, teardown)

    def test_capture_is_fixed_fields_not_exception_or_browser_content(self):
        capture = next(node for node in self.finally_body if 'pre-teardown.json' in ast.unparse(node))
        record = next(node for node in ast.walk(capture) if isinstance(node, ast.Dict))
        self.assertEqual({node.value for node in record.keys}, {
            'version', 'fixture_stage', 'provider_stage', 'failure_category', 'owned_processes'})
        text = ast.unparse(capture)
        self.assertIn('p.poll()', text)
        self.assertNotIn('str(', text)
        self.assertNotIn('repr(', text)
        self.assertNotIn('secret', text)
        self.assertNotIn('csrf', text)
        self.assertNotIn('bodies', text)
        self.assertNotIn('traceback', text)
        self.assertIn('connection_refused', text)
        self.assertIn('fixture_error', text)

    def test_fixture_stage_is_a_literal_allowlist(self):
        stages = {node.value.value for node in ast.walk(self.tree)
                  if isinstance(node, ast.Assign) and isinstance(node.value, ast.Constant)
                  and any(isinstance(target, ast.Name) and target.id == 'fixture_stage'
                          for target in node.targets)}
        self.assertEqual(stages, {'setup', 'vessel_start', 'account_binding', 'session_bootstrap',
                                  'viewer_start', 'local_consent', 'main_browser_run',
                                  'main_browser_acceptance', 'final_evidence'})


if __name__ == '__main__':
    unittest.main()
