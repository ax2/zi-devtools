"""Assemble a small unsigned candidate, never a signed/installable publication."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess
from tool_directory import apply_manifest, directory_snapshot
from plugin_proof import validate_budget, validate_cold, validate_compute_metadata

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--proof', type=Path, required=True)
parser.add_argument('--view-proof', type=Path, required=True)
parser.add_argument('--budget-proof', type=Path, required=True)
parser.add_argument('--cold-proof', type=Path, required=True)
parser.add_argument('--engine-provenance', type=Path, required=True)
parser.add_argument('--cases-dir', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
output = args.output.absolute()
assert not output.exists(), 'Use a new candidate directory; never replace a prior delivery'
assert output.resolve().is_relative_to((root/'release').resolve()), 'Candidate must stay in release'
assert not any(p.is_symlink() for p in output.parents), 'No links'
proof = json.loads(args.proof.read_text(encoding='utf-8'))
digest = lambda data: hashlib.sha256(data).hexdigest()
module = args.wasm.read_bytes()
assert digest(module)==proof['wasmSha256'] and len(module)<=2*1024*1024
assert len(proof['cases']) == 76, 'Expected the 67 frozen cases and nine dense SQL additions'
assert sum(case['id'].startswith('sql-options-dense-') for case in proof['cases']) == 9
view_proof = json.loads(args.view_proof.read_text(encoding='utf-8'))
budget_proof = json.loads(args.budget_proof.read_text(encoding='utf-8'))
cold_proof = json.loads(args.cold_proof.read_text(encoding='utf-8'))
engine_provenance = json.loads(args.engine_provenance.read_text(encoding='utf-8'))
budget_fixture = json.loads((args.cases_dir/'fuel-proof.json').read_text(encoding='utf-8'))
assert view_proof['viewSha256']==digest((root/'plugins/text/views/main.html').read_bytes())
assert view_proof['wasmSha256']==digest(module)
assert all(item['pass'] for item in view_proof['results']) and view_proof['offlinePass']
assert budget_fixture['wasmSha256']==digest(module) and len(budget_fixture['cases'])==len(proof['cases'])
assert budget_proof['engineProvenance']==engine_provenance and budget_proof['wasmtimeVersion']=='36.0.0'
validate_budget(budget_proof,proof,digest(module),len(module))
validate_cold(cold_proof,proof,digest(module),len(module),engine_provenance)
fixture_cases={item['id']:item for item in budget_fixture['cases']}
budget_cases={item['id']:item for item in budget_proof['cases']}
assert set(fixture_cases)==set(budget_cases)=={item['id'] for item in proof['cases']}
for case_id,case in fixture_cases.items():
    budget_case=budget_cases[case_id]
    for field,sha_field in [('requestFile','requestSha256'),('expectedFile','expectedSha256')]:
        assert case[field]==budget_case[field] and Path(case[field]).name==case[field]
        contents=(args.cases_dir/case[field]).read_bytes()
        assert digest(contents)==case[sha_field]==budget_case[sha_field]
git = lambda *a: subprocess.check_output(['git',*a],cwd=root).decode().strip()
source = dict(revision=git('rev-parse','HEAD'),dirty=bool(git('status','--porcelain')))
assert not source['dirty'], 'Commit the candidate source before freezing'
contract = root/'contracts/studio-devtools/v1'
catalog = json.loads((contract/'catalog.example.json').read_text(encoding='utf-8'))
plugin = json.loads((contract/'plugin.example.json').read_text(encoding='utf-8'))
plugin['version'] = '0.4.4'
catalog['packageVersion'] = plugin['version']
catalog['source']=source
versions = {item['id']:item['tool_version'] for item in json.loads((root/'docs/tools.json').read_text(encoding='utf-8'))['tools']}
for item in catalog['tools']: item['version']=versions[item['sourceToolId']]
json_format = next(item for item in catalog['tools'] if item['toolId'] == 'devtools.text.json.format')
catalog['tools'].append(dict(json_format, toolId='devtools.text.json.validate',
                             capabilityId='devtools.text.json.validate', title='JSON 校验',
                             version=versions['json']))
capabilities = plugin['contributes']['capabilities']
json_format_capability = next(item for item in capabilities if item['id'] == 'devtools.text.json.format')
capabilities.insert(2, dict(json_format_capability, id='devtools.text.json.validate', title='JSON 校验'))
sql_input_file = root/'plugins/text/sql-input.schema.json'
sql_input = json.loads(sql_input_file.read_text(encoding='utf-8'))
capabilities.append(dict(json_format_capability, id='devtools.text.sql.format', title='SQL 格式化', inputSchema=sql_input))
apply_manifest(plugin)
plugin['activationEvents'] += ['onCapability:devtools.text.json.validate', 'onCapability:devtools.text.sql.format']
pi_tools = plugin['contributes']['piTools']
json_format_tool = next(item for item in pi_tools if item['capability'] == 'devtools.text.json.format')
pi_tools.insert(2, dict(json_format_tool, name='devtools_text_json_validate', title='JSON 校验',
                         description='Run the bounded local JSON 校验 utility on explicitly supplied text.',
                         capability='devtools.text.json.validate'))
pi_tools.append(dict(json_format_tool, name='devtools_text_sql_format', title='SQL 格式化',
                     description='Format explicitly supplied SQL with optional dialect, indentation and keyword case.',
                     inputSchema=sql_input,
                     capability='devtools.text.sql.format'))
sql_format = next(item for item in catalog['tools'] if item['toolId'] == 'devtools.text.json.format')
catalog['tools'].append(dict(sql_format, toolId='devtools.text.sql.format',
                             capabilityId='devtools.text.sql.format', sourceToolId='sql-format',
                             title='SQL 格式化', version=versions['sql-format']))
validate_compute_metadata(plugin, catalog, json.loads((contract/'plugin.example.json').read_text(encoding='utf-8')),
                          require_directory=True, input_overrides={'devtools.text.sql.format': sql_input})
provenance = dict(source=source,contract=catalog['contract'],contractIndexSha256=digest((contract/'SHA256SUMS.json').read_bytes()),rustc=subprocess.check_output(['rustc','--version']).decode().strip(),status='unsigned candidate; not Host-accepted')
directory_snapshot_bytes = (json.dumps(directory_snapshot(plugin),ensure_ascii=False,indent=2)+'\n').encode()
provenance['directoryProfile'] = dict(id='tool.directory.v1',version='1.0.0-preview.1',
    schemaSha256=digest((root/'contracts/studio-tool-directory/1.0.0-preview.1/schema.json').read_bytes()),
    capabilityMapSha256=digest((root/'contracts/studio-tool-directory/1.0.0-preview.1/capabilities.json').read_bytes()),
    snapshotSha256=digest(directory_snapshot_bytes))
source_files = [root/'Cargo.lock',root/'Cargo.toml',root/'docs/tools.json',
                root/'scripts/package_text_plugin.py',root/'scripts/verify_text_wasi.py',
                root/'scripts/tool_directory.py',root/'scripts/plugin_proof.py',root/'scripts/verify_tool_directory.py',
                root/'scripts/verify_text_plugin_view.py',root/'scripts/cdp_client.py',
                root/'scripts/verify_diagnostics_fuel.py',root/'scripts/verify_diagnostics_cold.py',root/'scripts/bootstrap_diagnostics_engine.py',
                root/'scripts/sync_studio_plugin_map.py',root/'src/tools.rs',
                root/'src/sql_formatter.rs',
                root/'docs/studio-plugin-migration.json',root/'docs/studio-plugin-handoff.md',
                root/'plugins/text/README.md',root/'plugins/text/views/main.html',sql_input_file]
source_files += sorted((root/'crates').rglob('*.rs')) + sorted((root/'crates').rglob('Cargo.toml'))
source_files += sorted((root/'contracts/studio-tool-directory/1.0.0-preview.1').glob('*'))
source_files += sorted(path for path in (root/'vendor/sqlformat').rglob('*') if path.is_file())
provenance['sourceFiles'] = [dict(path=path.relative_to(root).as_posix(),sha256=digest(path.read_bytes())) for path in source_files]
def write(path,data):
    target=output/path
    target.parent.mkdir(parents=True,exist_ok=True)
    target.write_bytes(data)
def json_bytes(value): return (json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode()
write(Path('runtime/tools.wasm'),module)
write(Path('plugin.json'),json_bytes(plugin))
write(Path('verification/tool-directory-snapshot.json'),directory_snapshot_bytes)
write(Path('MIGRATION.md'),('''# Migration to the Text 0.4.4 candidate\n\nThe seven capability IDs and the directory profile are unchanged. SQL accepts optional dialect, indent and keywordCase. A text-only request produces the former generic, four-space, uppercase result. The other capabilities retain their text-only schemas. The SQL response still contains text. The packaged Text View continues to use the generic defaults. Common dense 8 KiB SQL templates still complete within the existing Worker budget. The formatter reports the existing 48 KiB result limit early for proven over-limit layouts. Deep inputs that require protected-literal restoration may still consume the existing fuel budget. The 0.4.3 directory remains available. This unsigned candidate requires separate Studio install and lifecycle acceptance.\n''').encode())
write(Path('verification/fuel-proof.json'),json_bytes(budget_proof))
write(Path('verification/cold-proof.json'),json_bytes(cold_proof))
write(Path('verification/engine-provenance.json'),json_bytes(engine_provenance))
write(Path('verification/verify_fuel.py'),(root/'scripts/verify_diagnostics_fuel.py').read_bytes())
write(Path('verification/verify_cold.py'),(root/'scripts/verify_diagnostics_cold.py').read_bytes())
write(Path('verification/bootstrap_engine.py'),(root/'scripts/bootstrap_diagnostics_engine.py').read_bytes())
write(Path('verification/requirements.txt'),b'wasmtime==36.0.0\n')
write(Path('verification/README.md'),('''# Replaying the independent runtime proofs\n\nThe package is an unsigned provider directory, not a Studio Host acceptance result. Replaying requires Windows PowerShell, Python 3.13, network access for the hash-pinned Wasmtime 36.0.2 C-API archive, and the package files. From the repository root, create a new cache under `target` and record its local engine identity:\n\n```powershell\npy -3.13 verification/bootstrap_engine.py --cache-dir target/text-plugin-wasmtime-36.0.2 --provenance target/text-plugin-engine.json\n$env:PYTHONPATH = (Resolve-Path target/text-plugin-wasmtime-36.0.2).Path\npy -3.13 verification/verify_fuel.py --wasm release/text-plugin-0.4.4-candidate-01/runtime/tools.wasm --fixtures release/text-plugin-0.4.4-candidate-01/fixtures/budget --output target/text-plugin-fuel --text-results --engine-provenance target/text-plugin-engine.json\npy -3.13 verification/verify_cold.py --wasm release/text-plugin-0.4.4-candidate-01/runtime/tools.wasm --fixtures release/text-plugin-0.4.4-candidate-01/fixtures/budget --output target/text-plugin-cold.json --engine-provenance target/text-plugin-engine.json\n```\n\nThe packaged fuel/cold proofs bind the WASM digest, raw requests, exact expected response bytes, Wasmtime native DLL digest and all 76 case IDs. They do not test Studio's production Worker, UI bridge, Pi authorization, install lifecycle or Host pipe behavior.\n''').encode())
for case_file in sorted(args.cases_dir.iterdir()):
    if case_file.is_file():
        write(Path('fixtures/budget')/case_file.name,case_file.read_bytes())
write(Path('catalog.json'),json_bytes(catalog))
write(Path('provenance.json'),json_bytes(provenance))
for name in ['request','input','result','delivery']:
    write(Path(f'schemas/{name}.schema.json'),(contract/f'{name}.schema.json').read_bytes())
write(Path('schemas/sql-input.schema.json'),sql_input_file.read_bytes())
write(Path('fixtures/text.json'),(contract/'fixtures.json').read_bytes())
write(Path('fixtures/json-validation.json'),json_bytes({
    'sourceToolId':'json','sourceToolVersion':versions['json'],'operations':[
        {'capabilityId':'devtools.text.json.format','input':'{"name":"Zi"}','expected':'{\n  "name": "Zi"\n}'},
        {'capabilityId':'devtools.text.json.minify','input':'{ "name" : "Zi" }','expected':'{"name":"Zi"}'},
        {'capabilityId':'devtools.text.json.validate','input':'{"name":"Zi"}','expected':'JSON 有效（插件兼容规则）'}],
    'invalidValidationInputs':['not-json','{"x":1,"x":2}','9007199254740992']}))
write(Path('fixtures/sql-format.json'),json_bytes({
    'sourceToolId':'sql-format','sourceToolVersion':versions['sql-format'],
    'operations':[{'capabilityId':'devtools.text.sql.format',
                   'input':'select id,name from users where active=true',
                   'expected':'SELECT id, name\nFROM users\nWHERE active = TRUE'}],
    'limitations':['deep SQL requiring protected-literal restoration can still reach the unchanged Worker budgets','no semantic validation or database connection']}))
write(Path('views/main.html'),(root/'plugins/text/views/main.html').read_bytes())
write(Path('LICENSE'),(root/'LICENSE').read_bytes())
for license_name in ['LICENSE-MIT', 'LICENSE-APACHE']:
    write(Path('licenses') / ('sqlformat-' + license_name), (root/'vendor/sqlformat'/license_name).read_bytes())
write(Path('NOTICE'),b'ZiDevTools Text 0.4.4. MIT. Rust core dependencies: serde/serde_json (MIT OR Apache-2.0), base64 (MIT OR Apache-2.0), sha2 and RustCrypto dependencies (MIT OR Apache-2.0), sqlformat (MIT OR Apache-2.0). Rust standard library (MIT OR Apache-2.0).\n')
write(Path('README.md'),(root/'plugins/text/README.md').read_bytes())
files=[]
for path in sorted(output.rglob('*')):
    if path.is_file():
        data=path.read_bytes()
        assert len(data)<=10*1024*1024
        files.append(dict(path=path.relative_to(output).as_posix(),size=len(data),sha256=digest(data)))
assert len(files)<=2048 and sum(item['size'] for item in files)<=50*1024*1024
index_bytes=json.dumps(files,ensure_ascii=False,separators=(',',':')).encode()
verification=dict(**provenance,schemaVersion=1,pluginId=plugin['id'],packageVersion=plugin['version'],
                  signatureVerified=False,files=files,fileCount=len(files),totalBytes=sum(row['size'] for row in files),
                  directoryIndexSha256=digest(index_bytes),
                  directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
                      objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False),
                  packageSha256=None,signature='unsigned; no .zicode-plugin container or signing key',functionalProof=proof,viewProof=view_proof,fuelProof=budget_proof,coldProof=cold_proof,unverified=['Studio install, update/rollback, revoke, uninstall','Studio production Worker, pipe-drain and Host timing behavior','Actual Host SDK, scenes and Pi Broker authorization','macOS/Linux standalone and Host behavior'])
output.parent.joinpath(output.name+'-verification.json').write_bytes(json_bytes(verification))
print(f'Unsigned candidate: {output}; {len(files)} files, {sum(item["size"] for item in files)} bytes; no signed package')
