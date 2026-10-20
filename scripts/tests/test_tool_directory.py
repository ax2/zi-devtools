import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tool_directory import apply_manifest, load_capabilities, validate_directory


class ToolDirectoryTests(unittest.TestCase):
    def test_mapping_covers_current_provider_ids(self):
        metadata = load_capabilities()
        self.assertEqual(len(metadata), 43)
        for capability_id in metadata:
            self.assertTrue(capability_id.startswith(('devtools.inspect.', 'devtools.trace.', 'devtools.text.', 'devtools.transforms.', 'devtools.json.object.', 'devtools.image.')))

    def test_directory_negotiation_preserves_other_required_profiles(self):
        manifest = {'id': 'com.zicode.devtools.json-object', 'version': '0.1.1',
                    'requiredProfiles': {'output.schema.v1': '=1.0.0-rc.1', 'result.json.v1': '=1.0.0-preview.1'},
                    'contributes': {'capabilities': [{'id': 'devtools.json.object.inspect'}, {'id': 'devtools.json.object.receive'}]}}
        original = copy.deepcopy(manifest['requiredProfiles'])
        apply_manifest(manifest)
        self.assertEqual(manifest['requiredProfiles'], dict(original, **{'tool.directory.v1': '=1.0.0-preview.1'}))
        manifest['requiredProfiles']['tool.directory.v1'] = '=9.0.0'
        with self.assertRaisesRegex(AssertionError, 'Conflicting'):
            apply_manifest(manifest)

    def test_apply_manifest_requires_exact_capability_coverage(self):
        manifest = {'schemaVersion': 1, 'id': 'example', 'version': '0.1.0',
                    'contributes': {'capabilities': [
                        {'id': 'devtools.trace.java.trace'}, {'id': 'devtools.trace.django.trace'}]}}
        updated = apply_manifest(manifest)
        self.assertEqual(updated['schemaVersion'], 2)
        self.assertEqual(updated['requiredProfiles'], {'tool.directory.v1': '=1.0.0-preview.1'})
        self.assertEqual(updated['contributes']['capabilities'][0]['directory']['categoryId'], 'code')
        missing = copy.deepcopy(manifest)
        missing['contributes']['capabilities'].append({'id': 'devtools.trace.unknown'})
        with self.assertRaises(AssertionError):
            apply_manifest(missing)

    def test_aliases_reject_nfkc_case_and_trim_collisions(self):
        with self.assertRaises(AssertionError):
            validate_directory({'categoryId': 'text', 'aliases': ['Ａ', 'a']})
        with self.assertRaises(AssertionError):
            validate_directory({'categoryId': 'text', 'aliases': ['alias\ncommand']})

    def test_utf8_budgets_are_measured_in_bytes(self):
        with self.assertRaises(AssertionError):
            validate_directory({'categoryId': 'text', 'description': '界' * 171})
        with self.assertRaises(AssertionError):
            validate_directory({'categoryId': 'text', 'aliases': ['中' * 43]})


if __name__ == '__main__':
    unittest.main()
