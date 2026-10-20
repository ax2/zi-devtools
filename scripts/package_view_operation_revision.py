"""Freeze independently verified View navigation candidates, preserving old candidates."""
import argparse
import copy
import hashlib
import json
from pathlib import Path

from package_directory_revision import guard, guard_tree
from package_studio_catalog_revision import ROOT, ENCODING, json_bytes, directory_files, verify_package
from tool_directory import validate_directory

PROFILE = 'view.operation.directory.v1'
VERSION = '=1.0.0-preview.1'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate_revision(before, after, old_catalog, new_catalog):
    old, new = copy.deepcopy(before), copy.deepcopy(after)
    if (old.pop('version'), new.pop('version')) != ('0.1.2','0.1.3'):
        raise ValueError('Expected View navigation revision 0.1.2 -> 0.1.3')
    if new.pop('requiredProfiles') != {**old.pop('requiredProfiles'),PROFILE:VERSION}:
        raise ValueError('Only the exact navigation profile may be added')
    operations = new['contributes'].pop('viewOperations')
    proposal = json.loads((ROOT/'proposals/studio-view-operation-directory/2026-10-18/operations.json').read_bytes())
    expected = [{k:op[k] for k in ['id','title','viewId','sourceToolId','directory']}
                for op in proposal['operations'] if op['packageId']==after['id']]
    if operations != expected:
        raise ValueError('Expected exact six-operation mapping split between two packages')
    for op in operations:
        validate_directory(op['directory'])
    if old != new:
        raise ValueError('Navigation revision changed permissions or executable contributions')
    old, new = copy.deepcopy(old_catalog), copy.deepcopy(new_catalog)
    key = 'packageVersion' if 'packageVersion' in old else 'pluginVersion'
    if (old.pop(key),new.pop(key)) != ('0.1.2','0.1.3'):
        raise ValueError('Catalog version mismatch')
    if 'requiredProfiles' in old:
        if new.pop('requiredProfiles') != {**old.pop('requiredProfiles'),PROFILE:VERSION}:
            raise ValueError('Catalog profile mismatch')
    if old != new:
        raise ValueError('Source scope or source tool version drift')


def validate_proof(proof, manifest, view, manifest_bytes):
    if proof.get('status') != 'passed' or proof.get('runtimeAccepted') is not False:
        raise ValueError('Expected passed provider proof, not Host acceptance')
    expected = {(op['id'],theme) for op in manifest['contributes']['viewOperations'] for theme in ['light','dark']}
    rows = [row for row in proof['cases'] if row['operationId'] in {op for op,_ in expected}]
    if len(rows) != len(expected) or {(row['operationId'],row['theme']) for row in rows} != expected:
        raise ValueError('Missing or duplicate rendered operation/theme evidence')
    for row in rows:
        if row['viewSha256'] != digest(view) or row['manifestSha256'] != digest(manifest_bytes):
            raise ValueError('Rendered proof is stale')
        if row['status'] != 'opened' or row['busyStatus'] != 'unsupported' or row['noAutomaticRequest'] is not True or row['preservesDraftAndResult'] is not True:
            raise ValueError('Navigation or gating evidence failed')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plugin', choices=['agent-assist-preview','spring-actuator'], required=True)
    parser.add_argument('--proof', type=Path, required=True)
    args = parser.parse_args()
    baseline_path = 'release/plugin-candidates/'+args.plugin+'-0.1.2-candidate-01'
    receipt_path = baseline_path+'-verification.json'
    baseline = guard(ROOT/baseline_path, ROOT/'release')
    guard_tree(baseline)
    verified = verify_package(baseline_path,receipt_path)
    name = args.plugin+'-0.1.3-candidate-01'
    output = guard(ROOT/'release/plugin-candidates'/name,ROOT/'release/plugin-candidates')
    receipt = output.with_name(name+'-verification.json')
    if output.exists() or receipt.exists():
        raise ValueError('Refusing to overwrite frozen candidate or receipt')
    source = ROOT/'plugins'/args.plugin
    guard_tree(source)
    manifest_bytes = (source/'plugin.json').read_bytes()
    manifest = json.loads(manifest_bytes)
    validate_revision(json.loads((baseline/'plugin.json').read_bytes()),manifest,
                      json.loads((baseline/'catalog.json').read_bytes()),json.loads((source/'catalog.json').read_bytes()))
    proof = json.loads(args.proof.read_bytes())
    view = (source/'views/main.html').read_bytes()
    validate_proof(proof,manifest,view,manifest_bytes)
    payload = {row['path']:(baseline/row['path']).read_bytes() for row in directory_files(baseline)}
    for path in ['plugin.json','catalog.json','README.md','views/main.html']:
        payload[path] = (source/path).read_bytes()
    payload['verification/view-operation-navigation.json'] = args.proof.read_bytes()
    payload['verification/view-operation-baseline-receipt.json'] = (ROOT/receipt_path).read_bytes()
    for path in ['upstream-contract.md','provenance.json']:
        payload['verification/view-operation-contract/'+path] = (ROOT/'contracts/studio-view-operation-directory/1.0.0-preview.1'/path).read_bytes()
    payload['verification/verify_view_operation_navigation.py'] = (ROOT/'scripts/verify_view_operation_navigation.py').read_bytes()
    files = [{'path':path,'size':len(data),'sha256':digest(data)} for path,data in sorted(payload.items())]
    result = {'schemaVersion':1,'packageId':manifest['id'],'packageVersion':manifest['version'],
              'baseline':verified,'files':files,'directoryIndexEncoding':ENCODING,
              'directoryIndexSha256':digest(json_bytes(files)),'viewSha256':digest(view),
              'signatureVerified':False,'runtimeAccepted':False,'fullToolAcceptance':False,
              'limitations':['Provider browser proof uses a mock public SDK; real Studio installation/transport/native acceptance pending',
                             'View operations only navigate; model and HTTP calls still require separate user action and Host confirmation',
                             'Inherited baseline verification files describe their historical versions, not the changed View runtime']}
    output.mkdir()
    for path,data in payload.items():
        target = output/path
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_bytes(data)
    with receipt.open('xb') as stream:
        stream.write(json_bytes(result,pretty=True))
    print(json.dumps(verify_package(output.relative_to(ROOT).as_posix(),receipt.relative_to(ROOT).as_posix()),ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
