"""Shared validation and manifest helpers for the vendored Studio directory profile."""
import json
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SNAPSHOT = ROOT / 'contracts/studio-tool-directory/1.0.0-preview.1'
PROFILE = 'tool.directory.v1'
PROFILE_VERSION = '1.0.0-preview.1'
CATEGORIES = {'text', 'data', 'web', 'code', 'files', 'media', 'system', 'ai', 'knowledge', 'automation', 'security', 'other'}


def load_capabilities():
    return json.loads((SNAPSHOT / 'capabilities.json').read_text(encoding='utf-8'))['capabilities']


def _normalized(value):
    return unicodedata.normalize('NFKC', value).strip().lower()


def validate_directory(directory):
    assert isinstance(directory, dict) and set(directory) <= {'categoryId', 'aliases', 'description'}
    assert directory.get('categoryId') in CATEGORIES
    aliases = directory.get('aliases', [])
    assert isinstance(aliases, list) and len(aliases) <= 16
    seen = set()
    for alias in aliases:
        assert isinstance(alias, str) and alias.strip() and len(alias.encode('utf-8')) <= 128
        assert not any(unicodedata.category(ch) == 'Cc' for ch in alias)
        key = _normalized(alias)
        assert key not in seen, 'Directory aliases collide after NFKC/lowercase/trim'
        seen.add(key)
    if 'description' in directory:
        description = directory['description']
        assert isinstance(description, str) and description.strip() and len(description.encode('utf-8')) <= 512
        assert not any(unicodedata.category(ch) == 'Cc' for ch in description)
    return True


def apply_manifest(manifest):
    metadata = load_capabilities()
    capabilities = manifest['contributes']['capabilities']
    ids = {item['id'] for item in capabilities}
    assert len(ids) == len(capabilities), 'Duplicate capability IDs'
    assert ids, 'Candidate must declare capabilities'
    provider_prefix = '.'.join(next(iter(ids)).split('.')[:2]) + '.'
    expected = {capability_id for capability_id in metadata if capability_id.startswith(provider_prefix)}
    assert ids == expected, 'Directory metadata must exactly match the candidate provider capability set'
    manifest['schemaVersion'] = 2
    profiles = dict(manifest.get('requiredProfiles', {}))
    assert profiles.get(PROFILE, '=' + PROFILE_VERSION) == '=' + PROFILE_VERSION, 'Conflicting directory profile version'
    profiles[PROFILE] = '=' + PROFILE_VERSION
    manifest['requiredProfiles'] = profiles
    for capability in capabilities:
        directory = metadata[capability['id']]
        validate_directory(directory)
        capability['directory'] = directory.copy()
    return manifest


def directory_snapshot(manifest):
    return {
        'profile': PROFILE,
        'profileVersion': PROFILE_VERSION,
        'packageId': manifest['id'],
        'packageVersion': manifest['version'],
        'capabilities': [
            {'id': item['id'], **item['directory']}
            for item in manifest['contributes']['capabilities']
        ],
    }
