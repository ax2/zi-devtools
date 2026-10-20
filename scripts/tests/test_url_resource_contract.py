import json
from pathlib import Path
import unittest

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]


class UrlResourceContractTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads((ROOT / 'plugins/url-text-resource/plugin.json').read_text(encoding='utf-8'))
        self.cap = self.manifest['contributes']['capabilities'][0]
        self.schema = json.loads((ROOT / 'plugins/url-text-resource/schemas/output.schema.json').read_text(encoding='utf-8'))
        self.validator = Draft202012Validator(self.schema)

    def test_real_success_and_seven_business_errors_are_representable(self):
        self.validator.validate({'ok': True, 'encodedText': '%E4%B8%AD', 'sourceByteCount': 3})
        for code in ['INVALID_REQUEST', 'INVALID_RESOURCE', 'EMPTY_RESOURCE', 'INPUT_TOO_LARGE',
                     'UNSUPPORTED_MIME', 'INVALID_UTF8', 'RESOURCE_READ_FAILED']:
            self.validator.validate({'ok': False, 'error': {'code': code, 'message': '错误说明'}})

    def test_mixed_missing_unknown_and_excess_fields_are_rejected(self):
        success = {'ok': True, 'encodedText': 'a', 'sourceByteCount': 1}
        failure = {'ok': False, 'error': {'code': 'INVALID_UTF8', 'message': '错误说明'}}
        for invalid in [
            {**success, 'error': failure['error']}, {**failure, 'encodedText': 'a'},
            {'ok': False}, {**success, 'sourceByteCount': 8193},
            {'ok': False, 'error': {'code': 'UNKNOWN', 'message': '错误'}},
            {'ok': False, 'error': {'code': 'INVALID_UTF8'}},
            {'ok': False, 'error': {'code': 'INVALID_UTF8', 'message': 'x'*257}},
            {**failure, 'path': 'unrequested'}, {**success, 'encodedText': 'a'*24577},
        ]:
            with self.subTest(value=str(invalid)[:80]):
                self.assertFalse(self.validator.is_valid(invalid))

    def test_declared_features_have_explicit_profiles(self):
        self.assertEqual(self.cap['outputSchema'], self.schema)
        self.assertEqual(self.manifest['requiredProfiles'], {
            'resource.input.types.v1': '=1.0.0-preview.1',
            'output.schema.v1': '=1.0.0-rc.1', 'tool.directory.v1': '=1.0.0-preview.1'})
        self.assertEqual(self.cap['resourceInputMimeTypes'], ['text/plain'])
        self.assertEqual(self.cap['id'], 'devtools.transforms.url.encode')


if __name__ == '__main__':
    unittest.main()
