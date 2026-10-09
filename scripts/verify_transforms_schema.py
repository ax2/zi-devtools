"""Validate candidate metadata and raw expectations with actual JSON Schema.

Uses jsonschema only in the local verification cache, never product runtime.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

import jsonschema
from referencing import Registry, Resource
from plugin_proof import validate_compute_metadata

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--fixtures', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--plugin', choices=['transforms', 'inspect', 'trace'], default='transforms')
args = parser.parse_args()
snapshot = ROOT / 'contracts/studio-devtools/v1'
plugin = ROOT / 'plugins' / args.plugin
manifest = json.loads((plugin / 'plugin.json').read_text(encoding='utf-8'))
digest = lambda data: hashlib.sha256(data).hexdigest()
load = lambda path: json.loads(path.read_text(encoding='utf-8'))
base = load(snapshot / 'request.schema.json')
validate_compute_metadata(manifest, load(plugin / 'catalog.json'), load(snapshot / 'plugin.example.json'))
request = load(plugin / 'request.schema.json')
reverted = copy.deepcopy(request)
assert reverted.pop('$comment').startswith('Derived from frozen rc.1')
assert reverted['properties']['pluginId']['const'] == manifest['id']
reverted['properties']['pluginId']['const'] = 'com.zicode.devtools.text'
assert reverted == base, 'Only the manifest ID binding may differ from frozen request schema'
registry = Registry().with_resources((p.name, Resource.from_contents(load(p))) for p in snapshot.glob('*.schema.json'))
validators = {name: jsonschema.Draft202012Validator(load(snapshot / (name + '.schema.json')), registry=registry)
              for name in ['input', 'result', 'delivery']}
validators['request'] = jsonschema.Draft202012Validator(request, registry=registry)
for validator in validators.values():
    validator.check_schema(validator.schema)
validators['delivery'].validate(load(plugin / 'catalog.json'))
proof = load(args.fixtures / 'fuel-proof.json')
valid = 0
for case in proof['cases']:
    raw = (args.fixtures / case['expectedFile']).read_bytes()
    assert digest(raw) == case['expectedSha256']
    validators['result'].validate(json.loads(raw))
    raw_request = (args.fixtures / case['requestFile']).read_bytes()
    assert digest(raw_request) == case['requestSha256']
    if case['expectedError'] is None:
        data = json.loads(raw_request)
        validators['request'].validate(data)
        validators['input'].validate(data['input'])
        valid += 1
result = dict(wasmSha256=proof['wasmSha256'], requestSchemaSha256=digest((plugin / 'request.schema.json').read_bytes()),
              catalogSha256=digest((plugin / 'catalog.json').read_bytes()),
              fixtureProofSha256=digest((args.fixtures / 'fuel-proof.json').read_bytes()),
              contractIndexSha256=digest((snapshot / 'SHA256SUMS.json').read_bytes()),
              resultCases=len(proof['cases']), successfulRequestCases=valid, catalogOperations=len(manifest['contributes']['capabilities']),
              binding='Only pluginId.const specialized; frozen snapshot preserved',
              limitations=['Provider schema validation; not production Host acceptance'])
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'PASS delivery catalog and {len(proof["cases"])} result schemas; {valid} successful request/input schemas; only plugin ID binding differs')
