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
from fields_vectors import rows, version
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
