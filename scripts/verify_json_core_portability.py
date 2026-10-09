"""Execute fixed core vectors on native64 and actual WASI32; not a plugin adapter or Host proof."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native', type=Path, default=ROOT/'target/debug/examples/portability.exe')
parser.add_argument('--wasm', type=Path, default=ROOT/'target/wasm32-wasip1/plugin/examples/portability.wasm')
parser.add_argument('--output', type=Path)
args = parser.parse_args()

def pretty(value):
    return json.dumps(value,ensure_ascii=False,sort_keys=True,indent=2)

def report(equal, changes, unordered=False):
    return pretty(dict(equal=equal,changes=changes,arrayOrder='ignored (duplicates retained)' if unordered else 'by index'))

expected = [
    dict(id='path-unicode',outcome=dict(text=pretty([1,2]))),
    dict(id='path-u32-plus',outcome=dict(text='[]')),
    dict(id='path-u64-max',outcome=dict(text='[]')),
    dict(id='path-u64-overflow',outcome=dict(error='数组索引过大')),
    dict(id='path-duplicate-key',outcome=dict(text=pretty([2]))),
    dict(id='path-wide-integer',outcome=dict(text=pretty([9007199254740993]))),
    dict(id='diff-ordered',outcome=dict(text=report(False,[dict(path='/0',kind='changed',before=1,after=2),dict(path='/1',kind='changed',before=2,after=1)]))),
    dict(id='diff-unordered',outcome=dict(text=report(True,[],True))),
    dict(id='diff-duplicates',outcome=dict(text=report(False,[dict(path='',kind='changed',before=[1,1,2],after=[1,2,2])],True))),
    dict(id='diff-pointer',outcome=dict(text=report(False,[dict(path='/a~1b~0',kind='changed',before=1,after=None)]))),
]
outputs=[]
for command,width in [([str(args.native)],64),(['node',str(ROOT/'scripts/run_text_wasi.mjs'),str(args.wasm)],32)]:
    process=subprocess.run(command,input=b'',capture_output=True,timeout=20)
    assert process.returncode==0,process.stderr.decode('utf-8',errors='replace')
    value=json.loads(process.stdout.decode('utf-8'))
    assert value['pointerWidth']==width
    assert value['cases']==expected,[(row['id'],row['outcome']) for row in value['cases'] if row not in expected]
    outputs.append(dict(pointerWidth=width,stdoutSha256=hashlib.sha256(process.stdout).hexdigest(),cases=value['cases']))
proof=dict(cases=len(expected),passed=True,nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),wasmSha256=hashlib.sha256(args.wasm.read_bytes()).hexdigest(),runs=outputs,
    limitations=['Fixed algorithm fixture only; not a general WASI adapter, Host protocol or package','Node functional execution supplies no fuel/production isolation or five-second cold deadline acceptance'])
if args.output:
    args.output.parent.mkdir(parents=True,exist_ok=True)
    args.output.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'PASS {len(expected)} independent expected core vectors: native64 and actual WASI32; not Host acceptance')
