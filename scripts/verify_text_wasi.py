"""Run real WASI/native adapters with identical bytes. No sibling repo/runtime deps."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import time

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--wasm', type=Path, required=True)
parser.add_argument('--native', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
wasm = args.wasm.read_bytes()
assert len(wasm) <= 2 * 1024 * 1024
# Inspect the actual module's memory declaration rather than trusting build flags.
assert wasm[:8] == b'\0asm\x01\0\0\0'
def leb(data, at):
    value=shift=0
    while True:
        byte=data[at];at+=1;value|=(byte&127)<<shift
        if byte<128:return value,at
        shift+=7
        assert shift<=35
at=8
memory_max=None
while at<len(wasm):
    section=wasm[at];size,start=leb(wasm,at+1);at=start+size
    if section==5:
        count,pos=leb(wasm,start);assert count==1
        flags,pos=leb(wasm,pos);assert flags==1
        minimum,pos=leb(wasm,pos);memory_max,pos=leb(wasm,pos)
        assert minimum<=memory_max==1024
assert memory_max==1024

def request(cap, value):
    return dict(pluginId='com.zicode.devtools.text', sceneId='coding', capabilityId=cap, commandId=None, input=value)

cases = []
for case in json.loads((root/'contracts/studio-devtools/v1/fixtures.json').read_text(encoding='utf-8'))['cases']:
    value = case.get('input')
    if value is None:
        gen = case['inputGenerator']
        value = {'text':gen['textRepeat']*gen['count']}
    expected = case.get('expectedText')
    if 'expectedRule' in case:
        expected = base64.b64encode(value['text'].encode()).decode()
    cases.append((case['id'], request(case['capabilityId'], value), expected, case.get('expectedError')))
for field in ['pluginId','sceneId','capabilityId','commandId','input']:
    value = request('devtools.text.sha256', {'text':'secret-test'})
    del value[field]
    cases.append(('missing-'+field,value,None,'INVALID_INPUT'))
for value in [' YQ==', 'YQ==\n', 'YQ', 'YR==', '_w==']:
    cases.append(('strict-base64-'+str(len(cases)),request('devtools.text.base64.decode',{'text':value}),None,'INVALID_ENCODING'))
for value in ['9007199254740992.0','9.007199254740992e15','1e100','{"a":{"x":1,"\\u0078":2}}','['*65+'0'+']'*65]:
    cases.append(('strict-json-'+str(len(cases)),request('devtools.text.json.minify',{'text':value}),None,'INVALID_INPUT'))
near = '['*60 + ','.join(['0']*320) + ']'*60
cases.append(('large-output',request('devtools.text.json.format',{'text':near}),json.dumps(json.loads(near),ensure_ascii=False,indent=2),None))
over = '['*60 + ','.join(['0']*750) + ']'*60
cases.append(('output-expansion-limit',request('devtools.text.json.format',{'text':over}),None,'INPUT_TOO_LARGE'))
cases.append(('request-byte-limit',b' '*(48*1024+1),None,'INPUT_TOO_LARGE'))
cases.append(('invalid-utf8',b'\xff',None,'INVALID_INPUT'))
cases.append(('duplicate-envelope',b'{"pluginId":"com.zicode.devtools.text","pluginId":"other","sceneId":"coding","capabilityId":"devtools.text.sha256","commandId":null,"input":{"text":"abc"}}',None,'INVALID_INPUT'))

results = []
for name, value, expected, error in cases:
    data = value if isinstance(value,bytes) else json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()
    start = time.monotonic()
    native = subprocess.run([str(args.native)],input=data,capture_output=True,timeout=5,check=True)
    wasi = subprocess.run(['node','--disable-warning=ExperimentalWarning',str(root/'scripts/run_text_wasi.mjs'),str(args.wasm)],input=data,capture_output=True,timeout=5,check=True)
    assert native.stdout == wasi.stdout, name+' byte parity'
    assert not native.stderr, name+' native stderr'
    assert len(wasi.stdout) <= 48*1024, name+' result limit'
    output = json.loads(wasi.stdout)
    assert set(output) == {'contractVersion','ok','data','error'} and output['contractVersion']=='1.0.0-rc.1', name
    if error:
        assert output['ok'] is False and output['data'] is None and output['error']['code']==error, (name,output)
        assert 1 <= len(output['error']['message']) <= 240 and 'secret-test' not in output['error']['message'],name
    else:
        assert output['ok'] is True and output['error'] is None and output['data']=={'text':expected},name
    telemetry = json.loads(wasi.stderr)
    assert telemetry['exitCode']==0 and telemetry['memoryBytes']<=64*1024*1024,name
    results.append(dict(id=name,requestBytes=len(data),resultBytes=len(wasi.stdout),elapsedMs=round((time.monotonic()-start)*1000),**telemetry))
args.output.parent.mkdir(parents=True,exist_ok=True)
args.output.write_text(json.dumps(dict(wasmBytes=len(wasm),wasmSha256=hashlib.sha256(wasm).hexdigest(),memoryMaxBytes=memory_max*65536,nativeSha256=hashlib.sha256(args.native.read_bytes()).hexdigest(),cases=results,limitations=['Node functional parity only; Studio installation/UI/Pi Broker untested','Node is not a secure sandbox; no fuel or Host timeout acceptance','5-second subprocess timeout is a verification guard, not Host acceptance']),ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(f'PASS {len(results)} native/WASI byte-parity cases; module {len(wasm)} bytes; maximum result {max(r["resultBytes"] for r in results)} bytes')
