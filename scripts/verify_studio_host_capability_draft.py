from pathlib import Path
import json
import hashlib
import re

ROOT = Path(__file__).resolve().parents[1]
DRAFT = ROOT / 'contracts' / 'studio-host-capabilities' / 'draft-20261014'
SCHEMAS = DRAFT / 'schemas'
EXAMPLES = DRAFT / 'examples'

def load(path):
    return json.loads(path.read_text(encoding='utf-8'))

def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)

service = load(SCHEMAS / 'service-request.schema.json')
desktop = load(SCHEMAS / 'desktop-request.schema.json')
service_event = load(SCHEMAS / 'service-event.schema.json')
service_response = load(SCHEMAS / 'service-response.schema.json')
desktop_event = load(SCHEMAS / 'desktop-event.schema.json')
for schema in (service, desktop, service_event, service_response, desktop_event):
    assert schema['$schema'] == 'https://json-schema.org/draft/2020-12/schema'
    assert schema['$id'].endswith('-draft-2026-10-14.schema.json')
    assert len(schema['oneOf']) >= 3

# Identity, scene, request ID and local-path access belong to Host and cannot be
# supplied as provider-owned fields in these bridge bodies.
for schema in (service, desktop):
    root_properties = schema.get('properties', {})
    assert not {'pluginId', 'sceneId', 'requestId', 'path', 'absolutePath'} & set(root_properties)
    branches = schema['oneOf']
    operations = [branch['properties']['operation']['const'] for branch in branches]
    assert len(operations) == len(set(operations))
    assert all(branch.get('additionalProperties') is False for branch in branches)

for schema in (service, service_event, desktop, desktop_event):
    for node in walk(schema):
        assert 'path' not in node.get('properties', {})
        assert 'absolutePath' not in node.get('properties', {})

vectors = load(DRAFT / 'vectors.json')
assert vectors['status'] == 'proposal-only'
assert 'RFC 8785 JCS' in vectors['canonicalization']['mcpArgumentDigest']
assert 'cross-language JCS conformance remains a Studio Host test' in vectors['canonicalization']['fixtureNote']
assert len(vectors['accept']) == 5 and len(vectors['reject']) == 1
assert len(vectors['responseExamples']) == 4
assert any('four MCP calls' in requirement for requirement in vectors['hostRuntimeRequirements'])

