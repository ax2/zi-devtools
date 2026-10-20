import copy
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from package_directory_revision import guard, validate_revision
from tool_directory import apply_manifest


class DirectoryRevisionTests(unittest.TestCase):
    def setUp(self):
        self.original = {'id': 'com.zicode.devtools.json-object', 'version': '0.1.0', 'schemaVersion': 2,
                         'runtime': {'wasi': 'runtime/tools.wasm'}, 'permissions': {'optional': ['pi.invoke']},
                         'requiredProfiles': {'output.schema.v1': '=1.0.0-rc.1', 'result.json.v1': '=1.0.0-preview.1'},
                         'contributes': {'capabilities': [{'id': 'devtools.json.object.inspect', 'inputSchema': {'type': 'object'}},
                                                         {'id': 'devtools.json.object.receive', 'inputSchema': {'type': 'object'}}]}}
        self.updated = copy.deepcopy(self.original)
        self.updated['version'] = '0.1.1'
        apply_manifest(self.updated)
        self.catalog = {'packageId': self.original['id'], 'packageVersion': '0.1.0', 'source': {'revision': 'original'},
                        'tools': [{'capabilityId': 'devtools.json.object.inspect', 'version': '1.3.0'}]}
        self.next_catalog = copy.deepcopy(self.catalog)
        self.next_catalog['packageVersion'] = '0.1.1'

    def validate(self):
        validate_revision(self.original, self.updated, self.catalog, self.next_catalog)

    def test_directory_only_change_is_accepted(self):
        self.validate()

    def test_rejects_runtime_and_permission_changes(self):
        for key, value in [('runtime', {'wasi': 'other.wasm'}), ('permissions', {'required': ['network']})]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.updated)
                changed[key] = value
                with self.assertRaises(ValueError):
                    validate_revision(self.original, changed, self.catalog, self.next_catalog)

    def test_rejects_input_schema_changes(self):
        self.updated['contributes']['capabilities'][0]['inputSchema']['type'] = 'string'
        with self.assertRaises(ValueError):
            self.validate()

    def test_rejects_dropped_output_profile(self):
        del self.updated['requiredProfiles']['output.schema.v1']
        with self.assertRaises(ValueError):
            self.validate()

    def test_rejects_unmapped_directory_text(self):
        self.updated['contributes']['capabilities'][0]['directory']['description'] = 'different'
        with self.assertRaises(ValueError):
            self.validate()

    def test_rejects_version_reuse_and_source_version_changes(self):
        self.updated['version'] = '0.1.0'
        with self.assertRaises(ValueError):
            self.validate()
        self.updated['version'] = '0.1.1'
        self.next_catalog['tools'][0]['version'] = '9.0.0'
        with self.assertRaises(ValueError):
            self.validate()

    def test_guard_rejects_parent_escape(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.assertEqual(guard(root / 'candidate', root), root / 'candidate')
            with self.assertRaises(ValueError):
                guard(root / '..' / 'outside', root)


if __name__ == '__main__':
    unittest.main()
