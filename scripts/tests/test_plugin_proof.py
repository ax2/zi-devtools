import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from plugin_proof import validate_cold


class ColdProofTests(unittest.TestCase):
    def setUp(self):
        self.engine = dict(nativeEngineVersion='36.0.2', dllSha256='d' * 64)
        self.parity = dict(cases=[dict(id='a', expectedSha256='a' * 64), dict(id='b', expectedSha256='b' * 64)])
        self.cold = dict(wasmSha256='c' * 64, wasmBytes=20, engine=self.engine, remaining=0, failures=[],
                         rows=[dict(id=name, closed=True, passed=True, elapsedMs=100, exitCode=0, resultSha256=name * 64) for name in ['a', 'b']])

    def verify(self, cold=None, require_pass=True):
        return validate_cold(self.cold if cold is None else cold, self.parity, 'c' * 64, 20, self.engine, require_pass)

    def test_exact_results_pass(self):
        self.assertEqual(self.verify(), [])

    def test_duplicate_case_cannot_replace_missing_case(self):
        self.cold['rows'][1] = copy.deepcopy(self.cold['rows'][0])
        with self.assertRaises(AssertionError):
            self.verify()

    def test_another_module_or_engine_is_rejected(self):
        for key, value in [('wasmSha256', 'x' * 64), ('wasmBytes', 21)]:
            candidate = copy.deepcopy(self.cold); candidate[key] = value
            with self.assertRaises(AssertionError):
                self.verify(candidate)
        candidate = copy.deepcopy(self.cold); candidate['engine']['dllSha256'] = 'x' * 64
        with self.assertRaises(AssertionError):
            self.verify(candidate)

    def test_result_from_other_case_cannot_keep_pass_flag(self):
        self.cold['rows'][0]['resultSha256'] = 'b' * 64
        with self.assertRaises(AssertionError):
            self.verify()

    def test_deadline_failure_is_preserved_even_with_correct_result(self):
        self.cold['rows'][0].update(elapsedMs=5000, passed=False)
        self.cold['failures'] = ['a']
        self.assertEqual(self.verify(require_pass=False), ['a'])
        with self.assertRaises(AssertionError):
            self.verify()
        self.cold['failures'] = []
        with self.assertRaises(AssertionError):
            self.verify(require_pass=False)

    def test_nonfinite_metrics_and_unclosed_process_are_rejected(self):
        for value in [float('nan'), float('inf'), -1, True]:
            candidate = copy.deepcopy(self.cold); candidate['rows'][0]['elapsedMs'] = value
            with self.assertRaises(AssertionError):
                self.verify(candidate)
        self.cold['rows'][0]['closed'] = False
        with self.assertRaises(AssertionError):
            self.verify()


if __name__ == '__main__':
    unittest.main()
