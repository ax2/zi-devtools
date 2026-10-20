"""Freeze the numeric-input image adapter and its independently checked runtime proof."""
import copy
import hashlib
import json
import subprocess
from pathlib import Path

from package_studio_catalog_revision import ROOT, ENCODING, json_bytes, verify_package
from package_directory_revision import guard, guard_tree
from tool_directory import apply_manifest, directory_snapshot
from benchmark_studio_image_resource import wasm_memory_ceiling

BASELINE = ROOT/'release/plugin-candidates/image-resize-0.1.1-candidate-02'
OUTPUT = ROOT/'release/plugin-candidates/image-resize-0.2.0-candidate-01'
WASM = Path('C:/Users/ax2/AppData/Local/ZiDevTools/zi-devtools/wasm32-wasip1/plugin/zi-image-wasi.wasm')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate_revision(before, after):
    old, new = copy.deepcopy(before), copy.deepcopy(after)
    if (old.pop('version'),new.pop('version')) != ('0.1.1','0.2.0'):
        raise ValueError('Expected image adapter revision 0.1.1 -> 0.2.0')
    profiles = old.pop('requiredProfiles')
    if new.pop('requiredProfiles') != {**profiles, 'resource.input.types.v1':'=1.0.0-preview.1', 'tool.directory.v1':'=1.0.0-preview.1'}:
        raise ValueError('Unexpected profile change')
    old_cap, new_cap = old['contributes']['capabilities'][0], new['contributes']['capabilities'][0]
    if new_cap.pop('resourceInputMimeTypes') != ['image/png','image/jpeg','image/webp']:
        raise ValueError('Unexpected input MIME filter')
    expected = copy.deepcopy(after)
    apply_manifest(expected)
    if expected != after:
        raise ValueError('Directory metadata differs from provider snapshot')
    new_cap.pop('directory')
    # Only numeric input contracts and discovery change; output/routing stay fixed.
    old_schema,new_schema = old_cap['inputSchema'],new_cap['inputSchema']
    for key, minimum, maximum, default in [('maxWidth',1,4096,1600),('quality',35,95,82)]:
        expected_property = {'type':'integer','minimum':minimum,'maximum':maximum,'default':default,
                             'title':new_schema['properties'][key]['title']}
        if new_schema['properties'][key] != expected_property:
            raise ValueError('Unexpected numeric parameter constraints')
        new_schema['properties'][key] = old_schema['properties'][key]
    new_schema['properties']['format']['title'] = old_schema['properties']['format']['title']
    if old != new:
        raise ValueError('Metadata revision changed runtime, routing, output, permissions or unrelated behavior')


