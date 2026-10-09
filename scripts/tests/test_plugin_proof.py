import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from plugin_proof import validate_cold, validate_compute_metadata, validate_budget


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


class BudgetProofTests(unittest.TestCase):
    def setUp(self):
        case = dict(id='a', requestSha256='a' * 64, expectedSha256='b' * 64, requestBytes=10, resultBytes=50, expectedError=None)
        self.parity = dict(wasmSha256='c' * 64, wasmBytes=20, cases=[case])
        row = dict(**case, passBudget=True, trap=None, elapsedMs=10, fuelUsed=100, fuelRemaining=9999900, memoryBytes=65536)
        self.budget = dict(wasmSha256='c' * 64, wasmBytes=20, fixtureKind='plugin-runtime',
                           budget=dict(fuel=10000000, deadlineSeconds=5, memoryBytes=67108864, guestStackBytes=2097152),
                           engineProvenance=dict(nativeEngineVersion='36.0.2'), cases=[row], failures=[])

    def verify(self):
        return validate_budget(self.budget, self.parity, 'c' * 64, 20)

    def test_bound_measurements_pass(self):
        self.verify()

    def test_false_pass_cannot_hide_a_trap_or_exhaustion(self):
        for key, value in [('trap', 'fuel exhausted'), ('fuelRemaining', -1), ('fuelUsed', 10000001), ('memoryBytes', 67108865), ('elapsedMs', 5000), ('elapsedMs', float('nan'))]:
            old = self.budget['cases'][0][key]
            self.budget['cases'][0][key] = value
            with self.assertRaises(AssertionError):
                self.verify()
            self.budget['cases'][0][key] = old

    def test_other_case_bytes_and_duplicate_rows_are_rejected(self):
        self.budget['cases'][0]['expectedSha256'] = 'x' * 64
        with self.assertRaises(AssertionError):
            self.verify()
        self.budget['cases'][0]['expectedSha256'] = 'b' * 64
        self.budget['cases'].append(copy.deepcopy(self.budget['cases'][0]))
        with self.assertRaises(AssertionError):
            self.verify()


class ComputeMetadataTests(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).resolve().parents[2]
        load = lambda p: json.loads(p.read_text(encoding='utf-8'))
        self.reference = load(root / 'contracts/studio-devtools/v1/plugin.example.json')
        self.manifest = load(root / 'plugins/trace/plugin.json')
        self.catalog = load(root / 'plugins/trace/catalog.json')

    def verify(self):
        return validate_compute_metadata(self.manifest, self.catalog, self.reference)

    def test_frozen_pi_routes_bind_the_actual_trace_catalog(self):
        self.verify()

    def test_wrong_pi_field_name_is_rejected(self):
        tool = self.manifest['contributes']['piTools'][0]
        tool['capabilityId'] = tool.pop('capability')
        with self.assertRaises(AssertionError):
            self.verify()

    def test_unregistered_or_duplicate_route_is_rejected(self):
        for capability in ['devtools.foreign.operation', self.manifest['contributes']['piTools'][1]['capability']]:
            self.manifest['contributes']['piTools'][0]['capability'] = capability
            with self.assertRaises(AssertionError):
                self.verify()

    def test_input_and_authority_drift_is_rejected(self):
        original = copy.deepcopy(self.manifest)
        self.manifest['contributes']['piTools'][0]['inputSchema']['properties']['text']['maxLength'] = 8193
        with self.assertRaises(AssertionError):
            self.verify()
        self.manifest = original
        self.manifest['permissions']['required'].append('filesystem.read')
        with self.assertRaises(AssertionError):
            self.verify()


if __name__ == '__main__':
    unittest.main()
