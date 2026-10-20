"""Freeze a directory-only revision while retaining the exact verified runtime."""
import argparse
import copy
import json
import os
from pathlib import Path
import re

from package_studio_catalog_revision import ENCODING, ROOT, directory_files, json_bytes, sha256, verify_package
from tool_directory import PROFILE, PROFILE_VERSION, SNAPSHOT, apply_manifest, directory_snapshot


def guard(path, root):
    path, root = path.absolute(), root.absolute()
    if not path.is_relative_to(root):
        raise ValueError('Path escapes project release directory')
    for item in [path, *path.parents]:
        if item.is_symlink() or item.is_junction():
            raise ValueError('Linked paths are not allowed')
    if not path.resolve().is_relative_to(root.resolve()):
        raise ValueError('Resolved path escapes project release directory')
    return path


def guard_tree(path):
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in dirs + files:
            item = Path(base) / name
            if item.is_symlink() or item.is_junction():
                raise ValueError('Linked package entries are not allowed')


def validate_revision(original, updated, original_catalog, current_catalog):
    before, after = copy.deepcopy(original), copy.deepcopy(updated)
    versions = [value.get('version') for value in [before, after]]
    if not all(isinstance(v, str) and re.fullmatch(r'\d+\.\d+\.\d+', v) for v in versions):
        raise ValueError('Explicit release versions required')
    if tuple(map(int, versions[1].split('.'))) <= tuple(map(int, versions[0].split('.'))):
        raise ValueError('Directory revision must increase package version')
    expected = apply_manifest(copy.deepcopy(updated))
    if expected != updated:
        raise ValueError('Directory metadata does not match its declared mapping')
    if after.get('schemaVersion') != 2 or before.get('schemaVersion') not in (1, 2):
        raise ValueError('Invalid manifest schema revision')
    if after.get('requiredProfiles') != {**before.get('requiredProfiles', {}), PROFILE: '=' + PROFILE_VERSION}:
        raise ValueError('Only directory negotiation may be added')
    for manifest in [before, after]:
        manifest.pop('version')
        manifest.pop('schemaVersion')
        manifest.pop('requiredProfiles', None)
        for capability in manifest['contributes']['capabilities']:
            capability.pop('directory', None)
    if before != after:
        raise ValueError('Runtime, routing, schemas, permissions or other metadata changed')
    old_catalog, new_catalog = copy.deepcopy(original_catalog), copy.deepcopy(current_catalog)
    if old_catalog['packageVersion'] != versions[0] or new_catalog['packageVersion'] != versions[1]:
        raise ValueError('Catalog/package version mismatch')
    for catalog in [old_catalog, new_catalog]:
        catalog.pop('packageVersion')
        catalog.pop('source', None)
    if old_catalog != new_catalog:
        raise ValueError('Directory revision cannot change source tool versions or operation mappings')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plugin', choices=['transforms', 'json-object'], required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--receipt', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    release = ROOT / 'release'
    baseline = guard(ROOT / args.baseline, release)
    receipt_path = guard(ROOT / args.receipt, release)
    output = guard(ROOT / args.output, release / 'plugin-candidates')
    external = output.with_name(output.name + '-verification.json')
    if output.exists() or external.exists():
        raise ValueError('Never overwrite a frozen candidate or receipt')
    guard_tree(baseline)
    baseline_verified = verify_package(baseline.relative_to(ROOT).as_posix(), receipt_path.relative_to(ROOT).as_posix())
    if baseline_verified['directoryIndexStatus'] != 'matched':
        raise ValueError('Baseline needs a verified explicit directory-index encoding')
    receipt_bytes = receipt_path.read_bytes()
    previous = json.loads(receipt_bytes)
    source = ROOT / 'plugins' / args.plugin
    original = json.loads((baseline / 'plugin.json').read_bytes())
    current = json.loads((source / 'plugin.json').read_bytes())
    original_catalog = json.loads((baseline / 'catalog.json').read_bytes())
    source_catalog = json.loads((source / 'catalog.json').read_bytes())
    validate_revision(original, current, original_catalog, source_catalog)
    catalog = copy.deepcopy(original_catalog)
    catalog['packageVersion'] = current['version']
    payload = {row['path']: (baseline / row['path']).read_bytes() for row in directory_files(baseline)}
    payload['plugin.json'] = (source / 'plugin.json').read_bytes()
    payload['catalog.json'] = json_bytes(catalog, pretty=True)
    if args.plugin == 'json-object':
        # The old verifier required an exact pre-directory profile dictionary.
        # Only the replay harness changes; WASI, requests and expected bytes stay fixed.
        payload['verification/verify_json_object_wasi.py'] = (ROOT / 'scripts/verify_json_object_wasi.py').read_bytes()
    payload['README.md'] = (f"# {current['name']} {current['version']} directory revision\n\n"
        f"Adds {PROFILE}@{PROFILE_VERSION} category, aliases and descriptions. Runtime, View, permissions, input/output schemas and source tool versions are byte/semantic-equivalent to {original['version']}. "
        "No Host setting or grant is enabled. Unsigned provider candidate; not Host acceptance.\n\n"
        "The original runtime evidence is retained unchanged. Its old manifest/catalog references resolve against verification/directory-baseline, not the new metadata. "
        "See verification/directory-revision.json for exact changed-file hashes and baseline receipt.\n\n"
        "## Original runtime documentation\n\n").encode() + payload['README.md']
    added = {
        'verification/directory-baseline/plugin.json': (baseline / 'plugin.json').read_bytes(),
        'verification/directory-baseline/catalog.json': (baseline / 'catalog.json').read_bytes(),
        'verification/directory-baseline/receipt.json': receipt_bytes,
        'verification/tool-directory-snapshot.json': json_bytes(directory_snapshot(current), pretty=True),
        'contracts/tool-directory/schema.json': (SNAPSHOT / 'schema.json').read_bytes(),
    }
    if any(name in payload for name in [*added, 'verification/directory-revision.json']):
        raise ValueError('Baseline already contains revision evidence; use its original runtime base')
    changed = [dict(path=name, beforeSha256=sha256((baseline / name).read_bytes()), afterSha256=sha256(data))
               for name, data in payload.items() if data != (baseline / name).read_bytes()]
    if {row['path'] for row in changed} - {'plugin.json', 'catalog.json', 'README.md', 'verification/verify_json_object_wasi.py'}:
        raise ValueError('Unexpected runtime file mutation')
    revision = dict(kind='directory-metadata-only', baseline=baseline_verified, changedFiles=changed,
                    runtimeFilesChanged=False, permissionsChanged=False, hostAccepted=False,
                    manifestSha256=sha256(payload['plugin.json']), catalogSha256=sha256(payload['catalog.json']),
                    metadataSources=[dict(path=p.relative_to(ROOT).as_posix(), sha256=sha256(p.read_bytes())) for p in [
                        source / 'plugin.json', source / 'catalog.json', SNAPSHOT / 'capabilities.json',
                        Path(__file__), ROOT / 'scripts/tool_directory.py', ROOT / 'scripts/verify_json_object_wasi.py']])
    added['verification/directory-revision.json'] = json_bytes(revision, pretty=True)
    payload.update(added)
    output.mkdir()
    for name, contents in payload.items():
        target = guard(output / name, output)
        target.parent.mkdir(parents=True, exist_ok=True)
        with target.open('xb') as stream:
            stream.write(contents)
    files = directory_files(output)
    receipt = copy.deepcopy(previous)
    for key in ['receiptRevision', 'revisesReceipt']:
        receipt.pop(key, None)
    receipt.update(packageId=current['id'], packageVersion=current['version'], requiredProfiles=current['requiredProfiles'],
                   files=files, directoryIndexSha256=sha256(json_bytes(files)), directoryIndexEncoding=ENCODING,
                   metadataRevision=revision, sourceFilesScope='Inherited original runtime source snapshot; metadata sources recorded separately',
                   packageSha256=None, status='unsigned directory metadata revision; not Host-accepted')
    with external.open('xb') as stream:
        stream.write(json_bytes(receipt, pretty=True))
    verified = verify_package(output.relative_to(ROOT).as_posix(), external.relative_to(ROOT).as_posix())
    print(json.dumps(verified, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
