"""Assemble an unsigned provider proposal with current runtime/View evidence."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

root=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser()
for name in ['wasm','proof','view-proof','output']:parser.add_argument('--'+name,type=Path,required=True)
args=parser.parse_args();output=args.output.absolute()
assert not output.exists(),'Use a new directory; never replace a prior delivery'
assert output.resolve().is_relative_to((root/'release').resolve()),'Candidate must stay in release'
assert not any(p.is_symlink() or p.is_junction() for p in output.parents),'No linked directory ancestors'
digest=lambda data:hashlib.sha256(data).hexdigest()
module=args.wasm.read_bytes();proof=json.loads(args.proof.read_text(encoding='utf-8'))
assert digest(module)==proof['wasmSha256'] and len(module)<=2*1024*1024
assert proof['memoryMaxBytes']==64*1024*1024 and len(proof['cases'])==29
view=root/'plugins/diagnostics/views/main.html';view_proof=json.loads(args.view_proof.read_text(encoding='utf-8'))
assert view_proof['viewSha256']==digest(view.read_bytes()) and view_proof['offlinePass']
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
sources=[root/'Cargo.toml',root/'Cargo.lock',root/'scripts/sync_diagnostics_plugin.py',root/'scripts/package_diagnostics_plugin.py',root/'scripts/verify_diagnostics_wasi.py',root/'scripts/run_text_wasi.mjs']
for folder in ['crates/zi-diagnostics-core','crates/zi-diagnostics-wasi','contracts/diagnostics/v1','plugins/diagnostics']:
    sources+=sorted(p for p in (root/folder).rglob('*') if p.is_file())
provenance=dict(source=source,profile='diagnostics.compute.wasi.v1.proposal',status='unsigned candidate; diagnostic profile not Host-accepted',rustc=subprocess.check_output(['rustc','--version']).decode().strip(),sourceFiles=[dict(path=p.relative_to(root).as_posix(),sha256=digest(p.read_bytes())) for p in sources])
def write(name,data):
    path=output/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
def encoded(value):return (json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode()
write('runtime/tools.wasm',module);write('plugin.json',encoded(manifest));write('catalog.json',encoded(catalog));write('provenance.json',encoded(provenance))
for p in (root/'contracts/diagnostics/v1').glob('*.schema.json'):write('schemas/'+p.name,p.read_bytes())
write('views/main.html',view.read_bytes());write('README.md',(root/'plugins/diagnostics/README.md').read_bytes());write('LICENSE',(root/'LICENSE').read_bytes())
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
write('NOTICE',('ZiDevTools Diagnostics 0.1.0. MIT. Cargo dependency closure (including build dependencies):\n'+'\n'.join(sorted(notices))+'\nRust standard library: MIT OR Apache-2.0.\n').encode())
files=[]
for p in sorted(output.rglob('*')):
    if p.is_file():
        data=p.read_bytes();assert len(data)<=10*1024*1024
        files.append(dict(path=p.relative_to(output).as_posix(),sha256=digest(data),size=len(data)))
assert len(files)<=2048 and sum(f['size'] for f in files)<=50*1024*1024
verification=dict(**provenance,files=files,directoryIndexSha256=digest(json.dumps(files,ensure_ascii=False,separators=(',',':')).encode()),packageSha256=None,signature='unsigned directory; no signing key or container',functionalProof=proof,viewProof=view_proof,unverified=['Diagnostic schema/profile/error enums require public Host agreement','Studio installation, update/rollback, revoke, uninstall','Actual Host SDK/Pi Broker and authorization','Production sandbox/fuel/timeout/isolation','macOS/Linux Host behavior'])
output.parent.joinpath(output.name+'-verification.json').write_bytes(encoded(verification))
for item in files:assert digest((output/item['path']).read_bytes())==item['sha256']
print(f'Unsigned diagnostics candidate: {output}; {len(files)} files, {sum(f["size"] for f in files)} bytes; not Host-accepted')
