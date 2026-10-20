"""Freeze metadata-only application discovery revisions without changing View bytes."""
import argparse
import copy
import hashlib
import json

from package_directory_revision import guard, guard_tree
from package_studio_catalog_revision import ROOT, ENCODING, json_bytes, directory_files, verify_package
from tool_directory import validate_directory

PROFILE = 'application.directory.v1'
VERSION = '=1.0.0-preview.1'
SNAPSHOT = ROOT/'contracts/studio-application-directory/1.0.0-preview.1/applications.json'
BASELINES = {
    'agent-assist-preview': ('release/agent-assist-preview-0.1.1-candidate-02',
        'release/plugin-candidates/agent-assist-preview-0.1.1-candidate-02-verification-revision-01.json',
        'agent-assist-preview-0.1.2-candidate-01'),
    'spring-actuator': ('release/plugin-candidates/spring-actuator-0.1.1-candidate-02',
        'release/plugin-candidates/spring-actuator-0.1.1-candidate-02-verification-revision-01.json',
        'spring-actuator-0.1.2-candidate-01'),
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate_revision(before, after, old_catalog, new_catalog):
    old, new = copy.deepcopy(before), copy.deepcopy(after)
    if (old.pop('version'), new.pop('version')) != ('0.1.1','0.1.2'):
        raise ValueError('Expected application metadata revision 0.1.1 -> 0.1.2')
    profiles = old.pop('requiredProfiles')
    if new.pop('requiredProfiles') != {**profiles, PROFILE:VERSION}:
        raise ValueError('Only the supported application directory profile may be added')
    activities = new['contributes']['activities']
    if len(activities) != 1:
        raise ValueError('Expected one primary activity')
    metadata = activities[0].pop('directory')
    validate_directory(metadata)
    expected = json.loads(SNAPSHOT.read_bytes())['applications'][after['id']]
    if metadata != expected:
        raise ValueError('Directory metadata differs from local contract mapping')
    if old != new:
        raise ValueError('Metadata revision changed permissions, routing, runtime or contribution semantics')
    if any(after['contributes'].get(key) for key in ('capabilities','commands','piTools')):
        raise ValueError('Applications must not claim executable tools')
    old, new = copy.deepcopy(old_catalog), copy.deepcopy(new_catalog)
    key = 'packageVersion' if 'packageVersion' in old else 'pluginVersion'
    if (old.pop(key), new.pop(key)) != ('0.1.1','0.1.2'):
        raise ValueError('Catalog/package versions differ')
    if 'requiredProfiles' in old:
        if new.pop('requiredProfiles') != {**old.pop('requiredProfiles'),PROFILE:VERSION}:
            raise ValueError('Catalog profile mismatch')
    if old != new:
        raise ValueError('Source versions or scope changed in a directory-only revision')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plugin', choices=BASELINES, required=True)
    args = parser.parse_args()
    baseline_path, receipt_path, name = BASELINES[args.plugin]
    baseline = guard(ROOT/baseline_path, ROOT/'release')
    guard_tree(baseline)
    verified = verify_package(baseline_path, receipt_path)
    output = guard(ROOT/'release/plugin-candidates'/name, ROOT/'release/plugin-candidates')
    receipt = guard(output.with_name(name+'-verification.json'), ROOT/'release/plugin-candidates')
    if output.exists() or receipt.exists():
        raise ValueError('Refusing to overwrite frozen directory or receipt')
    source = ROOT/'plugins'/args.plugin
    manifest = json.loads((source/'plugin.json').read_bytes())
    validate_revision(json.loads((baseline/'plugin.json').read_bytes()),manifest,
                      json.loads((baseline/'catalog.json').read_bytes()),json.loads((source/'catalog.json').read_bytes()))
    payload = {row['path']:(baseline/row['path']).read_bytes() for row in directory_files(baseline)}
    for path in ('plugin.json','catalog.json','README.md'):
        payload[path]=(source/path).read_bytes()
    if payload['views/main.html'] != (source/'views/main.html').read_bytes():
        raise ValueError('View changed; cannot reuse baseline runtime evidence')
    payload['verification/application-directory.json']=json_bytes({
        'profile':PROFILE,'version':VERSION,'packageId':manifest['id'],
        'activity':manifest['contributes']['activities'][0]},pretty=True)
    payload['verification/application-directory-baseline-receipt.json']=(ROOT/receipt_path).read_bytes()
    files=[{'path':name,'size':len(data),'sha256':digest(data)} for name,data in sorted(payload.items())]
    result={'schemaVersion':1,'packageId':manifest['id'],'packageVersion':manifest['version'],
            'baseline':verified,'files':files,'directoryIndexEncoding':ENCODING,
            'directoryIndexSha256':digest(json_bytes(files)),
            'viewSha256':digest(payload['views/main.html']),
            'signatureVerified':False,'runtimeAccepted':False,'fullToolAcceptance':False,
            'limitations':['Metadata-only revision; View and all executable/schema bytes retained from verified baseline',
                           'Two application entries only; six View operations are not independently discoverable or Pi callable',
                           'Baseline runtime proofs are historical; no new Studio installation or native UI acceptance']}
    output.mkdir()
    for name,data in payload.items():
        path=output/name
        path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(data)
    with receipt.open('xb') as stream:
        stream.write(json_bytes(result,pretty=True))
    print(json.dumps(verify_package(output.relative_to(ROOT).as_posix(),receipt.relative_to(ROOT).as_posix()),ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
