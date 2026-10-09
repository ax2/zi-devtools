"""Generate manifest, catalog and View operation metadata from the Rust registry."""
import argparse
import copy
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
source = (ROOT / 'crates/zi-text-core/src/transforms.rs').read_text(encoding='utf-8')
matches = re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"\s*,\s*source_tool_id:\s*"([^"]+)"\s*,\s*title:\s*"([^"]+)"\s*,\s*group:\s*"([^"]+)"', source)
actions = [dict(id=id_, sourceToolId=tool, group=group, title=title) for id_, tool, title, group in matches]
assert len(actions) == 23 and len({a['id'] for a in actions}) == len(actions)
versions = {t['id']: t['tool_version'] for t in json.loads((ROOT / 'docs/tools.json').read_text(encoding='utf-8'))['tools']}
manifest = json.loads((ROOT / 'contracts/studio-devtools/v1/plugin.example.json').read_text(encoding='utf-8'))
manifest.update(id='com.zicode.devtools.transforms', name='ZiDevTools Transforms', displayName='ZiDevTools 文本转换',
                description='Grouped bounded text transformations; no host resources, files or network.', version='0.1.0')
manifest['activationEvents'] = ['onView:devtools.transforms.main'] + ['onCapability:devtools.transforms.' + a['id'] for a in actions]
schema = dict(type='object', properties=dict(text=dict(type='string', maxLength=8192)), required=['text'], additionalProperties=False)
manifest['contributes'] = dict(
    activities=[dict(id='devtools.transforms', title='文本转换', order=851)],
    views=[dict(id='devtools.transforms.main', activity='devtools.transforms', location='main', title='文本转换', entry='views/main.html')],
    capabilities=[dict(id='devtools.transforms.' + a['id'], title=a['title'], inputSchema=copy.deepcopy(schema)) for a in actions],
    piTools=[dict(name='devtools_transforms_' + a['id'].replace('.', '_'), title=a['title'],
                  description=f"Run bounded local {a['title']} on explicitly supplied UTF-8 text, at most 8192 bytes. No file or network access.",
                  capability='devtools.transforms.' + a['id'], inputSchema=copy.deepcopy(schema)) for a in actions])
catalog = dict(contract='zicode.devtools-plugin/1.0.0-rc.1', packageId=manifest['id'], packageVersion=manifest['version'],
               source=dict(revision='REPLACE_WITH_PROVIDER_SOURCE_REVISION', dirty=True),
               tools=[dict(sourceToolId=a['sourceToolId'], toolId='devtools.transforms.' + a['id'],
                           capabilityId='devtools.transforms.' + a['id'], version=versions[a['sourceToolId']],
                           status='candidate', profile='compute.wasi.v1', title=a['title']) for a in actions])
outputs = {ROOT / 'plugins/transforms/plugin.json': json.dumps(manifest, ensure_ascii=False, indent=2) + '\n',
           ROOT / 'plugins/transforms/catalog.json': json.dumps(catalog, ensure_ascii=False, indent=2) + '\n'}
view = ROOT / 'plugins/transforms/views/main.html'
html = view.read_text(encoding='utf-8')
updated, count = re.subn(r'const actions=\[.*?\], samples=', lambda _: 'const actions=' + json.dumps(actions, ensure_ascii=False) + ', samples=', html, count=1)
assert count == 1, 'View registry marker missing'
outputs[view] = updated
for path, content in outputs.items():
    if args.check:
        assert path.read_text(encoding='utf-8') == content, f'Run sync_transforms_plugin.py: {path}'
    else:
        path.write_text(content, encoding='utf-8')
assert {a['sourceToolId'] for a in actions} <= versions.keys()
print(f'Transforms registry: {len(actions)} operations, {len({a["sourceToolId"] for a in actions})} source tools; manifest/View/catalog/Pi names synchronized')
