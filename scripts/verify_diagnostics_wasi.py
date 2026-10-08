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

def request(action, text, secondary=''):
    return dict(pluginId='com.zicode.devtools.diagnostics', sceneId='coding',
                capabilityId='devtools.diagnostics.'+action, commandId=None,
                input=dict(text=text, secondary=secondary))

# Independent observable invariants, rather than merely equal outputs.
samples = [
    ('java.threads', '"worker" #1 nid=0x1\n java.lang.Thread.State: RUNNABLE\n at app.Main.run(Main.java:1)', '', {'threadCount':1}),
    ('java.dependencies', '[INFO] demo:app:jar:1.0\n[INFO] +- org.example:client:jar:2.0:compile', '', {'entries/0/module':'demo:app'}),
    ('java.gc', '[0.002s][info][gc] Using G1\n[1.020s][info][gc] GC(0) Pause Young (Normal) (G1 Evacuation Pause) 24M->3M(256M) 4.500ms', '', {'pauseCount':1,'totalPauseMs':4.5}),
    ('java.jfr_json', '{"recording":{"events":[{"type":"jdk.GarbageCollection","values":{"duration":"PT0.004S"}}]}}', '', {'eventCount':1,'timeline/0/durationSeconds':0.004}),
    ('spring.config', 'server:\n  port: 8080\npassword: synthetic-private-value', 'server:\n  port: 8081\npassword: changed-private-value', {'changes/0/left':'[REDACTED]'}),
    ('django.migrations', '[X] shop.0001\n[ ] shop.0002 ... (shop.0001)', 'DROP TABLE old_orders;', {'migrations/0/id':'shop.0001','dependencies/0/dependsOn':'shop.0001'}),
    ('django.sql', '[{"requestId":"r","sql":"SELECT name FROM users WHERE id=1","durationMs":2.5},{"requestId":"r","sql":"SELECT name FROM users WHERE id=2","durationMs":3}]', '', {'queryCount':2,'totalMs':5.5,'groups/0/count':2}),
    ('django.urls', '[{"route":"users/<int:pk>/","name":"detail","namespace":"api"}]', '{"name":"api:detail","kwargs":{"pk":42}}', {'routeCount':1,'reverseCheck/candidates/0/parameterNamesMatch':True}),
    ('django.openapi', '{"openapi":"3.0.3","paths":{"/users/":{"get":{"responses":{"200":{"description":"OK"}}}}}}', '{"openapi":"3.0.3","paths":{}}', {'operationChanges/0/change':'removed'}),
    ('django.checks', '?: (security.W018) DEBUG enabled', '', {'items/0/id':'security.W018'}),
    ('celery.report', 'Task demo.send[task-1] received\nTask demo.send[task-1] retry: Retry in 1s\nTask demo.send[task-1] succeeded in 0.25s: None', '', {'tasks/0/retries':1,'tasks/0/lastObservedState':'succeeded'}),
]
cases = [(action, request(action,text,second), checks, None) for action,text,second,checks in samples]
registry=set(re.findall(r'Action\s*\{\s*id:\s*"([^"]+)"', (root/'crates/zi-diagnostics-core/src/lib.rs').read_text(encoding='utf-8')))
assert registry=={row[0] for row in samples}, 'Every registered action needs an observable success fixture'
schema=json.loads((root/'contracts/diagnostics/v1/request.schema.json').read_text(encoding='utf-8'))
assert set(schema['properties']['capabilityId']['enum'])=={'devtools.diagnostics.'+action for action in registry}
base = request('django.checks', '?: (security.W018) DEBUG enabled')
for field in ['pluginId','sceneId','capabilityId','commandId','input']:
    value=copy.deepcopy(base);del value[field]
    cases.append(('missing-'+field,value,None,'INVALID_INPUT'))
for field,value,error in [('pluginId','other','INVALID_INPUT'),('sceneId','','INVALID_INPUT'),
                          ('capabilityId','devtools.diagnostics.unknown','UNSUPPORTED_OPERATION'),
                          ('commandId','other','INVALID_INPUT')]:
    case=copy.deepcopy(base);case[field]=value
    cases.append(('invalid-'+field,case,None,error))
case=copy.deepcopy(base);case['capabilityId']=None;case['commandId']='other'
cases.append(('command-not-registered',case,None,'UNSUPPORTED_OPERATION'))
for field in ['text','secondary']:
    case=copy.deepcopy(base);case['input'][field]='中'*2731
    cases.append(('utf8-limit-'+field,case,None,'INPUT_TOO_LARGE'))
case=copy.deepcopy(base);case['input']['extra']='secret-test'
cases.append(('unknown-input-field',case,None,'INVALID_INPUT'))
cases += [('request-limit',b' '*(48*1024+1),None,'INPUT_TOO_LARGE'),
          ('invalid-utf8',b'\xff',None,'INVALID_INPUT'),
          ('invalid-report',request('java.jfr_json','secret-test'),None,'INVALID_REPORT')]
encoded=json.dumps(base,separators=(',',':')).encode()
cases.append(('duplicate-envelope',encoded.replace(b'"input":',b'"pluginId":"other","input":'),None,'INVALID_INPUT'))
cases.append(('output-expansion',request('java.threads',''.join(f'"t{i}" #1\n' for i in range(500))),None,'OUTPUT_TOO_LARGE'))

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
