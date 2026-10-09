"""Exercise real native/WASI report adapters; no Studio acceptance claim."""
import argparse
import copy
import hashlib
import json
import re
from pathlib import Path
import subprocess
import time

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--native', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()

from diagnostics_vectors import cases, samples

wasm=args.wasm.read_bytes()
assert wasm[:8]==b'\0asm\x01\0\0\0'
def leb(data, at):
    value=shift=0
    while True:
        byte=data[at];at+=1;value|=(byte&127)<<shift
        if byte<128:return value,at
        shift+=7;assert shift<=35
at=8;memory_max=None
while at<len(wasm):
    section=wasm[at];size,start=leb(wasm,at+1);at=start+size
    if section==5:
        count,pos=leb(wasm,start);assert count==1
        flags,pos=leb(wasm,pos);assert flags==1
        minimum,pos=leb(wasm,pos);memory_max,pos=leb(wasm,pos)
        assert minimum<=memory_max==1024
assert memory_max==1024
results=[]
for name,value,checks,error in cases:
    data=value if isinstance(value,bytes) else json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()
    start=time.monotonic()
    native=subprocess.run([str(args.native)],input=data,capture_output=True,timeout=8,check=True)
    wasi=subprocess.run(['node','--disable-warning=ExperimentalWarning',str(root/'scripts/run_text_wasi.mjs'),str(args.wasm)],input=data,capture_output=True,timeout=8,check=True)
    assert native.stdout==wasi.stdout,(name,'byte parity')
    assert not native.stderr and len(wasi.stdout)<=48*1024,name
    output=json.loads(wasi.stdout)
    assert set(output)=={'contractVersion','ok','data','error'} and output['contractVersion']=='1.0.0-rc.1',(name,output)
    if error:
        assert not output['ok'] and output['data'] is None and output['error']['code']==error,(name,output)
        assert 'secret-test' not in output['error']['message'],name
    else:
        assert output['ok'] and output['error'] is None,(name,output)
        report=json.loads(output['data']['text'])
        for path,expected in checks.items():
            actual=report
            for part in path.split('/'):
                actual=actual[int(part)] if isinstance(actual,list) else actual[part]
            assert actual==expected,(name,path,actual,expected)
        if name=='spring.config':
            assert 'synthetic-private-value' not in output['data']['text'] and 'changed-private-value' not in output['data']['text']
    telemetry=json.loads(wasi.stderr)
    assert telemetry['exitCode']==0 and telemetry['memoryBytes']<=64*1024*1024,name
    results.append(dict(id=name,requestSha256=hashlib.sha256(data).hexdigest(),resultSha256=hashlib.sha256(wasi.stdout).hexdigest(),requestBytes=len(data),resultBytes=len(wasi.stdout),elapsedMs=round((time.monotonic()-start)*1000),**telemetry))
args.output.parent.mkdir(parents=True,exist_ok=True)
args.output.write_text(json.dumps(dict(wasmBytes=len(wasm),wasmSha256=hashlib.sha256(wasm).hexdigest(),nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),memoryMaxBytes=memory_max*65536,cases=results,limitations=['Node functional runtime only; no production sandbox/fuel guarantee','Studio installation/View/Host acceptance untested','8-second subprocess verification guard is not Host timeout acceptance']),ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'PASS {len(results)} native/WASI byte-parity cases; module {len(wasm)} bytes')
