"""Verify vendored Studio directory metadata and current provider manifests."""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
from tool_directory import CATEGORIES, PROFILE, PROFILE_VERSION, SNAPSHOT, load_capabilities, validate_directory

parser = argparse.ArgumentParser()
parser.add_argument('manifests', nargs='*', type=Path)
args = parser.parse_args()

schema = json.loads((SNAPSHOT / 'schema.json').read_text(encoding='utf-8'))
assert set(schema['properties']['categoryId']['enum']) == CATEGORIES
snapshot = json.loads((SNAPSHOT / 'capabilities.json').read_text(encoding='utf-8'))
assert snapshot['profile'] == PROFILE and snapshot['profileVersion'] == PROFILE_VERSION
mapping = load_capabilities()
for capability_id, directory in mapping.items():
    validate_directory(directory)
    assert len(directory.get('aliases', [])) >= 2, f'{capability_id} needs search aliases in supported languages'
assert len(mapping) == len(set(mapping))

for path in args.manifests:
    manifest = json.loads(path.read_text(encoding='utf-8'))
    assert manifest['schemaVersion'] == 2, f'{path}: schemaVersion must be 2'
    assert manifest['requiredProfiles'].get(PROFILE) == f'={PROFILE_VERSION}', f'{path}: wrong required profile'
    capabilities = manifest['contributes']['capabilities']
    ids = {item['id'] for item in capabilities}
    assert len(ids) == len(capabilities), f'{path}: duplicate capability IDs'
    assert ids <= set(mapping), f'{path}: capability missing from vendored mapping'
    for capability in capabilities:
        expected = mapping[capability['id']]
        validate_directory(capability.get('directory'))
        assert capability['directory'] == expected, f'{path}: stale or incorrect metadata for {capability["id"]}'

print(f'Valid directory profile {PROFILE}@{PROFILE_VERSION}: {len(mapping)} mapped capabilities')