# JSON Schema maxLength counts Unicode characters, so independently verify the
# Host's specified UTF-8 byte guards and trusted, one-shot UI-token bindings.
model_input = next(b for b in service['oneOf'] if b['properties']['operation']['const'] == 'model.generate')['properties']['input']['properties']
assert model_input['goal']['x-host-utf8-bytes-maximum'] == 2048
assert model_input['materials']['x-host-utf8-aggregate-bytes-maximum'] == 32768
gesture_schema = next(b for b in desktop['oneOf'] if b['properties']['operation']['const'] == 'clipboard.subscribe')['properties']['input']['properties']['userGestureId']
assert gesture_schema['x-host-trusted-one-shot-binding'] == ['pluginId', 'sceneId', 'operation', 'permissionGeneration']
consent_schema = model_input['consentId']
assert 'provider-supplied string is not consent' in consent_schema['description']
mcp_input = next(b for b in service['oneOf'] if b['properties']['operation']['const'] == 'mcp.tool.call')['properties']['input']['properties']
assert mcp_input['consentId']['x-host-consent-binding'] == [
    'pluginId', 'sceneId', 'operation', 'permissionGeneration', 'runId',
    'serverRef', 'toolName', 'toolSchemaSha256', 'argumentsDigest'
]
security = {v['id']: v for v in vectors['hostSecurityVectors']}
assert len(security) == len(vectors['hostSecurityVectors'])
goal_vector = security['goal-utf8-byte-limit']
goal = goal_vector['providerRequest']['input']['goal']
assert len(goal) <= model_input['goal']['maxLength'] and len(goal.encode('utf-8')) > model_input['goal']['x-host-utf8-bytes-maximum']
assert goal_vector['providerRequest']['input']['consentId']
assert goal_vector['expected'] == 'reject'
materials_vector = security['materials-aggregate-utf8-byte-limit']
materials = materials_vector['providerRequest']['input']['materials']
assert all(len(item['text']) <= 32768 for item in materials)
assert sum(len(item['text'].encode('utf-8')) for item in materials) == 32769
assert materials_vector['providerRequest']['input']['consentId']
assert materials_vector['expected'] == 'reject'
required_security_vectors = {
    'model-consent-valid', 'model-consent-provider-chosen', 'model-consent-replay',
    'model-consent-wrong-permission-generation', 'model-consent-wrong-plugin',
    'model-consent-wrong-scene', 'mcp-consent-valid', 'mcp-consent-provider-chosen',
    'mcp-consent-wrong-arguments', 'clipboard-gesture-valid',
    'clipboard-gesture-provider-chosen', 'clipboard-gesture-replay',
    'clipboard-gesture-wrong-operation'
}
assert required_security_vectors <= set(security)
for vector in security.values():
    if vector['kind'] != 'one-shot-user-authorization':
        continue
    body = vector['providerRequest']
    host = vector['hostContext']
    grant = vector['authorization']
    token_field = 'userGestureId' if body['operation'] == 'clipboard.subscribe' else 'consentId'
    token = body['input'][token_field]
    binding_matches = grant['binding'] == host and host['operation'] == body['operation']
    if body['operation'] == 'mcp.tool.call':
        canonical_arguments = json.dumps(body['input']['arguments'], ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode('utf-8')
        expected_digest = 'sha256:' + hashlib.sha256(canonical_arguments).hexdigest()
        assert host['argumentsDigest'] == expected_digest
    usable = (grant['issuer'] == 'studio_host' and grant['trustedInteraction'] is True
              and grant['oneShot'] is True and grant['consumed'] is False
              and grant['tokenId'] == token and binding_matches)
    assert (vector['expected'] == 'accept') == usable, vector['id']

for filename in vectors['accept'] + vectors['reject']:
    example = load(EXAMPLES / filename)
    if 'operation' in example:
        schema = service if example['operation'].startswith(('models.', 'model.', 'mcp.', 'task.')) else desktop
        branch = next(b for b in schema['oneOf'] if b['properties']['operation']['const'] == example['operation'])
        required = branch['properties']['input']['required']
        assert set(required) <= set(example['input'])
        if example['operation'] == 'model.generate':
            assert len(example['input']['goal'].encode('utf-8')) <= 2048
            assert len(example['input']['materials']) <= 32
        if example['operation'] == 'mcp.tool.call' and filename in vectors['accept']:
            assert example['input']['consentId'] != ''
    else:
        branch = next(b for b in desktop_event['oneOf'] if b['properties']['type']['const'] == example['type'])
        assert set(branch['required']) <= set(example)
    assert not any(re.search(r'(^|_)(absolute_)?path$', key, re.I) for node in walk(example) for key in node)

for filename in vectors['responseExamples']:
    example = load(EXAMPLES / filename)
    branch = next(b for b in service_response['oneOf']
                  if (b['properties']['operation'].get('const') == example['operation']
                      or example['operation'] in b['properties']['operation'].get('enum', [])
                      or b['properties']['operation'].get('type') == 'string')
                  and set(b['required']) <= set(example))
    assert set(branch['required']) <= set(example)
    if example['operation'] == 'mcp.tools.list':
        assert all('effectClass' in tool for tool in example['result']['tools'])
    assert not any(re.search(r'(^|_)(absolute_)?path$', key, re.I) for node in walk(example) for key in node)

negative = load(EXAMPLES / 'service-denied-no-consent.json')
consent_schema = next(b for b in service['oneOf'] if b['properties']['operation']['const'] == 'mcp.tool.call')['properties']['input']['properties']['consentId']
assert consent_schema['minLength'] == 1 and negative['input']['consentId'] == ''
print(f"Draft guard OK: 5 JSON Schemas parsed, {len(vectors['accept']) + len(vectors['reject'])} request/event examples, {len(vectors['responseExamples'])} response examples and {len(security)} UTF-8/token security vectors are structurally consistent. This does not claim JSON Schema engine or Studio Host conformance.")
