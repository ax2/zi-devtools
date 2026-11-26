"""Freeze the real Windows Worker candidate for a local declarative manifest preview."""

from datetime import datetime, timezone
import json
from pathlib import Path
import re
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from verify_plugin_manifest_preview import CAPABILITY, CONTRACT, MAX_RESULT, PLUGIN, PROFILE, check, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = 'plugin-manifest-preview'
VERSION = '0.1.0'
TITLE = 'ZiDevTools 插件清单预览'
SCOPE = '逐次确认后核对一份本机 ZiDevTools 声明式 JSON 清单，查看包身份、原字节 SHA-256 和按序工具声明。'


def manifest():
    nil = {'type':'null'}
    boolean = {'type':'boolean'}
    contract = {'type':'string','const':CONTRACT}

    def nullable(field):
        return {'oneOf':[field,nil]}

    def ordinal():
        return dict(integer(64), minimum=1)

    adapter = {'type':'object','oneOf':[
        obj(type={'type':'string','const':'builtin'},tool=string(64),action=integer(64)),
        obj(type={'type':'string','const':'http'},method={'type':'string','enum':['GET','POST']}),
    ]}
    row = obj(ordinal=ordinal(),id=string(64),name=string(128),category=string(32),
              version=nullable(string(32)),aliasCount=integer(16),historyCount=integer(64),adapter=adapter)
    preview = obj(schemaVersion={'type':'integer','const':1},id=string(64),name=string(128),version=string(40),
                  totalTools=ordinal(),builtinTools=integer(64),httpTools=integer(64),toolStart=ordinal(),
                  nextToolStart=nullable(ordinal()),tools={'type':'array','minItems':1,'maxItems':8,'items':row})
    data = obj(preview=preview,byteLength=integer(262144),sha256=string(64),text=string(4096))
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=data,error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    start = dict(ordinal(),title='工具起始序号（可选）',description='默认从第 1 项开始，每次按原清单顺序查看最多 8 项。')
    capability = dict(id=CAPABILITY,title=TITLE,description=SCOPE,
        resourceInputMimeTypes=['application/octet-stream','application/json'],
        inputSchema=dict(obj(toolStart=start),required=[]),
        outputSchema={'type':'object','oneOf':[success,failure]},maxResultBytes=MAX_RESULT,
        directory=dict(categoryId='files',aliases=['插件清单预检','ZiDevTools 声明式插件','核对插件 SHA-256','Plugin manifest preview'],description=SCOPE))
    return dict(schemaVersion=2,id=PLUGIN,name='ZiDevTools Declarative Manifest Preview',displayName=TITLE,
        description=SCOPE,version=VERSION,publisher='zicode',license='MIT',
        engines={'zicode':'>=0.7.2 <1.0.0','pluginApi':'^1.0.0'},
        requiredProfiles={'resource.input.types.v1':'=1.0.0-preview.1','output.schema.v1':'=1.0.0-rc.1',
                          'tool.directory.v1':'=1.0.0-preview.1'},
        activationEvents=['onCapability:'+CAPABILITY],runtime={'wasi':'runtime/tools.wasm','resourceInputProfile':PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[capability]),
        permissions=dict(required=[dict(name='resource.read',reason='仅读取本次确认提供的 ZiDevTools 声明式 JSON 清单。')],optional=[]))


def license_set(registry):
    tree = subprocess.check_output(['cargo','tree','--locked','--offline','-p','zi-plugin-manifest-wasi',
        '--target','wasm32-wasip1','--edges','normal,build','--prefix','none','--format','{p};{l}'],cwd=ROOT).decode()
    result={}
    for line in tree.splitlines():
        match=re.fullmatch(r'([\w-]+) v([\w.+-]+)(?: \(proc-macro\))?;(.+?)(?: \(\*\))?',line)
        if match:
            name, version, license=match.groups()
            base=registry/f'{name}-{version}'
            texts=sorted(file for file in base.iterdir() if file.is_file() and file.name.upper().startswith(('LICENSE','COPYING')))
            assert texts, f'missing upstream license for {name} {version}'
            result[(name,version)]=(license,texts)
    assert any(name.startswith('icu_') for name,version in result), 'ICU transitive notice required'
    return result


