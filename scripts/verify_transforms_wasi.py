"""Check all registered transforms against independent vectors and real adapters.

Exports exact request/result bytes for independent fuel-budget replay. Node parity
is functional evidence only, never a Studio Host or sandbox acceptance claim.
"""
import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--native', type=Path, required=True)
parser.add_argument('--output-dir', type=Path, required=True)
parser.add_argument('--plugin', choices=['transforms', 'inspect'], default='transforms')
args = parser.parse_args()
module = args.wasm.read_bytes()
assert module[:8] == b'\0asm\x01\0\0\0' and len(module) <= 2 * 1024 * 1024


def leb(data, offset):
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
        shift += 7
        assert shift <= 35


offset = 8
memory_max = None
while offset < len(module):
    section = module[offset]
    size, start = leb(module, offset + 1)
    offset = start + size
    if section == 5:
        count, cursor = leb(module, start)
        flags, cursor = leb(module, cursor)
        minimum, cursor = leb(module, cursor)
        memory_max, cursor = leb(module, cursor)
        assert count == flags == 1 and minimum <= memory_max == 1024
assert memory_max == 1024


fixture_path = ROOT / 'plugins' / args.plugin / 'fixtures.json'
fixtures = json.loads(fixture_path.read_text(encoding='utf-8'))['cases']
plugin_id = 'com.zicode.devtools.' + args.plugin
sample = next(c for c in fixtures if c['expectedError'] is None)


def request(capability=None, value=None, **changes):
    data = dict(pluginId=plugin_id, sceneId='coding',
                capabilityId=sample['capabilityId'] if capability is None else capability, commandId=None,
                input=sample['input'] if value is None else value)
    data.update(changes)
    return data


cases = [(c['id'], request(c['capabilityId'], c['input']),
          c['expectedText'], c['expectedError']) for c in fixtures]
for field in ['pluginId', 'sceneId', 'capabilityId', 'commandId', 'input']:
    value = request()
    del value[field]
    cases.append(('missing-' + field, value, None, 'INVALID_INPUT'))
for name, changes, error in [
    ('unknown-outer-field', {'extra': True}, 'INVALID_INPUT'),
    ('unknown-input-field', {'input': {'text': 'x', 'extra': True}}, 'INVALID_INPUT'),
    ('unknown-operation', {'capabilityId': 'devtools.' + args.plugin + '.missing'}, 'UNSUPPORTED_OPERATION'),
    ('foreign-text-capability', {'capabilityId': 'devtools.text.sha256'}, 'UNSUPPORTED_OPERATION'),
    ('foreign-plugin', {'pluginId': 'other'}, 'INVALID_INPUT'),
    ('old-plugin-new-capability', {'pluginId': 'com.zicode.devtools.text'}, 'UNSUPPORTED_OPERATION' if args.plugin == 'transforms' else 'INVALID_INPUT'),
    ('scene-empty', {'sceneId': ''}, 'INVALID_INPUT'),
    ('scene-over', {'sceneId': 'a' * 129}, 'INVALID_INPUT'),
    ('command-not-null', {'commandId': 'anything'}, 'INVALID_INPUT'),
]:
    cases.append((name, request(**changes), None, error))
cases.append(('scene-exact-128', request(sceneId='a' * 128), sample['expectedText'], None))
cases.extend([
    ('invalid-request-utf8', b'\xff', None, 'INVALID_INPUT'),
    ('request-over-limit', b' ' * (49152 + 1), None, 'INPUT_TOO_LARGE'),
    ('duplicate-input-key', ('{"pluginId":' + json.dumps(plugin_id) + ',"sceneId":"coding","capabilityId":' + json.dumps(sample['capabilityId']) + ',"commandId":null,"input":{"text":"secret-test","text":"other"}}').encode(), None, 'INVALID_INPUT'),
    ('duplicate-outer-key', ('{"pluginId":' + json.dumps(plugin_id) + ',"pluginId":"other","sceneId":"coding","capabilityId":' + json.dumps(sample['capabilityId']) + ',"commandId":null,"input":{"text":"secret-test"}}').encode(), None, 'INVALID_INPUT'),
])
manifest = json.loads((ROOT / 'plugins' / args.plugin / 'plugin.json').read_text(encoding='utf-8'))
registered = {c['id'] for c in manifest['contributes']['capabilities']}
assert registered == {c['capabilityId'] for c in fixtures}
assert len({c[0] for c in cases}) == len(cases)
args.output_dir.mkdir(parents=True, exist_ok=True)
rows = []
for index, (name, value, text, error) in enumerate(cases):
    data = value if isinstance(value, bytes) else json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()
    start = time.monotonic()
    flags = subprocess.CREATE_NO_WINDOW if hasattr(subprocess, 'CREATE_NO_WINDOW') else 0
    native = subprocess.run([str(args.native.resolve())], input=data, capture_output=True,
                            timeout=5, check=True, creationflags=flags)
    wasi = subprocess.run(['node', '--disable-warning=ExperimentalWarning',
                           str(ROOT / 'scripts/run_text_wasi.mjs'), str(args.wasm.resolve())],
                          input=data, capture_output=True, timeout=5, check=True, creationflags=flags)
    assert native.stdout == wasi.stdout, name + ': byte parity'
    assert not native.stderr and len(wasi.stdout) <= 49152, name
    result = json.loads(wasi.stdout)
    assert set(result) == {'contractVersion', 'ok', 'data', 'error'} and result['contractVersion'] == '1.0.0-rc.1', name
    if error:
        assert result['ok'] is False and result['data'] is None and result['error']['code'] == error, (name, result)
        assert set(result['error']) == {'code', 'message'} and 1 <= len(result['error']['message']) <= 240, name
        assert 'secret-test' not in result['error']['message'], name
    else:
        assert result['ok'] is True and result['error'] is None and result['data'] == {'text': text}, (name, result, text)
    telemetry = json.loads(wasi.stderr)
    assert telemetry['exitCode'] == 0 and telemetry['memoryBytes'] <= 64 * 1024 * 1024, name
    stem = f'{index:03d}-{name}'
    (args.output_dir / (stem + '.request')).write_bytes(data)
    (args.output_dir / (stem + '.expected.json')).write_bytes(native.stdout)
    rows.append(dict(id=name, requestFile=stem + '.request', expectedFile=stem + '.expected.json',
                     requestSha256=hashlib.sha256(data).hexdigest(), expectedSha256=hashlib.sha256(native.stdout).hexdigest(),
                     checks={}, expectedError=error, requestBytes=len(data), resultBytes=len(native.stdout),
                     elapsedMs=round((time.monotonic() - start) * 1000), **telemetry))
    if (index + 1) % 25 == 0:
        print(f'Validated {index + 1}/{len(cases)} cases', flush=True)
proof = dict(wasmBytes=len(module), wasmSha256=hashlib.sha256(module).hexdigest(),
             nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(), fixtureSha256=hashlib.sha256(fixture_path.read_bytes()).hexdigest(),
             memoryMaxBytes=memory_max * 65536, operations=sorted(registered), cases=rows,
             limitations=['Node functional parity only; no fuel/Host timeout/security acceptance',
                          'Studio installation, production View and Pi are not tested here'])
(args.output_dir / 'fuel-proof.json').write_text(json.dumps(proof, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
print(f'PASS {len(rows)} cases / {len(registered)} operations; module {len(module)} bytes', flush=True)
