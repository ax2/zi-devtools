import copy
import json
import sys
from pathlib import Path
import unittest
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from package_image_resize_revision import validate_revision


class ImageResizeInputTests(unittest.TestCase):
    def setUp(self):
        self.schema = json.loads((ROOT / 'plugins/image-resize/schemas/resize-input.schema.json').read_text(encoding='utf-8'))
        self.validate = Draft202012Validator(self.schema).is_valid

    def test_numeric_width_matches_adapter_range(self):
        for value in range(10000):
            self.assertEqual(self.validate({'maxWidth': value, 'format': 'webp', 'quality': 82}),
                             1 <= value <= 4096, value)

    def test_quality_and_lexical_rejections(self):
        for value in range(100):
            self.assertEqual(self.validate({'maxWidth': 1600, 'format': 'jpeg', 'quality': value}), 35 <= value <= 95)
        for invalid in ['1\n', ' 1', '+1', '-1', '１', '1.0', '', '00001', '16', -1, True, None, 1.5]:
            self.assertFalse(self.validate({'maxWidth': invalid, 'format': 'png', 'quality': 82}), repr(invalid))

    def test_manifest_schema_and_explicit_resource_filter(self):
        manifest = json.loads((ROOT / 'plugins/image-resize/plugin.json').read_text(encoding='utf-8'))
        capability = manifest['contributes']['capabilities'][0]
        self.assertEqual(capability['inputSchema'], self.schema)
        self.assertEqual(capability['resourceInputMimeTypes'], ['image/png', 'image/jpeg', 'image/webp'])
        self.assertEqual(manifest['requiredProfiles']['resource.input.types.v1'], '=1.0.0-preview.1')
        self.assertEqual(manifest['requiredProfiles']['tool.directory.v1'], '=1.0.0-preview.1')

    def test_input_declaration_uses_negotiated_host_subset(self):
        # Input and output have separate Host validators. Full JSON Schema validity
        # does not imply admission by the bounded input-schema implementation.
        supported = {'$schema', '$id', 'title', 'description', 'default', 'type',
                     'properties', 'required', 'additionalProperties', 'items',
                     'minItems', 'maxItems', 'minProperties', 'maxProperties',
                     'minLength', 'maxLength', 'minimum', 'maximum', 'enum', 'const'}
        def check(schema):
            self.assertFalse(set(schema) - supported)
            for child in schema.get('properties', {}).values():
                check(child)
            if 'items' in schema:
                check(schema['items'])
        check(self.schema)

    def test_candidate_revision_keeps_output_and_permissions(self):
        before = json.loads((ROOT/'release/plugin-candidates/image-resize-0.1.1-candidate-02/plugin.json').read_bytes())
        after = json.loads((ROOT/'plugins/image-resize/plugin.json').read_bytes())
        validate_revision(before, after)
        for field in ('permissions', 'runtime'):
            changed = copy.deepcopy(after)
            changed[field] = {}
            with self.assertRaises(ValueError):
                validate_revision(before, changed)
        changed = copy.deepcopy(after)
        changed['contributes']['capabilities'][0]['inputSchema']['properties']['maxWidth']['maximum'] = 99999
        with self.assertRaises(ValueError):
            validate_revision(before, changed)
        changed = copy.deepcopy(after)
        changed['contributes']['capabilities'][0]['outputSchema'] = {'type':'object'}
        with self.assertRaises(ValueError):
            validate_revision(before, changed)


if __name__ == '__main__':
    unittest.main()
