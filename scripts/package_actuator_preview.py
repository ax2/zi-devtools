"""Create a review-only bundle for the unimplemented Studio HTTP GET profile."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT=Path(__file__).resolve().parents[1]
NAME='diagnostics-actuator-service-http-preview-20261015'
BUNDLE=ROOT/'proposals'/NAME
RECEIPT=ROOT/'proposals'/f'{NAME}-source-provenance.json'
CONTRACT=ROOT/'contracts/studio-host-capabilities/service-http-get-preview-20261015'
VIEW=ROOT/'plugins/diagnostics/views/spring-actuator-preview.html'
parser=argparse.ArgumentParser()
parser.add_argument('--desktop-proof',type=Path,default=ROOT/'target/actuator-preview/desktop-proof.json')
parser.add_argument('--mobile-proof',type=Path,default=ROOT/'target/actuator-preview/mobile-proof.json')
args=parser.parse_args()
assert not BUNDLE.exists() and not RECEIPT.exists(),'Refusing to overwrite a previous review bundle'
digest=lambda data:hashlib.sha256(data).hexdigest()
tool_catalog=json.loads((ROOT/'docs/tools.json').read_text(encoding='utf-8'))
tool=next(row for row in tool_catalog['tools'] if row['id']=='spring-actuator')
manifest=json.loads((ROOT/'plugins/diagnostics/plugin.json').read_text(encoding='utf-8'))
assert tool['tool_version']=='1.1.1'
assert manifest['requiredProfiles']=={'compute.wasi.v1':'=1.0.0-rc.1','output.schema.v1':'=1.0.0-rc.1'}
assert not any('service.http.get' in key for key in manifest['requiredProfiles'])
upstream=json.loads((CONTRACT/'upstream-source.json').read_text(encoding='utf-8'))
assert upstream['sha256']=='bb3e9e49bf8ee6c33577c994331d2216c7c67a7b29d3e2388e04753e409775e5'
proofs=[]
for path,theme,width,height in [(args.desktop_proof,'light',1180,900),(args.mobile_proof,'dark',390,844)]:
    proof=json.loads(path.read_text(encoding='utf-8'))
    assert proof['pass'] and proof['profile']=='service.http.get.v1@1.0.0-preview.1'
    assert proof['viewport']==dict(width=width,height=height,devicePixelRatio=1,prefersDark=theme=='dark')
    assert proof['sdkCalls']==6 and proof['pageErrors']==[] and proof['errors']==[]
    screenshot=path.parent/proof['screenshot']
    assert digest(screenshot.read_bytes())==proof['screenshotSha256']
    proofs.append((path,proof,screenshot))
fixture=json.loads((CONTRACT/'fixtures/view-cases.json').read_text(encoding='utf-8'))
assert set(proofs[0][1]['cases'])=={case['id'] for case in fixture['cases']}
assert proofs[0][1]['cases']==proofs[1][1]['cases']

files=[]
def copy_file(source,relative):
    source=Path(source);destination=BUNDLE/relative
    destination.parent.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(source,destination)
    files.append(destination)

for source in sorted(CONTRACT.rglob('*')):
    if source.is_file():copy_file(source,Path('contracts/studio-host-capabilities/service-http-get-preview-20261015')/source.relative_to(CONTRACT))
copy_file(VIEW,'plugins/diagnostics/views/spring-actuator-preview.html')
copy_file(ROOT/'plugins/diagnostics/plugin.json','plugins/diagnostics/plugin.json')
copy_file(ROOT/'plugins/diagnostics/catalog.json','plugins/diagnostics/catalog.json')
copy_file(ROOT/'docs/diagnostics-plugin-coverage.md','docs/diagnostics-plugin-coverage.md')
copy_file(ROOT/'scripts/verify_actuator_view.py','scripts/verify_actuator_view.py')
copy_file(ROOT/'scripts/cdp_client.py','scripts/cdp_client.py')
source_tool=dict(id=tool['id'],tool_version=tool['tool_version'],name=tool['name'],scope=tool['scope'])
destination=BUNDLE/'contracts/studio-host-capabilities/service-http-get-preview-20261015/source-tool.json'
destination.write_text(json.dumps(source_tool,ensure_ascii=False,indent=2)+'\n',encoding='utf-8');files.append(destination)
for label,path,proof,screenshot in [('desktop',*proofs[0]),('mobile',*proofs[1])]:
    copy_file(path,Path('verification')/f'{label}-proof.json')
    copy_file(screenshot,Path('verification')/proof['screenshot'])
    proof_copy=json.loads((BUNDLE/'verification'/f'{label}-proof.json').read_text(encoding='utf-8'))
    proof_copy['screenshot']=f'{label}-{proof_copy["screenshot"]}'
    (BUNDLE/'verification'/f'{label}-proof.json').write_text(json.dumps(proof_copy,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')

readme='''# Spring Actuator service profile View adapter (review bundle)\n\nThis is a **review-only, not-installable** bundle for Studio's proposed `service.http.get.v1@1.0.0-preview.1`. The profile is not implemented or supported. The bundle deliberately has no root plugin manifest and the current diagnostics manifest copied under `plugins/diagnostics/` does not require this profile.\n\nThe View calls only `window.zicode.services.http.get({origin, endpoint}, {signal})`. It has no browser network fallback, WASI network access, credentials, arbitrary path, headers, or request body. Its local validation and schemas are provider proposals. The Host must perform all authoritative URL/DNS/IP checks, user confirmation, one-use authorization, redirect/proxy/TLS policy, response limits, and cancellation.\n\n## Known incompatibilities with standalone Spring Actuator 1.1.1\n\n- Studio's profile is unimplemented and must not be declared in an installable manifest. This bundle is not an integration acceptance result.\n- Only `/actuator/health`, `/actuator/info`, and `/actuator/metrics` are fixed endpoints. A named `metrics/<name>` query is not supported.\n- The profile rejects all credentials. Optional Bearer authentication in the desktop tool is outside this adapter.\n- Origin-only input excludes a Spring context path or custom Actuator base path.\n- Studio's 3 s connect / 8 s total / 1 MiB response budgets are tighter than the desktop tool's 5 s / 20 s / 2 MiB limits.\n- Studio has not frozen the response schema or stable error-code set. The included SDK result schema is a provider proposal only.\n\n## Evidence\n\n`verification/desktop-proof.json` and `verification/mobile-proof.json` record Edge runs at 1180×900 light and 390×844 dark, DPR 1, six SDK-shaped mock calls each, and zero page errors. Screenshots are hash-bound in the source receipt. Tests cover health 200/503, info/metrics, malformed origins, unsupported endpoint, error-code handling, AbortSignal, and suppression of a late result. No HTTP server, DNS, TLS, proxy, Host permission, or confirmation was exercised.\n\nRun the provider View tests from this bundle with Edge or Chrome:\n\n```powershell\npython scripts/verify_actuator_view.py --width 1180 --height 900 --theme light --output target/desktop-proof.json --screenshot target/desktop.png\npython scripts/verify_actuator_view.py --width 390 --height 844 --theme dark --output target/mobile-proof.json --screenshot target/mobile.png\n```\n'''
readme_path=BUNDLE/'README.md';readme_path.write_text(readme,encoding='utf-8');files.append(readme_path)
review=dict(id='com.zicode.devtools.diagnostics-actuator-preview',version='0.0.1-preview.1',status='provider review bundle only; not an installable plugin',hostProfile='service.http.get.v1@1.0.0-preview.1',sourceToolId=tool['id'],sourceToolVersion=tool['tool_version'],manifestRequiredProfiles=manifest['requiredProfiles'],requiresUnimplementedProfile=True,knownIncompatibilities=['No Studio Host implementation or acceptance','Named metrics/<name> is unavailable','Bearer authentication is unavailable','Context/base paths are unavailable','Host request budgets are lower than the standalone tool','SDK result schema and error codes are not frozen'])
review_path=BUNDLE/'review-bundle.json';review_path.write_text(json.dumps(review,ensure_ascii=False,indent=2)+'\n',encoding='utf-8');files.append(review_path)
file_rows=[]
for path in sorted(files):
    data=path.read_bytes();file_rows.append(dict(path=path.relative_to(BUNDLE).as_posix(),size=len(data),sha256=digest(data)))
index=digest(json.dumps(file_rows,ensure_ascii=False,separators=(',',':')).encode())
try:revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
except Exception:revision=None
try:dirty=bool(subprocess.check_output(['git','status','--porcelain'],cwd=ROOT,text=True).strip())
except Exception:dirty=True
receipt=dict(bundle=str(BUNDLE.relative_to(ROOT)),status=review['status'],sourceRevision=revision,sourceTreeDirty=dirty,
             sourceTool=source_tool,upstreamProfile=upstream,pluginManifestSha256=digest((ROOT/'plugins/diagnostics/plugin.json').read_bytes()),
             viewSha256=digest(VIEW.read_bytes()),fixtureFileSha256=digest((CONTRACT/'fixtures/view-cases.json').read_bytes()),
             desktopProof=proofs[0][1],mobileProof=proofs[1][1],files=file_rows,directoryIndexSha256=index,
             externalChecksNotPerformed=['Studio service.http.get Host conformance','Host confirmation and authorization lifecycle',
                 'DNS rebinding/pinning, address classification, TLS, redirects, proxy exclusion, response streaming limits',
                 'Installed plugin lifecycle, revoke/uninstall, Pi/browser network separation'])
RECEIPT.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'Review-only Actuator bundle: {BUNDLE}; {len(file_rows)} files, {sum(row["size"] for row in file_rows)} bytes; receipt={RECEIPT}')
