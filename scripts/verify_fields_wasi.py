"""Experimental proposal.2 JSON adapter functional parity only, not Host or fuel proof."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from jsonschema import Draft202012Validator
ROOT=Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native',type=Path,default=ROOT/'target/debug/zi-fields-wasi.exe')
parser.add_argument('--wasm',type=Path,default=ROOT/'target/wasm32-wasip1/plugin/zi-fields-wasi.wasm')
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--fixtures',type=Path)
args=parser.parse_args()
version='1.1.0-proposal.2'
def raw(value):return json.dumps(value,ensure_ascii=False,separators=(',',':')).encode('utf-8')
def request(cap,input):return raw(dict(contractVersion=version,pluginId='com.zicode.devtools.compare',sceneId='coding',capabilityId='devtools.compare.'+cap,commandId=None,input=input))
def success(text):return raw(dict(contractVersion=version,ok=True,data=dict(text=text),error=None))
def failure(code,message):return raw(dict(contractVersion=version,ok=False,data=None,error=dict(code=code,message=message)))
def pretty(value):return json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)
def diff(equal,changes,unordered):return pretty(dict(equal=equal,changes=changes,arrayOrder='ignored (duplicates retained)' if unordered else 'by index'))
rows=[
 ('query',request('json.path',dict(text='{"项":[1,2]}',query='$.项[*]')),success(pretty([1,2]))),
 ('large-index',request('json.path',dict(text='[1]',query='$[4294967296]')),success('[]')),
 ('u64-max',request('json.path',dict(text='[1]',query='$[18446744073709551615]')),success('[]')),
 ('wide-integer',request('json.path',dict(text='[9007199254740993]',query='$[0]')),success(pretty([9007199254740993]))),
 ('ordered',request('json.diff.ordered',dict(left='[1,2]',right='[2,1]')),success(diff(False,[dict(path='/0',kind='changed',before=1,after=2),dict(path='/1',kind='changed',before=2,after=1)],False))),
 ('unordered',request('json.diff.unordered',dict(left='[1,2]',right='[2,1]')),success(diff(True,[],True))),
 ('duplicates',request('json.diff.unordered',dict(left='[1,1,2]',right='[1,2,2]')),success(diff(False,[dict(path='',kind='changed',before=[1,1,2],after=[1,2,2])],True))),
 ('missing-query',request('json.path',dict(text='{}')),failure('INVALID_INPUT','查询输入结构无效')),
 ('query-byte-over',request('json.path',dict(text='{}',query='中'*1366)),failure('INPUT_TOO_LARGE','查询字段超过 UTF-8 字节限制')),
 ('unsupported-draft-operation',request('regex.matches',dict(text='a',pattern='a')),failure('UNSUPPORTED_OPERATION','该实验适配未注册此能力')),
]
old=request('json.path',dict(text='{}',query='$')).replace(version.encode(),b'1.0.0-rc.1')
rows.append(('old-contract',old,failure('INVALID_INPUT','协议、插件或场景标识无效')))
rows.append(('invalid-utf8',b'\xff',failure('INVALID_ENCODING','请求不是有效 UTF-8')))
rows.append(('exact-text-bytes',request('json.path',dict(text='{}'+' '*8190,query='$')),success(pretty([{}]))))
rows.append(('text-byte-over',request('json.path',dict(text='{}'+' '*8191,query='$')),failure('INPUT_TOO_LARGE','查询字段超过 UTF-8 字节限制')))
rows.append(('exact-query-bytes',request('json.path',dict(text='{}',query='$.'+'a'*4094)),success('[]')))
rows.append(('exact-path-steps',request('json.path',dict(text='{}',query='$'+'.a'*128)),success('[]')))
rows.append(('path-steps-over',request('json.path',dict(text='{}',query='$'+'.a'*129)),failure('INVALID_INPUT','输入无法按该操作处理')))
rows.append(('request-byte-over',b' '*(49152+1),failure('INPUT_TOO_LARGE','请求超过 48 KiB')))
prefix='x'*1000
left=json.dumps({prefix:{f'key{i}':0 for i in range(100)}},separators=(',',':'))
right=json.dumps({prefix:{f'key{i}':1 for i in range(100)}},separators=(',',':'))
rows.append(('output-byte-over',request('json.diff.ordered',dict(left=left,right=right)),failure('INPUT_TOO_LARGE','结果超过 48 KiB')))
validator=Draft202012Validator(json.loads((ROOT/'proposals/studio-fields/v0.1.1/result.schema.json').read_text(encoding='utf-8')))
proof=[]
for name,body,expected in rows:
 for kind,cmd in [('native',[str(args.native)]),('wasi',['node',str(ROOT/'scripts/run_text_wasi.mjs'),str(args.wasm)])]:
  completed=subprocess.run(cmd,input=body,capture_output=True,timeout=20)
  assert completed.returncode==0,(name,kind,completed.stderr.decode(errors='replace'))
  assert completed.stdout==expected,(name,kind,completed.stdout,expected)
  validator.validate(json.loads(completed.stdout))
 proof.append(dict(id=name,passed=True,requestSha256=hashlib.sha256(body).hexdigest(),resultSha256=hashlib.sha256(expected).hexdigest()))
value=dict(contractVersion=version,experimentalOperations=3,cases=proof,wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),limitations=[f'{len(rows)} functional vectors only; not complete adversarial/budget/cold verification','Experimental proposal.2 only; no frozen rc.1 fallback, plugin View, package, negotiated Host or Pi acceptance','Node is not a fuel or production isolation proof'])
if args.fixtures:
 args.fixtures.mkdir(parents=True,exist_ok=False)
 portable=[]
 for index,(name,body,expected) in enumerate(rows):
  stem=f'{index:03d}-{name}'
  (args.fixtures/(stem+'.request')).write_bytes(body)
  (args.fixtures/(stem+'.expected.json')).write_bytes(expected)
  envelope=json.loads(expected)
  portable.append(dict(id=name,requestFile=stem+'.request',expectedFile=stem+'.expected.json',requestSha256=hashlib.sha256(body).hexdigest(),expectedSha256=hashlib.sha256(expected).hexdigest(),checks={},expectedError=None if envelope['ok'] else envelope['error']['code']))
 (args.fixtures/'fuel-proof.json').write_text(json.dumps(dict(status='Portable functional expectations only; not budget proof',cases=portable),ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
args.output.parent.mkdir(parents=True,exist_ok=True)
args.output.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'PASS {len(proof)} actual native/WASI proposal.2 JSON vectors and result schemas; experimental only')
