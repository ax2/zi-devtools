"""Compare resource-sized native/WASI JSON actions against independent exact bytes."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import struct
import time

PLUGIN='com.zicode.devtools.json-file'
PROFILE='resource.wasi.json-file-transform.v1'
digest=lambda data: hashlib.sha256(data).hexdigest()
def invoke_worker(executable, module, capability, params, files, **override):
    request = dict(workerPath="", workerKind="wasi", pluginId=PLUGIN, capabilityId=capability,
                   sceneId="programming", executionProfile=PROFILE, input=params,
                   resourceOutputProfile="resource.wasi.output.draft1")
    request.update(override)
    frame = compact(dict(profile="resource.wasi.input.draft2", request=request,
                         module=dict(byteLength=len(module), sha256=digest(module)),
                         resources=[dict(resourceId=f"aaaaaaaa-aaaa-4aaa-8aaa-{index:012d}",
                                         contentVersion=digest(data), mimeType="application/octet-stream",
                                         byteLength=len(data)) for index, data in enumerate(files)]))
    result = subprocess.run([str(executable), "--zicode-plugin-worker"],
                            input=b"ZIRW\0\0\0\x02" + struct.pack("<I", len(frame)) + frame + module + b"".join(files),
                            capture_output=True, timeout=15, creationflags=subprocess.CREATE_NO_WINDOW)
    data = result.stdout
    if data[:8] != b"ZIWO\0\0\0\x01":
        assert result.returncode != 0, (result.returncode, data[:512])
        return json.loads(data), None, False
    assert result.returncode == 0
    end = 12 + struct.unpack_from("<I", data, 8)[0]
    header, body = json.loads(data[12:end]), data[end:]
    assert header["schemaVersion"] == 1 and header["profile"] == "resource.wasi.output.draft1"
    if header["resources"]:
        assert header["resources"] == [
            dict(index=0, mimeType="application/json", byteLength=len(body), contentVersion=digest(body))
        ]
        return header["result"], body, True
    assert not body
    return header["result"], None, True


LIMIT=1024*1024
MAX=8*LIMIT

def pretty(value): return json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2).encode()
def compact(value): return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
def sha(value): return hashlib.sha256(value).hexdigest()

def fixtures():
    def case(name,action,data,expected=None,error=None):
        return name, 'devtools.json-file.'+action, data, expected, error
    sample={'a':[1,True,None,'字🙂'], 'b':-2.5}
    for action in ['format','minify','validate']:
        expected=pretty(sample) if action=='format' else compact(sample) if action=='minify' else b'{"valid":true}'
        yield case('unicode-'+action,action,compact(sample),expected)
        for name,data in [('duplicate',b'{"x":1,"\\u0078":2}'),('over-u64',b'18446744073709551616'),
            ('long-decimal',b'0.123456789012345678901234567890'),('underflow',b'1e-999'),
            ('malformed',b'{'),('invalid-utf8',b'"\xff"')]:
            yield case(name+'-'+action, action,data,error='INVALID_UTF8' if name=='invalid-utf8' else 'INVALID_JSON' if name=='malformed' else 'JSON_POLICY_REJECTED')
    for data in [b'0',b'false',b'null',b'[]',b'"number 18446744073709551616"',b'0.125', b'1.2300', b'1.25e3']:
        expected={b'1.2300':b'1.23',b'1.25e3':b'1250.0'}.get(data,data)
        yield case('scalar-'+data.decode(),'minify',data,expected)
    for action in ['format','minify','validate']:
        data=b'"'+b'x'*(LIMIT-2)+b'"'
        yield case('exact-1mib-'+action,action,data,b'{"valid":true}' if action=='validate' else data)
    yield case('over-1mib','format', b'0'+b' '*LIMIT,error='INPUT_TOO_LARGE')
    for depth in [64,65]:
        data=b'['*depth+b'0'+b']'*depth
        yield case('depth-'+str(depth),'minify',data,data if depth==64 else None,error='JSON_POLICY_REJECTED' if depth==65 else None)
    # Dense 1MiB JSON (524287 scalars) tests the actual Worker memory/fuel envelope.
    dense=b'['+b'0,'*524286+b'0] '
    assert len(dense)==LIMIT
    yield case('dense-exact-1mib','minify',dense,dense.rstrip())
    value=['']+[0]*60000
    for _ in range(63): value=[value]
    size=len(pretty(value))
    pad=MAX-size
    assert pad>0
    bottom=value
    for _ in range(63): bottom=bottom[0]
    bottom[0]='x'*pad
    expected=pretty(value)
    assert len(expected)==MAX and len(compact(value))<LIMIT
    yield case('exact-8mib-format','format',compact(value),expected)
    bottom[0]+='x'
    assert len(pretty(value))==MAX+1
    yield case('one-byte-over-output','format',compact(value),error='RESULT_TOO_LARGE')

def main():
    parser=argparse.ArgumentParser()
    for key in ['worker','native','module','output']: parser.add_argument('--'+key,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    result={'workerSha256':sha(args.worker.read_bytes()),'moduleSha256':sha(module),'cases':[]}
    for name,capability,data,expected,error in fixtures():
        start=time.monotonic()
        fixture=compact(dict(capabilityId=capability,input={},filesBase64=[base64.b64encode(data).decode()]))
        proc=subprocess.run([str(args.native)],input=fixture,capture_output=True,timeout=20,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
        native=json.loads(proc.stdout)
        report,output,binary=invoke_worker(args.worker,module,capability,{},[data])
        if error:
            assert native['ok'] is False and native['error']['code']==error and native['error']['message'], (name,native)
            assert output is None, (name,report)
            if error=='INPUT_TOO_LARGE' and not binary:
                assert report=={'ok':False,'result':None,'error':'resource_input_too_large'}, (name,report)
            else:
                assert report==native, (name,report)
        else:
            native_output=base64.b64decode(native['outputBase64'])
            assert sha(native_output)==sha(expected) and len(native_output)==len(expected), (name,len(native_output),len(expected))
            assert output==expected, (name,len(output or b''),len(expected),report)
            assert report==native['report'],(name,report,native['report'])
        row=dict(name=name,inputBytes=len(data),inputSha256=sha(data),outputBytes=len(output or b''),
                 outputSha256=sha(output) if output is not None else None,error=error,seconds=round(time.monotonic()-start,3))
        result['cases'].append(row)
        print(json.dumps(row,ensure_ascii=False),flush=True)
    for patch in [dict(pluginId='unrelated.plugin'),dict(executionProfile='resource.wasi.json-file-compare.v1')]:
        report,output,binary=invoke_worker(args.worker,module,'devtools.json-file.format',{},[b'0'],**patch)
        assert output is None and (not binary or not report.get('ok')),report
    result['policyChecks']=2
    result['verified']=True
    args.output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(dict(verified=True,cases=len(result['cases']),policyChecks=2)))

if __name__=='__main__': main()