def main():
    guard_tree(BASELINE)
    baseline_receipt = BASELINE.with_name(BASELINE.name+'-verification.json')
    baseline = verify_package(BASELINE.relative_to(ROOT).as_posix(), baseline_receipt.relative_to(ROOT).as_posix())
    guard(OUTPUT,ROOT/'release/plugin-candidates')
    receipt_path = OUTPUT.with_name(OUTPUT.name+'-verification.json')
    if OUTPUT.exists() or receipt_path.exists():
        raise ValueError('Refusing to overwrite frozen candidate or receipt')
    source = ROOT/'plugins/image-resize'
    manifest = json.loads((source/'plugin.json').read_bytes())
    validate_revision(json.loads((BASELINE/'plugin.json').read_bytes()),manifest)
    payload = {row['path']:(BASELINE/row['path']).read_bytes()
               for row in json.loads(baseline_receipt.read_bytes())['files']}
    for name in ('plugin.json','README.md','schemas/resize-input.schema.json'):
        payload[name]=(source/name).read_bytes()
    payload['runtime/tools.wasm']=WASM.read_bytes()
    capability = manifest['contributes']['capabilities'][0]
    if json.loads(payload['schemas/resize-input.schema.json']) != capability['inputSchema']:
        raise ValueError('Input sidecar differs from manifest')
    if (source/'schemas/resize-output.schema.json').read_bytes() != payload['schemas/resize-output.schema.json']:
        raise ValueError('Output sidecar changed from baseline')
    if wasm_memory_ceiling(payload['runtime/tools.wasm']) != 256 * 1024 * 1024:
        raise ValueError('Image WASM must declare a 256 MiB memory ceiling')
    catalog=json.loads((source/'catalog.json').read_bytes())
    revision=subprocess.run(['git','rev-parse','HEAD'],cwd=ROOT,check=True,capture_output=True,text=True).stdout.strip()
    catalog['source']={'revision':revision,'dirty':True}
    if catalog['packageVersion'] != manifest['version'] or catalog['requiredProfiles'] != manifest['requiredProfiles']:
        raise ValueError('Catalog/manifest mismatch')
    payload['catalog.json']=json_bytes(catalog,pretty=True)
    proof_path=ROOT/'target/image-resize-020/runtime-proof.json'
    proof=json.loads(proof_path.read_bytes())
    if not proof.get('passed') or len(proof['cases']) != 23:
        raise ValueError('Missing 23-case direct WASI verification')
    parity = [row['nativeComparison'] for row in proof['cases'] if 'nativeComparison' in row]
    if len(parity) != 9 or not all(row.get('byteIdentical') is True for row in parity):
        raise ValueError('Missing nine native/WASI format comparisons')
    if proof['wasmSha256'] != digest(payload['runtime/tools.wasm']) or proof['inputSchemaSha256'] != digest(payload['schemas/resize-input.schema.json']):
        raise ValueError('Proof does not match the runtime and schema being packaged')
    payload['verification/baseline-receipt.json']=baseline_receipt.read_bytes()
    payload['verification/runtime-proof.json']=proof_path.read_bytes()
    payload['verification/tool-directory-snapshot.json']=json_bytes(directory_snapshot(manifest),pretty=True)
    source_paths=['Cargo.lock','.cargo/config.toml','Cargo.toml',
                  'scripts/verify_image_resize_revision.py','scripts/benchmark_studio_image_resource.py']
    for crate in ('zi-image-core','zi-image-wasi'):
        source_paths.extend(path.relative_to(ROOT).as_posix() for path in (ROOT/'crates'/crate).rglob('*') if path.is_file())
    provenance={'revision':revision,'dirty':True,'sourceToolVersion':'0.12.0',
                'files':[{'path':name,'sha256':digest((ROOT/name).read_bytes())} for name in sorted(source_paths)],
                'buildCommand':'scripts/dev.ps1 -UseUserCache dt-image-plugin --offline',
                'wasmSha256':proof['wasmSha256']}
    payload['verification/source-provenance.json']=json_bytes(provenance,pretty=True)
    files=[{'path':name,'size':len(data),'sha256':digest(data)} for name,data in sorted(payload.items())]
    receipt={'schemaVersion':1,'packageId':manifest['id'],'packageVersion':manifest['version'],
             'historicalBaseline':baseline,'files':files,'directoryIndexEncoding':ENCODING,
             'directoryIndexSha256':digest(json_bytes(files)), 'wasmSha256':proof['wasmSha256'],
             'signatureVerified':False,'runtimeAccepted':False,'fullToolAcceptance':False,
             'limitations':['Unsigned provider candidate; no installation or native Studio acceptance',
                            'New numeric adapter; source image-workflow 0.12.0 has broader scope than this single capability',
                            'Historical Worker claims retained as provenance, not newly reproduced or validated against current Host binaries',
                            'New direct WASI cases cover small format/parameter fixtures, not full 16MP/deadline/concurrency acceptance']}
    OUTPUT.mkdir()
    for name,data in payload.items():
        path=OUTPUT/name
        path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(data)
    with receipt_path.open('xb') as stream:
        stream.write(json_bytes(receipt,pretty=True))
    verified=verify_package(OUTPUT.relative_to(ROOT).as_posix(),receipt_path.relative_to(ROOT).as_posix())
    print(json.dumps(verified,ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