def main():
    import argparse

    parser=argparse.ArgumentParser(description=__doc__)
    for key in ('module','evidence','crate-sources'):
        parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--candidate',default='01')
    args=parser.parse_args()
    if not re.fullmatch(r'[0-9]{2}',args.candidate):
        parser.error('use a two-digit candidate number')
    package=ROOT/'release/plugin-candidates'/f'{SLUG}-{VERSION}-candidate-{args.candidate}'
    receipt=package.with_name(package.name+'-verification.json')
    if package.exists() or receipt.exists():
        parser.error('package and receipt must be new')
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_bytes())
    vectors=list(fixtures())
    if (not 0<len(module)<=2*1024*1024 or proof['wasmSha256']!=digest(module) or proof['moduleBytes']!=len(module)
            or proof['workerProfile']!=PROFILE or proof['failures'] or proof['passed']!=len(vectors)):
        parser.error('module or Windows Worker proof does not match the candidate')
    for row, case in zip(proof['cases'],vectors,strict=True):
        check(row['result'],case)
        if (row['case']!=case['name'] or row['inputSha256']!=digest(case['raw']) or row['inputBytes']!=len(case['raw'])
                or row['resultBytes']!=len(compact(row['result']))):
            parser.error('fixed case does not match the receipt')

    def git(*args):
        return subprocess.check_output(['git',*args],cwd=ROOT,text=True).strip()

    if git('status','--porcelain','--untracked-files=no'):
        parser.error('commit source before freezing')
    licenses=license_set(args.crate_sources)
    (package/'runtime').mkdir(parents=True)
    (package/'runtime/tools.wasm').write_bytes(module)
    shutil.copyfile(ROOT/'LICENSE',package/'LICENSE')
    shutil.copyfile(ROOT/'plugins'/SLUG/'README.md',package/'README.md')
    notices=['ZiDevTools MIT. Cargo target dependency licenses (including build dependencies):']
    for (name,version),(license,texts) in sorted(licenses.items()):
        folder=package/'licenses'/f'{name}-{version}'
        folder.mkdir(parents=True)
        notices.append(f'{name} {version}: {license}')
        for item in texts:
            shutil.copyfile(item,folder/item.name)
    (package/'NOTICE').write_bytes(('\n'.join(notices)+'\nRust standard library: MIT OR Apache-2.0.\n').encode())
    dump(package/'plugin.json',manifest())
    dump(package/'catalog.json',dict(contract='zicode.devtools-plugin/1.0.0-rc.1',packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus='unsigned_provider_candidate_not_host_accepted',tools=[dict(sourceToolId='plugins',capabilityId=CAPABILITY,
            title=TITLE,status='candidate',inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/'fixtures.json',dict(generator='scripts/verify_plugin_manifest_preview.py',cases=[dict(id=case['name'],
        inputSha256=digest(case['raw']),inputBytes=len(case['raw']),toolStart=case['start'],mime=case['mime']) for case in vectors]))
    dump(package/'provenance.json',dict(sourceRevision=git('rev-parse','HEAD'),dirty=False,signature='unsigned',
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget='wasm32-wasip1',wasmProfile='plugin',
        moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=file.relative_to(package).as_posix(),size=file.stat().st_size,sha256=digest(file.read_bytes()))
           for file in sorted((p for p in package.rglob('*') if p.is_file()),key=lambda p:p.relative_to(package).as_posix())]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item['size'] for item in files),
        directoryIndexEncoding=dict(hash='SHA-256',encoding='UTF-8',content='files array in listed order',
            objectKeyOrder=['path','size','sha256'],ensureAscii=False,separators=[',',':'],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=='__main__':
    main()
