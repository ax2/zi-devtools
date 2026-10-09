"""Freeze an unsigned transform candidate bound to actual module and View proofs."""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from plugin_proof import validate_cold

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
for name in ['wasm', 'parity', 'budget', 'view-proof', 'cold-proof', 'schema-proof', 'output']:
    parser.add_argument('--' + name, type=Path, required=True)
parser.add_argument('--plugin', choices=['transforms', 'inspect'], default='transforms')
parser.add_argument('--cold-history', type=Path, help='Preserve a previous full cold run, including measured failures')
args = parser.parse_args()
plugin_dir = ROOT / 'plugins' / args.plugin
adapter = 'zi-inspect-wasi' if args.plugin == 'inspect' else 'zi-text-wasi'
sync_script = 'scripts/sync_' + args.plugin + '_plugin.py'
fixture_spec = json.loads((plugin_dir / 'fixtures.json').read_text(encoding='utf-8'))
case_count = len(fixture_spec['cases']) + 19
output = args.output.absolute()
assert not output.exists(), 'Never replace a prior candidate'
assert output.resolve().is_relative_to((ROOT / 'release').resolve())
assert not any(p.is_symlink() or p.is_junction() for p in output.parents), 'Linked ancestors rejected'
digest = lambda data: hashlib.sha256(data).hexdigest()
encoded = lambda value: (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode()
module = args.wasm.read_bytes()
parity = json.loads((args.parity / 'fuel-proof.json').read_text(encoding='utf-8'))
budget = json.loads((args.budget / 'fuel-proof.json').read_text(encoding='utf-8'))
view = plugin_dir / 'views/main.html'
view_proof = json.loads(args.view_proof.read_text(encoding='utf-8'))
cold = json.loads(args.cold_proof.read_text(encoding='utf-8'))
cold_history = json.loads(args.cold_history.read_text(encoding='utf-8')) if args.cold_history else None
schema_proof = json.loads(args.schema_proof.read_text(encoding='utf-8'))
assert len(module) <= 2 * 1024 * 1024 and parity['wasmSha256'] == budget['wasmSha256'] == view_proof['wasmSha256'] == digest(module)
assert parity['memoryMaxBytes'] == 64 * 1024 * 1024
assert len(parity['cases']) == len(budget['cases']) == case_count and not budget['failures']
assert parity['fixtureSha256'] == digest((plugin_dir / 'fixtures.json').read_bytes())
assert {c['id'] for c in fixture_spec['cases']} <= {c['id'] for c in parity['cases']}
assert budget['budget'] == dict(fuel=10000000, deadlineSeconds=5, memoryBytes=67108864, guestStackBytes=2097152)
assert budget['engineProvenance']['nativeEngineVersion'] == '36.0.2'
assert all(c['passBudget'] for c in budget['cases'])
assert view_proof['viewSha256'] == digest(view.read_bytes()) and view_proof['offlinePass']
validate_cold(cold, parity, digest(module), len(module), budget['engineProvenance'], require_pass=args.plugin == 'inspect')
if cold_history is not None:
    validate_cold(cold_history, parity, digest(module), len(module), budget['engineProvenance'], require_pass=False)
assert schema_proof['wasmSha256'] == digest(module) and schema_proof['resultCases'] == case_count
assert schema_proof['requestSchemaSha256'] == digest((plugin_dir / 'request.schema.json').read_bytes())
assert schema_proof['catalogSha256'] == digest((plugin_dir / 'catalog.json').read_bytes())
assert schema_proof['fixtureProofSha256'] == digest((args.parity / 'fuel-proof.json').read_bytes())
assert schema_proof['contractIndexSha256'] == digest((ROOT / 'contracts/studio-devtools/v1/SHA256SUMS.json').read_bytes())
subprocess.run(['python', str(ROOT / sync_script), '--check'], cwd=ROOT, check=True)
manifest = json.loads((plugin_dir / 'plugin.json').read_text(encoding='utf-8'))
catalog = json.loads((plugin_dir / 'catalog.json').read_text(encoding='utf-8'))
caps = {c['id'] for c in manifest['contributes']['capabilities']}
assert len(caps) == (8 if args.plugin == 'inspect' else 23) and caps == set(parity['operations']) == {t['capability'] for t in manifest['contributes']['piTools']}
assert caps == {c['capabilityId'] for c in catalog['tools']}
actions = {c.removeprefix('devtools.' + args.plugin + '.') for c in caps}
assert len(view_proof['results']) >= 2 and all(r['pass'] and r['actualWasi'] and set(r['actions']) == actions for r in view_proof['results'])
git = lambda *argv: subprocess.check_output(['git', *argv], cwd=ROOT).decode().strip()
source = dict(revision=git('rev-parse', 'HEAD'), dirty=bool(git('status', '--porcelain')))
catalog['source'] = source
paths = [ROOT / p for p in ['Cargo.toml', 'Cargo.lock', '.cargo/config.toml', sync_script,
                          'scripts/package_transforms_plugin.py', 'scripts/plugin_proof.py', 'scripts/verify_transforms_wasi.py',
                          'scripts/verify_transforms_schema.py', 'scripts/verify_diagnostics_fuel.py', 'scripts/run_text_wasi.mjs', 'src/tools.rs', 'src/tools_extra.rs']]
folders = ['crates/zi-text-core', 'crates/' + adapter, 'plugins/' + args.plugin, 'contracts/studio-devtools/v1']
if args.plugin == 'inspect':
    folders.append('crates/zi-inspect-core')
    paths += [ROOT / 'plugins/transforms/views/main.html', ROOT / 'src/tools_advanced.rs']
for folder in folders:
    paths.extend(p for p in (ROOT / folder).rglob('*') if p.is_file())
provenance = dict(source=source, profile='compute.wasi.v1', status='unsigned provider candidate; not Host-accepted',
                  sourceFiles=[dict(path=p.relative_to(ROOT).as_posix(), sha256=digest(p.read_bytes())) for p in sorted(set(paths))],
                  rustc=subprocess.check_output(['rustc', '--version']).decode().strip())
payload = {}


def write(name, data):
    path = output / name
    assert path.resolve().is_relative_to(output.resolve())
    assert name not in payload, f'Duplicate package file: {name}'
    payload[name] = data


for kind, folder, evidence in [('parity', args.parity, parity), ('budget', args.budget, budget)]:
    write(f'fixtures/{kind}/fuel-proof.json', encoded(evidence))
    for case in evidence['cases']:
        for key, sha_key in [('requestFile', 'requestSha256'), ('expectedFile', 'expectedSha256')]:
            name = case[key]
            assert Path(name).name == name
            data = (folder / name).read_bytes()
            assert digest(data) == case[sha_key]
            write(f'fixtures/{kind}/{name}', data)
assert {c['id']: c['expectedSha256'] for c in parity['cases']} == {c['id']: c['expectedSha256'] for c in budget['cases']}
write('runtime/tools.wasm', module)
write('plugin.json', encoded(manifest))
write('catalog.json', encoded(catalog))
write('provenance.json', encoded(provenance))
write('views/main.html', view.read_bytes())
write('fixtures/' + args.plugin + '.json', (plugin_dir / 'fixtures.json').read_bytes())
write('verification/view-proof.json', encoded(view_proof))
write('verification/cold-proof.json', encoded(cold))
if cold_history is not None:
    write('verification/cold-history.json', encoded(cold_history))
write('verification/schema-proof.json', encoded(schema_proof))
for name in ['verify_diagnostics_fuel.py', 'bootstrap_diagnostics_engine.py', 'verify_diagnostics_cold.py']:
    write('verification/' + name, (ROOT / 'scripts' / name).read_bytes())
write('verification/requirements.txt', b'wasmtime==36.0.0\n')
write('README.md', (plugin_dir / 'README.md').read_bytes())
write('LICENSE', (ROOT / 'LICENSE').read_bytes())
for p in (ROOT / 'contracts/studio-devtools/v1').glob('*.schema.json'):
    schema_path = plugin_dir / 'request.schema.json' if p.name == 'request.schema.json' else p
    write('schemas/' + p.name, schema_path.read_bytes())
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--offline', '--locked', '--filter-platform', 'wasm32-wasip1', '--format-version', '1'], cwd=ROOT))
packages = {p['id']: p for p in metadata['packages']}
nodes = {n['id']: n for n in metadata['resolve']['nodes']}
pending = [next(p['id'] for p in packages.values() if p['name'] == adapter)]
seen, notices = set(), []
while pending:
    id_ = pending.pop()
    if id_ in seen:
        continue
    seen.add(id_)
    package = packages[id_]
    pending.extend(nodes[id_]['dependencies'])
    assert package['license'], f'Missing license: {package["name"]}'
    notices.append(f'{package["name"]} {package["version"]}: {package["license"]}')
    if package['source'] is not None:
        base = Path(package['manifest_path']).parent
        licenses = [p for p in base.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE', 'COPYING'))]
        if package.get('license_file'):
            licenses.append(base / package['license_file'])
        assert licenses, f'Missing license text: {package["name"]}'
        for p in set(licenses):
            write(f'licenses/{package["name"]}-{package["version"]}/{p.name}', p.read_bytes())
write('NOTICE', (manifest['name'] + ' ' + manifest['version'] + '. MIT. Resolved Cargo dependency closure (including build dependencies):\n' + '\n'.join(sorted(notices)) + '\nRust standard library: MIT OR Apache-2.0.\n').encode())
files = []
for name, data in sorted(payload.items()):
    assert len(data) <= 10 * 1024 * 1024
    files.append(dict(path=Path(name).as_posix(), size=len(data), sha256=digest(data)))
assert len(files) <= 2048 and sum(f['size'] for f in files) <= 50 * 1024 * 1024
verification = dict(**provenance, files=files, directoryIndexSha256=digest(json.dumps(files, ensure_ascii=False, separators=(',', ':')).encode()),
                    directoryIndexEncoding=dict(hash='SHA-256', encoding='UTF-8', content='files array in listed order',
                        objectKeyOrder=['path', 'size', 'sha256'], ensureAscii=False, separators=[',', ':'], trailingNewline=False),
                    packageSha256=None, signature='unsigned directory; no signing key or container',
                    functionalProof=parity, fuelProof=budget, viewProof=view_proof, coldProof=cold, coldHistory=cold_history, schemaProof=schema_proof,
                    unverified=['Actual Studio SDK/installation/update/rollback/revoke/uninstall',
                                'Production Worker isolation, output pipes, cold deadline and Pi Broker authorization',
                                'macOS/Linux Host behavior'])
# All proof, dependency-license and size checks precede any package writes.
for name, data in payload.items():
    path = output / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
output.parent.joinpath(output.name + '-verification.json').write_bytes(encoded(verification))
for row in files:
    assert digest((output / row['path']).read_bytes()) == row['sha256']
print(f'Unsigned {args.plugin} candidate: {output}; {len(files)} files, {sum(f["size"] for f in files)} bytes; not Host-accepted')
