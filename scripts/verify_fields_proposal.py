"""Check only the draft multi-field schema, identity and UTF-8 budgets.

This is not an algorithm adapter, Host negotiation or runtime acceptance test.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path

from jsonschema import Draft202012Validator, ValidationError

ROOT = Path(__file__).resolve().parents[1]
FOLDER = ROOT / 'proposals/studio-fields/v0.1.0'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path)
args = parser.parse_args()
load = lambda path: json.loads(path.read_text(encoding='utf-8'))
proposal = load(FOLDER / 'proposal.json')
schema = load(FOLDER / 'request.schema.json')
assert proposal['status'] == 'provider_proposal_not_frozen_not_Host_supported'
assert proposal['contractVersion'] != '1.0.0-rc.1'
assert proposal['profile'] == 'compute.fields.v1'
assert schema['properties']['contractVersion']['const'] == proposal['contractVersion']
assert schema['properties']['pluginId']['const'] == proposal['packageId']
operations = {op['id']: op for op in proposal['operations']}
assert len(operations) == 4
catalog_versions = {tool['id']: tool['tool_version'] for tool in load(ROOT / 'docs/tools.json')['tools']}
assert all(op['sourceVersion'] == catalog_versions[op['sourceToolId']] for op in operations.values()), 'Proposal source tool versions are stale'
for branch in schema['oneOf']:
    op = operations[branch['properties']['capabilityId']['const']]
    assert branch['properties']['input'] == op['inputSchema']
    assert set(op['relayFields']) == set(op['utf8Limits']) == set(op['inputSchema']['required'])
Draft202012Validator.check_schema(schema)
validator = Draft202012Validator(schema)


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate field')
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError('non-JSON constant')


def valid(raw):
    if len(raw) > proposal['budget']['requestBytes']:
        return False
    try:
        request = json.loads(raw.decode('utf-8'), object_pairs_hook=unique,
                             parse_constant=reject_constant)
        validator.validate(request)
        if len(request['sceneId'].encode('utf-8')) > 128:
            return False
        op = operations[request['capabilityId']]
        sizes = {key: len(value.encode('utf-8')) for key, value in request['input'].items()}
        return all(size <= op['utf8Limits'][key] for key, size in sizes.items()) and sum(sizes.values()) <= proposal['budget']['totalInputUtf8Bytes']
    except (ValueError, UnicodeError, ValidationError, RecursionError):
        return False


rows = []
for case in load(FOLDER / 'fixtures.json')['cases']:
    raw = case['raw'].encode('utf-8') if 'raw' in case else json.dumps(case['request'], ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    actual = valid(raw)
    assert actual == case['valid'], case['id']
    rows.append(dict(id=case['id'], valid=actual, passed=True))
assert not valid(b'\xff')
result_schema = load(FOLDER / 'result.schema.json')
reverted = copy.deepcopy(result_schema)
reverted.pop('$comment')
reverted['title'] = 'compute.wasi.v1 tool result'
for branch in reverted['oneOf']:
    assert branch['properties']['contractVersion']['const'] == proposal['contractVersion']
    branch['properties']['contractVersion']['const'] = '1.0.0-rc.1'
assert reverted == load(ROOT / 'contracts/studio-devtools/v1/result.schema.json')
Draft202012Validator.check_schema(result_schema)
result_validator = Draft202012Validator(result_schema)
result_rows = []
for case in load(FOLDER / 'fixtures.json')['resultCases']:
    value = case['result']
    raw = json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    accepted = len(raw) <= proposal['budget']['resultBytes'] and result_validator.is_valid(value)
    assert accepted == case['valid'], case['id']
    result_rows.append(dict(id=case['id'], valid=accepted, passed=True))
proof = dict(status=proposal['status'], cases=rows, resultCases=result_rows, invalidUtf8Rejected=True,
             sourceFiles={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in FOLDER.glob('*.json')},
             limitations=['Schema/identity/byte checks only; no regex/diff algorithm, WASI, Host, View or Pi execution'])
if args.output:
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'PASS {len(rows)} request / {len(result_rows)} result draft vectors plus invalid UTF-8; proposal only, no runtime acceptance')
