"""Freeze the verified offline vector comparison and its external byte receipt."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_embedding_files_plugin import CAPABILITY, CONTRACT, MIME, PLUGIN, PROFILE, check, compact, digest, fixtures

ROOT=Path(__file__).resolve().parents[1]
SLUG='embedding-files'
VERSION='0.1.0'
TITLE='离线向量比较'
SCOPE='确认两份 JSON 向量或一份包含两个 embeddings 的已有响应，离线比较维度、范数、余弦相似度和欧氏距离。'
MAX_RESULT=2048


def manifest():
    nil={'type':'null'}
    contract={'type':'string','const':CONTRACT}
    metric={'type':'number','minimum':0}
    data=obj(dimensions=dict(integer(8192),minimum=1),normA=metric,normB=metric,
             cosine={'type':'number','minimum':-1,'maximum':1},euclidean=metric,text=string(512))
    success=obj(contractVersion=contract,ok={'type':'boolean','const':True},data=data,error=nil)
    failure=obj(contractVersion=contract,ok={'type':'boolean','const':False},data=nil,
                error=obj(code=string(32),message=string(256)))
    cap=dict(id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMax=2,resourceInputMimeTypes=[MIME,'application/json'],
             inputSchema=obj(),outputSchema={'type':'object','oneOf':[success,failure]},maxResultBytes=MAX_RESULT,
             directory=dict(categoryId='data',aliases=['嵌入向量','比较数字向量','余弦相似度','Embedding comparison'],description=SCOPE))
    return dict(schemaVersion=2,id=PLUGIN,name='ZiDevTools Offline Embedding Comparison',displayName=TITLE,
        description=SCOPE,version=VERSION,publisher='zicode',license='MIT',
        engines={'zicode':'>=0.7.2 <1.0.0','pluginApi':'^1.0.0'},
        requiredProfiles={'resource.input.types.v1':'=1.0.0-preview.1','output.schema.v1':'=1.0.0-rc.1',
                          'tool.directory.v1':'=1.0.0-preview.1'},
        activationEvents=['onCapability:'+CAPABILITY],runtime={'wasi':'runtime/tools.wasm','resourceInputProfile':PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[cap]),
        permissions=dict(required=[dict(name='resource.read',reason='读取本次确认的两份向量文件或一份已有响应。')],optional=[]))


def main():
    parser,args=candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error('use a two-digit candidate number')
    package=ROOT/'release/plugin-candidates'/f'{SLUG}-{VERSION}-candidate-{args.candidate}'
    receipt=package.with_name(package.name+'-verification.json')
    if package.exists() or receipt.exists():
        parser.error('package and receipt must be new')
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_bytes())
    fixed=list(fixtures())
    if (not 0<len(module)<=2*1024*1024 or proof['wasmSha256']!=digest(module) or proof['moduleBytes']!=len(module)
            or proof['workerProfile']!=PROFILE or proof['failures'] or proof['passed']!=len(fixed)):
        parser.error('module or Windows Worker proof does not match this candidate')
    receipts=[]
    for row,case in zip(proof['cases'],fixed,strict=True):
        check(row['result'],case)
        sizes=[len(raw) for raw in case['inputs']]
        hashes=[digest(raw) for raw in case['inputs']]
        if (row['case']!=case['name'] or row['inputBytes']!=sizes or row['inputSha256']!=hashes
                or row['resultBytes']!=len(compact(row['result'])) or row['resultBytes']>MAX_RESULT):
            parser.error('fixed case does not match evidence')
        receipts.append(dict(id=case['name'],inputSha256=hashes,inputBytes=sizes,mime=case['mimes'],ok=row['result']['ok']))

    def git(*args):
        return subprocess.check_output(['git',*args],cwd=ROOT,text=True).strip()

    if git('status','--porcelain','--untracked-files=no'):
        parser.error('commit source before freezing')
    (package/'runtime').mkdir(parents=True)
    (package/'runtime/tools.wasm').write_bytes(module)
    shutil.copyfile(ROOT/'LICENSE',package/'LICENSE')
    shutil.copyfile(ROOT/'plugins'/SLUG/'README.md',package/'README.md')
    (package/'NOTICE').write_text('ZiDevTools is MIT. Anyhow, memchr, Serde, serde_json and other dependencies retain their upstream licenses.\n',encoding='utf-8')
    dump(package/'plugin.json',manifest())
    dump(package/'catalog.json',dict(contract='zicode.devtools-plugin/1.0.0-rc.1',packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus='unsigned_provider_candidate_not_host_accepted',tools=[dict(sourceToolId='embedding-playground',
            capabilityId=CAPABILITY,title=TITLE,status='candidate',inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/'fixtures.json',dict(generator='scripts/verify_embedding_files_plugin.py',cases=receipts))
    dump(package/'provenance.json',dict(sourceRevision=git('rev-parse','HEAD'),dirty=False,signature='unsigned',
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget='wasm32-wasip1',wasmProfile='plugin',
        moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
           for path in sorted(package.rglob('*')) if path.is_file()]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(row['size'] for row in files),
        directoryIndexEncoding=dict(hash='SHA-256',encoding='UTF-8',content='files array in listed order',
            objectKeyOrder=['path','size','sha256'],ensureAscii=False,separators=[',',':'],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=='__main__':
    main()
