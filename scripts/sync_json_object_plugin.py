"""Synchronize JSON Object directory metadata without changing runtime or grants."""
import argparse
import json
from pathlib import Path

from tool_directory import apply_manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    plugin = root / 'plugins/json-object'
    manifest = json.loads((plugin / 'plugin.json').read_text(encoding='utf-8'))
    assert manifest['id'] == 'com.zicode.devtools.json-object'
    manifest['version'] = '0.1.1'
    apply_manifest(manifest)
    catalog = json.loads((plugin / 'catalog.json').read_text(encoding='utf-8'))
    catalog['packageVersion'] = manifest['version']
    tools = json.loads((root / 'docs/tools.json').read_text(encoding='utf-8'))['tools']
    version = next(tool['tool_version'] for tool in tools if tool['id'] == 'json')
    for tool in catalog['tools']:
        assert tool['sourceToolId'] == 'json'
        tool['version'] = version
    for name, value in [('plugin.json', manifest), ('catalog.json', catalog)]:
        path = plugin / name
        expected = json.dumps(value, ensure_ascii=False, indent=2) + '\n'
        actual = path.read_text(encoding='utf-8')
        if args.check:
            assert actual == expected, f'Run sync_json_object_plugin.py: {path}'
        elif actual != expected:
            path.write_text(expected, encoding='utf-8', newline='\n')
    print('JSON Object: 2 directory entries; output/result profiles and optional grants retained')


if __name__ == '__main__':
    main()
