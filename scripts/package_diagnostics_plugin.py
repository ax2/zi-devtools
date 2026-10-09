"""Assemble an unsigned provider proposal with current runtime/View evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser()
for name in ['wasm','proof','view-proof','fuel-proof','serializer-proof','serializer-module','cold-proof','output']:parser.add_argument('--'+name,type=Path,required=True)
args=parser.parse_args();output=args.output.absolute()
assert not output.exists(),'Use a new directory; never replace a prior delivery'
assert output.resolve().is_relative_to((root/'release').resolve()),'Candidate must stay in release'
assert not any(p.is_symlink() or p.is_junction() for p in output.parents),'No linked directory ancestors'
digest=lambda data:hashlib.sha256(data).hexdigest()
module=args.wasm.read_bytes();proof=json.loads(args.proof.read_text(encoding='utf-8'))
assert digest(module)==proof['wasmSha256'] and len(module)<=2*1024*1024
assert proof['memoryMaxBytes']==64*1024*1024 and len(proof['cases'])>=188
fuel=json.loads((args.fuel_proof/'fuel-proof.json').read_text(encoding='utf-8'))
assert fuel['wasmSha256']==digest(module) and not fuel['failures'] and len(fuel['cases'])>=188
assert fuel['budget']==dict(fuel=10000000,deadlineSeconds=5,memoryBytes=67108864,guestStackBytes=2097152)
serializer=json.loads((args.serializer_proof/'fuel-proof.json').read_text(encoding='utf-8'))
assert serializer['wasmSha256']==digest(args.serializer_module.read_bytes())
assert serializer['fixtureKind']=='shared-serializer-only' and not serializer['failures'] and len(serializer['cases'])==9
cold=json.loads(args.cold_proof.read_text(encoding='utf-8'))
assert cold['wasmSha256']==digest(module) and not cold['failures'] and len(cold['rows'])>=188 and cold['remaining']==0
assert cold['engine']['nativeEngineVersion']=='36.0.2'
view=root/'plugins/diagnostics/views/main.html';view_proof=json.loads(args.view_proof.read_text(encoding='utf-8'))
assert view_proof['viewSha256']==digest(view.read_bytes()) and view_proof['offlinePass'] and view_proof['wasmSha256']==digest(module)
subprocess.run(['python',str(root/'scripts/sync_diagnostics_plugin.py'),'--check'],cwd=root,check=True)
manifest=json.loads((root/'plugins/diagnostics/plugin.json').read_text(encoding='utf-8'))
catalog=json.loads((root/'plugins/diagnostics/catalog.json').read_text(encoding='utf-8'))
actions={t['capabilityId'].removeprefix('devtools.diagnostics.') for t in catalog['tools']}
assert actions <= {case['id'] for case in proof['cases']} and len(actions)==11
assert len(view_proof['results'])>=2
assert all(row['pass'] and row['actualWasi'] and set(row['actions'])==actions for row in view_proof['results'])
git=lambda *a:subprocess.check_output(['git',*a],cwd=root).decode().strip()
source=dict(revision=git('rev-parse','HEAD'),dirty=bool(git('status','--porcelain')))
catalog['source']=source
sources=[root/'Cargo.toml',root/'Cargo.lock',root/'scripts/sync_diagnostics_plugin.py',root/'scripts/package_diagnostics_plugin.py',root/'scripts/verify_diagnostics_wasi.py',root/'scripts/verify_diagnostics_fuel.py',root/'scripts/verify_diagnostics_cold.py',root/'scripts/bootstrap_diagnostics_engine.py',root/'scripts/diagnostics_vectors.py',root/'scripts/run_text_wasi.mjs']
for folder in ['crates/zi-diagnostics-core','crates/zi-diagnostics-wasi','contracts/diagnostics/v1','plugins/diagnostics']:
    sources+=sorted(p for p in (root/folder).rglob('*') if p.is_file())
provenance=dict(source=source,profile='diagnostics.compute.wasi.v1.proposal',status='unsigned candidate; diagnostic profile not Host-accepted',rustc=subprocess.check_output(['rustc','--version']).decode().strip(),sourceFiles=[dict(path=p.relative_to(root).as_posix(),sha256=digest(p.read_bytes())) for p in sources])
def write(name,data):
    path=output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
def encoded(value):return (json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode()
for kind,folder,evidence in [('budget',args.fuel_proof,fuel),('serializer',args.serializer_proof,serializer)]:
    write('fixtures/'+kind+'/fuel-proof.json',encoded(evidence))
    for case in evidence['cases']:
        for key in ['requestFile','expectedFile']:
            name=case[key];assert Path(name).name==name
            write('fixtures/'+kind+'/'+name,(folder/name).read_bytes())
write('verification/serializer.wasm',args.serializer_module.read_bytes());write('verification/verify_diagnostics_fuel.py',(root/'scripts/verify_diagnostics_fuel.py').read_bytes());write('verification/requirements.txt',b'wasmtime==36.0.0\n');write('verification/bootstrap_diagnostics_engine.py',(root/'scripts/bootstrap_diagnostics_engine.py').read_bytes());write('verification/verify_diagnostics_cold.py',(root/'scripts/verify_diagnostics_cold.py').read_bytes());write('verification/cold-proof.json',encoded(cold));write('runtime/tools.wasm',module);write('plugin.json',encoded(manifest));write('catalog.json',encoded(catalog));write('provenance.json',encoded(provenance))
for p in (root/'contracts/diagnostics/v1').glob('*.schema.json'):write('schemas/'+p.name,p.read_bytes())
write('contracts/coverage.md',(root/'docs/diagnostics-plugin-coverage.md').read_bytes());write('views/main.html',view.read_bytes());write('README.md',(root/'plugins/diagnostics/README.md').read_bytes());write('LICENSE',(root/'LICENSE').read_bytes())
metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=root))
packages={p['id']:p for p in metadata['packages']};nodes={n['id']:n for n in metadata['resolve']['nodes']}
pending=[next(p['id'] for p in metadata['packages'] if p['name']=='zi-diagnostics-wasi')];seen=set();notices=[]
while pending:
    id=pending.pop()
    if id in seen:continue
    seen.add(id);p=packages[id];pending.extend(nodes[id]['dependencies'])
    assert p['license'],f'Missing license declaration: {p["name"]}'
    notices.append(f'{p["name"]} {p["version"]}: {p["license"]}')
    if p['source'] is not None:
        base=Path(p['manifest_path']).parent
        licenses=sorted(f for f in base.iterdir() if f.is_file() and f.name.upper().startswith(('LICENSE','COPYING')))
        if p.get('license_file'):licenses.append(base/p['license_file'])
        assert licenses,f'Missing license text: {p["name"]}'
        for f in set(licenses):write(f'licenses/{p["name"]}-{p["version"]}/{f.name}',f.read_bytes())
write('NOTICE',('ZiDevTools Diagnostics 0.1.2. MIT. Cargo dependency closure (including build dependencies):\n'+'\n'.join(sorted(notices))+'\nRust standard library: MIT OR Apache-2.0.\n').encode())
files=[]
for p in sorted(output.rglob('*')):
    if p.is_file():
        data=p.read_bytes();assert len(data)<=10*1024*1024
        files.append(dict(path=p.relative_to(output).as_posix(),sha256=digest(data),size=len(data)))
assert len(files)<=2048 and sum(f['size'] for f in files)<=50*1024*1024
verification=dict(**provenance,files=files,directoryIndexSha256=digest(json.dumps(files,ensure_ascii=False,separators=(',',':')).encode()),packageSha256=None,signature='unsigned directory; no signing key or container',functionalProof=proof,viewProof=view_proof,fuelProof=fuel,serializerProof=serializer,coldProof=cold,buildProfile=dict(name='plugin',optLevel=3,lto='fat',panic='abort',standalone='release unchanged'),unverified=['Diagnostic schema/profile/error enums require public Host agreement','Studio installation, update/rollback, revoke, uninstall','Actual Host SDK/Pi Broker and authorization','Production Host budget/isolation; independent official C-API36.0.2 evidence is not Host acceptance','macOS/Linux Host behavior'])
output.parent.joinpath(output.name+'-verification.json').write_bytes(encoded(verification))
for item in files:assert digest((output/item['path']).read_bytes())==item['sha256']
print(f'Unsigned diagnostics candidate: {output}; {len(files)} files, {sum(f["size"] for f in files)} bytes; not Host-accepted')
