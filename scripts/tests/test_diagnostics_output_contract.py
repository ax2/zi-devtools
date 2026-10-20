import copy
import hashlib
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest

from jsonschema import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from diagnostics_output_contract import ROOT, load_contracts, load_report_contracts, validate_result, verify_packaged_fixtures, validate_runtime_evidence
from tool_directory import validate_directory


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()


class DiagnosticOutputContractTests(unittest.TestCase):
    def test_runtime_vectors_have_unique_ids_and_preserve_each_action(self):
        from diagnostics_vectors import cases, registry, samples, sample_ids
        ids = [case[0] for case in cases]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertGreaterEqual(len(ids), 258)
        self.assertTrue(registry <= set(ids))
        self.assertEqual(len(sample_ids), len(samples))
        for case_id in sample_ids:
            self.assertIn(case_id + '-request-49152', ids)
            self.assertIn(case_id + '-text-utf8-8192', ids)

    def test_runtime_evidence_rejects_partial_diagnostic_and_mismatched_runs(self):
        functional = {'cases': [dict(id='a', requestSha256='request', resultSha256='result')]}
        fuel = {'cases': [dict(id='a', requestSha256='request', expectedSha256='result', passBudget=True)], 'failures': []}
        cold = {'rows': [dict(id='a', resultSha256='result', passed=True, closed=True)], 'failures': [], 'remaining': 0, 'diagnosticOnly': False}
        validate_runtime_evidence(functional, fuel, cold)
        for target, key, value in [('cold', 'diagnosticOnly', True), ('cold', 'remaining', 1), ('cold', 'failures', ['a']), ('fuel', 'failures', ['a'])]:
            altered_fuel, altered_cold = copy.deepcopy(fuel), copy.deepcopy(cold)
            (altered_cold if target == 'cold' else altered_fuel)[key] = value
            with self.assertRaises(ValueError):
                validate_runtime_evidence(functional, altered_fuel, altered_cold)
        for key, value in [('id', 'different'), ('resultSha256', 'different'), ('passed', False), ('closed', False)]:
            altered = copy.deepcopy(cold)
            altered['rows'][0][key] = value
            with self.assertRaises(ValueError):
                validate_runtime_evidence(functional, fuel, altered)
        for rows in [[], cold['rows'] * 2]:
            with self.assertRaises(ValueError):
                validate_runtime_evidence(functional, fuel, dict(cold, rows=rows))

    def test_packaged_fixture_checks_digests_and_rejects_path_escape(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder)
            shutil.copyfile(ROOT / 'plugins/diagnostics/plugin.json', package / 'plugin.json')
            shutil.copytree(ROOT / 'contracts/diagnostics/v2', package / 'contracts/diagnostics/v2')
            request = b'invalid synthetic input'
            expected = encode(dict(contractVersion='1.0.0-rc.1', ok=False, data=None,
                                   error={'code': 'INVALID_INPUT', 'message': 'Invalid input'}))
            case = dict(requestFile='case.request', expectedFile='case.expected.json',
                        requestSha256=hashlib.sha256(request).hexdigest(),
                        expectedSha256=hashlib.sha256(expected).hexdigest())
            for kind in ('budget', 'serializer'):
                fixtures = package / 'fixtures' / kind
                fixtures.mkdir(parents=True)
                (fixtures / case['requestFile']).write_bytes(request)
                (fixtures / case['expectedFile']).write_bytes(expected)
                (fixtures / 'fuel-proof.json').write_bytes(encode({'cases': [case]}))
            result = verify_packaged_fixtures(package)
            self.assertEqual(result['fixtureCounts'], {'budget': 1, 'serializer': 1})
            self.assertFalse(result['runtimeExecuted'])
            fixtures = package / 'fixtures/budget'
            (fixtures / case['expectedFile']).write_bytes(expected + b' ')
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                verify_packaged_fixtures(package)
            (fixtures / 'fuel-proof.json').write_bytes(encode({'cases': [{**case, 'requestFile': '../outside'}]}))
            with self.assertRaisesRegex(ValueError, 'local basenames'):
                verify_packaged_fixtures(package)

    def test_operation_report_provenance_refs_resolve_offline_and_reject_missing_fields(self):
        validators = load_report_contracts()
        self.assertEqual(len(validators), 11)
        report = dict(diagnosticsProtocol='2.0.0-rc.1', data={}, generatedBy=dict(
            toolId='synthetic', toolVersion='1.0.0', runtimeFingerprint=dict(
                schema=1, appVersion='0.82.0-dev.105', os='wasi', architecture='wasm32',
                buildProfile='plugin', sha256='0' * 64)))
        for validator in validators.values():
            validator.validate(report)
            broken = copy.deepcopy(report)
            del broken['generatedBy']['runtimeFingerprint']['sha256']
            with self.assertRaises(ValidationError):
                validator.validate(broken)

    def test_all_capabilities_have_searchable_bounded_directory_metadata(self):
        manifest = json.loads((ROOT / 'plugins/diagnostics/plugin.json').read_text(encoding='utf-8'))
        self.assertEqual(manifest['requiredProfiles']['tool.directory.v1'], '=1.0.0-preview.1')
        for capability in manifest['contributes']['capabilities']:
            directory = capability['directory']
            validate_directory(directory)
            self.assertEqual(directory['categoryId'], 'code')
            self.assertGreaterEqual(len(directory['aliases']), 2)
            self.assertTrue(directory['description'].startswith('仅分析用户提供的报告文本。'))

    def setUp(self):
        self.contracts = load_contracts()
        self.success = dict(contractVersion='1.0.0-rc.1', ok=True, data={'text': '{}'}, error=None)

    def test_all_capabilities_accept_success_and_each_runtime_failure(self):
        codes = ['INVALID_INPUT', 'INPUT_TOO_LARGE', 'UNSUPPORTED_OPERATION', 'INVALID_REPORT', 'OUTPUT_TOO_LARGE']
        for validator in self.contracts.values():
            validate_result(validator, encode(self.success))
            for code in codes:
                validate_result(validator, encode(dict(contractVersion='1.0.0-rc.1', ok=False,
                                                     data=None, error={'code': code, 'message': '输入无效'})))

    def test_rejects_missing_extra_wrong_type_and_inconsistent_envelopes(self):
        invalid = []
        for key in self.success:
            row = copy.deepcopy(self.success)
            del row[key]
            invalid.append(row)
        invalid += [{**self.success, 'extra': True}, {**self.success, 'ok': False},
                    {**self.success, 'data': {'text': 2}}, {**self.success, 'data': {'text': '{}', 'extra': 1}},
                    {**self.success, 'contractVersion': '2.0.0'},
                    {**self.success, 'ok': False, 'data': None, 'error': {'code': 'UNKNOWN', 'message': 'x'}}]
        for validator in self.contracts.values():
            for value in invalid:
                with self.assertRaises(ValidationError):
                    validate_result(validator, encode(value))

    def test_utf8_and_escaped_exact_serialized_boundaries(self):
        validator = next(iter(self.contracts.values()))
        for prefix in ('', '中文', '中\x00"\\'):
            base = {**self.success, 'data': {'text': prefix}}
            overhead = len(encode(base))
            for size in (49151, 49152, 49153):
                value = {**base, 'data': {'text': prefix + 'a' * (size - overhead)}}
                encoded = encode(value)
                self.assertEqual(len(encoded), size)
                if size <= 49152:
                    validate_result(validator, encoded)
                else:
                    with self.assertRaisesRegex(ValueError, '48 KiB'):
                        validate_result(validator, encoded)

    def test_missing_output_declarations_are_rejected_before_packaging(self):
        manifest = json.loads((ROOT / 'plugins/diagnostics/plugin.json').read_text(encoding='utf-8'))
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'plugin.json'
            for key in ('outputSchema', 'maxResultBytes'):
                bad = copy.deepcopy(manifest)
                del bad['contributes']['capabilities'][0][key]
                path.write_bytes(encode(bad))
                with self.assertRaisesRegex(ValueError, 'Every diagnostic capability'):
                    load_contracts(path)


if __name__ == '__main__':
    unittest.main()
