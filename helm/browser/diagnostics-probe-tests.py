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
        protected = next(node for node in self.finally_body if isinstance(node, ast.Try))
        self.assertIn('pre-teardown.json', ast.unparse(protected.body))
        self.assertIn('send_signal', ast.unparse(protected.finalbody))
        self.assertTrue(protected.handlers)

    def test_capture_is_fixed_fields_not_exception_or_browser_content(self):
        capture = next(node for node in ast.walk(self.tree) if isinstance(node, ast.Expr) and 'pre-teardown.json' in ast.unparse(node))
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


class InjectedWriteFailureTests(unittest.TestCase):
    def test_evidence_failure_always_enters_teardown(self):
        tree = ast.parse(pathlib.Path(__file__).with_name('transport-probe.py').read_text())
        outer = next(node for node in tree.body if isinstance(node, ast.Try) and node.finalbody)
        protected = next(node for node in outer.finalbody if isinstance(node, ast.Try))
        # Execute the exact evidence protection with an injected write failure;
        # replace real process teardown with an observable owned-fixture marker.
        protected = ast.Try(body=protected.body, handlers=protected.handlers,
                            orelse=[], finalbody=ast.parse('teardown.append(True)').body)
        code = compile(ast.fix_missing_locations(ast.Module(body=[protected], type_ignores=[])), '<fixture>', 'exec')
        class Unwritable:
            def __truediv__(self, name):
                return self
            def write_text(self, text):
                raise OSError('injected disk full/private details')
        for failure in [None, AssertionError]:
            import json
            env = dict(E=Unwritable(), json=json, fixture_stage='setup', stage=0,
                       failure=failure, children=[], evidence_failed=False, teardown=[])
            exec(code, env)
            self.assertEqual(env['teardown'], [True])
            self.assertTrue(env['evidence_failed'])
            self.assertIs(env['failure'], failure)


if __name__ == '__main__':
    unittest.main()
