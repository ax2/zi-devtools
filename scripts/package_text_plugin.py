"""Assemble a small unsigned candidate, never a signed/installable publication."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--proof', type=Path, required=True)
parser.add_argument('--view-proof', type=Path, required=True)
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
assert len(proof['cases'])>=36
view_proof = json.loads(args.view_proof.read_text(encoding='utf-8'))
assert view_proof['viewSha256']==digest((root/'plugins/text/views/main.html').read_bytes())
assert all(item['pass'] for item in view_proof['results']) and view_proof['offlinePass']
git = lambda *a: subprocess.check_output(['git',*a],cwd=root).decode().strip()
source = dict(revision=git('rev-parse','HEAD'),dirty=bool(git('status','--porcelain')))
contract = root/'contracts/studio-devtools/v1'
catalog = json.loads((contract/'catalog.example.json').read_text(encoding='utf-8'))
catalog['source']=source
versions = {item['id']:item['tool_version'] for item in json.loads((root/'docs/tools.json').read_text(encoding='utf-8'))['tools']}
for item in catalog['tools']: item['version']=versions[item['sourceToolId']]
provenance = dict(source=source,contract=catalog['contract'],contractIndexSha256=digest((contract/'SHA256SUMS.json').read_bytes()),rustc=subprocess.check_output(['rustc','--version']).decode().strip(),status='unsigned candidate; not Host-accepted')
source_files = [root/'Cargo.lock',root/'Cargo.toml',root/'plugins/text/views/main.html']
source_files += sorted((root/'crates').rglob('*.rs')) + sorted((root/'crates').rglob('Cargo.toml'))
provenance['sourceFiles'] = [dict(path=path.relative_to(root).as_posix(),sha256=digest(path.read_bytes())) for path in source_files]
def write(path,data):
    target=output/path
    target.parent.mkdir(parents=True,exist_ok=True)
    target.write_bytes(data)
def json_bytes(value): return (json.dumps(value,ensure_ascii=False,indent=2)+'\n').encode()
write(Path('runtime/tools.wasm'),module)
write(Path('plugin.json'),(contract/'plugin.example.json').read_bytes())
write(Path('catalog.json'),json_bytes(catalog))
write(Path('provenance.json'),json_bytes(provenance))
for name in ['request','input','result','delivery']:
    write(Path(f'schemas/{name}.schema.json'),(contract/f'{name}.schema.json').read_bytes())
write(Path('fixtures/text.json'),(contract/'fixtures.json').read_bytes())
write(Path('views/main.html'),(root/'plugins/text/views/main.html').read_bytes())
write(Path('LICENSE'),(root/'LICENSE').read_bytes())
write(Path('NOTICE'),b'ZiDevTools Text 0.1.0. MIT. Rust core dependencies: serde/serde_json (MIT OR Apache-2.0), base64 (MIT OR Apache-2.0), sha2 and RustCrypto dependencies (MIT OR Apache-2.0). Rust standard library (MIT OR Apache-2.0).\n')
write(Path('README.md'),(root/'plugins/text/README.md').read_bytes())
files=[]
for path in sorted(output.rglob('*')):
    if path.is_file():
        data=path.read_bytes()
        assert len(data)<=10*1024*1024
        files.append(dict(path=path.relative_to(output).as_posix(),sha256=digest(data),size=len(data)))
assert len(files)<=2048 and sum(item['size'] for item in files)<=50*1024*1024
index_bytes=json.dumps(files,ensure_ascii=False,separators=(',',':')).encode()
verification=dict(**provenance,files=files,directoryIndexSha256=digest(index_bytes),packageSha256=None,signature='unsigned; no .zicode-plugin container or signing key',functionalProof=proof,viewProof=view_proof,unverified=['Studio install, update/rollback, revoke, uninstall','Host fuel/time/process isolation and maximum output pipe drain','Actual Host SDK, scenes and Pi Broker authorization','macOS/Linux standalone and Host behavior'])
output.parent.joinpath(output.name+'-verification.json').write_bytes(json_bytes(verification))
print(f'Unsigned candidate: {output}; {len(files)} files, {sum(item["size"] for item in files)} bytes; no signed package')
