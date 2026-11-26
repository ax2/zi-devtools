"""Exercise the offline embedding comparison in the existing Windows Studio Worker."""

import argparse
from hashlib import sha256
import json
import math
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PROFILE='resource.wasi.input.draft2'
PLUGIN='com.zicode.devtools.embedding'
CAPABILITY='devtools.embedding.compare-files'
CONTRACT='1.0.0-rc.1'
MAX_FILE=256*1024
MIME='application/octet-stream'
SECRET='SYNTHETIC_EMBEDDING_PRIVATE_METADATA'


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(',',':')).encode('utf-8')


def digest(data):
    return sha256(data).hexdigest()


def metrics(a,b):
    norm_a=math.sqrt(sum(x*x for x in a)); norm_b=math.sqrt(sum(x*x for x in b))
    cosine=max(-1,min(1, sum(x*y for x,y in zip(a,b))/(norm_a*norm_b)))
    return dict(dimensions=len(a),normA=norm_a,normB=norm_b,cosine=cosine,
                euclidean=math.sqrt(sum((x-y)**2 for x,y in zip(a,b))))


def vector(name, inputs, expected, *, mimes=None):
    return dict(name=name,inputs=[item if isinstance(item,bytes) else compact(item) for item in inputs],
                expected=expected,mimes=[MIME]*len(inputs) if mimes is None else mimes)


def response(a,b):
    return dict(model=SECRET,embeddings=[a,b],duration=12000,prompt_eval_count=2)


def padded(raw,size=MAX_FILE):
    extra=size-len(raw)
    assert extra>=0
    whitespace=(b'\r\t\n '*((extra+3)//4))[:extra]
    return whitespace[:extra//3]+raw+whitespace[extra//3:]


def fixtures():
    a=[3,4]; b=[0,2]; expected=metrics(a,b)
    yield vector('two-copied-arrays',[a,b],expected,mimes=['application/json',MIME])
    yield vector('reverse-resource-order',[b,a],metrics(b,a))
    yield vector('single-native-response',[response(a,b)],expected)
    yield vector('same-copied-array',[a,a],metrics(a,a))
    yield vector('orthogonal',[[1,0],[0,1]],metrics([1,0],[0,1]))
    yield vector('negative-collinear',[[1,2],[-2,-4]],metrics([1,2],[-2,-4]))
    yield vector('fraction-and-exponents',[b'[1e-2, -2E1, 0.25]',b'[0.30, -1.0, 2.5e-1]'],metrics([0.01,-20,.25],[.3,-1,.25]))
    yield vector('amplitude-limit',[[1000000,-1000000],[999999,-1000000]],metrics([1000000,-1000000],[999999,-1000000]))
    yield vector('not-finite-overflow',[b'[1e309]',b'[1]'],'INVALID_VECTOR')
    yield vector('beyond-amplitude',[[1000001],[3]],'INVALID_COMPONENT')
    yield vector('below-negative-amplitude',[[3],[-1000001]],'INVALID_COMPONENT')
    yield vector('json-number-as-string',[[SECRET],[1]],'INVALID_VECTOR')
    yield vector('non-numeric-boolean',[[True],[1]],'INVALID_VECTOR')
    yield vector('zero-vector',[[0,-0.0],[1,2]],'INVALID_NORM')
    yield vector('negative-zero-response',[response([1,2],[-0.,0])],'INVALID_NORM')
    yield vector('norm-underflow',[[1e-250],[1]],'INVALID_NORM')
    yield vector('empty-vector',[[],[1]],'DIMENSION_LIMIT')
    yield vector('different-dimensions',[a,[2]],'DIMENSIONS_DIFFER')
    yield vector('second-confirmation-required',[a],'SECOND_VECTOR_REQUIRED')
    yield vector('single-array-with-space',[padded(compact(a))],'SECOND_VECTOR_REQUIRED')
    yield vector('missing-second-component-response',[dict(embeddings=[a],model=SECRET)],'INVALID_EMBEDDINGS')
    yield vector('too-many-response-vectors',[dict(embeddings=[a,b,a],model=SECRET)],'INVALID_EMBEDDINGS')
    yield vector('duplicate-response-key',[b'{"embeddings":[[1],[2]],"embeddings":[[1],[2]]}'],'INVALID_EMBEDDINGS')
    yield vector('unknown-single-file',[{SECRET:SECRET}],'INVALID_EMBEDDINGS')
    yield vector('response-file-in-two-array-mode',[response(a,b),b],'INVALID_VECTOR')
    yield vector('bad-utf8',[b'[1,\xff]',b],'INVALID_VECTOR')
    yield vector('unexpected-utf8-bom',[b'\xef\xbb\xbf[1]',b'[1]'],'INVALID_VECTOR')
    yield vector('wrong-resource-type',[a,b],'UNSUPPORTED_MIME',mimes=['text/markdown',MIME])
    integers=[1+(i%23) for i in range(8192)]
    increasing=[1+(i%13) for i in range(8192)]
    yield vector('8192-integer-elements',[integers,increasing],metrics(integers,increasing))
    large_integers=[-1000000 if i%2==0 else 1000000 for i in range(8192)]
    yield vector('8192-amplitude-integers',[large_integers,large_integers[::-1]],metrics(large_integers,large_integers[::-1]))
    numeric_a=[math.sin(i*0.017+i/255) for i in range(1,8193)]
    numeric_b=[math.cos(i*0.021-i/121) for i in range(1,8193)]
    yield vector('8192-dense-fractional-elements',[numeric_a,numeric_b],'INPUT_TOO_COMPLEX')
    decimal_a=[round(value,6) for value in numeric_a]
    decimal_b=[round(value,6) for value in numeric_b]
    yield vector('8192-native-response-dense',[response(decimal_a,decimal_b)],'INPUT_TOO_COMPLEX')
    short_decimal=[1.0]*4096
    yield vector('4096-short-decimals',[short_decimal,short_decimal],metrics(short_decimal,short_decimal))
    metadata_heavy=response(a,b); metadata_heavy['extra_data']=[0]*17000
    yield vector('unrelated-shape-budget',[metadata_heavy],'INPUT_TOO_COMPLEX')
    yield vector('oversize-native-response',[response(numeric_a,numeric_b)],'INPUT_TOO_LARGE')
    for dim in (1024,1536,2048,3072,4096,6144):
        a_del=numeric_a[:dim]; b_del=numeric_b[:dim]
        scaled_expected=metrics(a_del,b_del) if dim<=4096 else 'INPUT_TOO_COMPLEX'
        yield vector(f'decimal-{dim}-ordered-files',[a_del,b_del],scaled_expected)
        yield vector(f'decimal-{dim}-embedded-response',[response(a_del,b_del)],scaled_expected)
    exact_source=compact(response(numeric_a[:4096],numeric_b[:4096]))
    marks=sum(exact_source.count(bytes([marker])) for marker in b'.eE')
    reserve=352*1024-len(exact_source)-marks*24
    assert reserve>0
    exact_source=exact_source.replace(b'"embeddings":',b'"embeddings":'+b' '*reserve,1)
    yield vector('weighted-work-exact-limit',[exact_source],metrics(numeric_a[:4096],numeric_b[:4096]))
    yield vector('weighted-work-limit-plus-one',[exact_source.replace(b'"embeddings":',b'"embeddings": ',1)],'INPUT_TOO_COMPLEX')
    marks_max=[1.0]*5000
    yield vector('short-decimal-marks-at-limit',[marks_max,marks_max],metrics(marks_max,marks_max))
    yield vector('short-decimal-marks-plus-one',[marks_max+[1.0],marks_max],'INPUT_TOO_COMPLEX')
    yield vector('8193-elements',[integers+[1],increasing],'DIMENSION_LIMIT')
    yield vector('two-exact-256-kib-files',[padded(compact(a)),padded(compact(b))],expected)
    yield vector('single-exact-256-kib-response',[padded(compact(response(a,b)))],expected)
    yield vector('256-kib-plus-one',[padded(compact(a),MAX_FILE+1),b],'INPUT_TOO_LARGE')
    long_literal=b'[1.'+b'0'*(MAX_FILE-4)+b']'
    assert len(long_literal)==MAX_FILE
    yield vector('dense-float-token-at-256-kib',[long_literal,[1]],metrics([1],[1]))
    internal=b'[1,'+b' '*(MAX_FILE-5)+b'2]'
    yield vector('internal-space-at-256-kib',[internal,b],metrics([1,2],b))


def envelopes(case,module):
    material=[dict(resourceId=f'3b6d7b35-6df0-4c2f-8c26-7b7f3ac81ca{index}',mimeType=mime,
                   byteLength=len(data),contentVersion=digest(data))
              for index,(data,mime) in enumerate(zip(case['inputs'],case['mimes'],strict=True))]
    packet=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId='programming',input={})
    local=compact(dict(commandId=None,resourceInputProfile=PROFILE,resources=[dict(index=i,**r) for i,r in enumerate(material)],**packet))
    header=compact(dict(profile=PROFILE,request=dict(workerKind='wasi',workerPath='',**packet),
                        module=dict(byteLength=len(module),sha256=digest(module)),resources=material))
    return local,b'ZIRW\x00\x00\x00\x02'+struct.pack('<I',len(header))+header+module+b''.join(case['inputs'])


def invoke(args,request):
    completed=subprocess.run(args,input=request,capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        decoded=json.loads(completed.stdout)
    except ValueError:
        decoded=dict(stdout=completed.stdout[-200:].decode(errors='replace'),stderr=completed.stderr[-400:].decode(errors='replace'))
    return completed.returncode,decoded


def check(report,case):
    expected=case['expected']
    assert report['contractVersion']==CONTRACT and SECRET.encode() not in compact(report)
    if isinstance(expected,str):
        assert report['ok'] is False and report['data'] is None and report['error']['code']==expected,report
        return
    assert report['ok'] is True and report['error'] is None,report
    actual=report['data']
    assert set(actual)==set(expected)|{'text'}
    for key,value in expected.items():
        assert math.isclose(actual[key],value,rel_tol=1e-11,abs_tol=1e-11),(key,actual[key],value)
    assert len(actual['text'])<512


def compare_native(native,worker):
    if not native['ok'] or not worker['ok']:
        assert native==worker,(native,worker)
        return
    assert native['data']['text']==worker['data']['text'],(native,worker)
    fields=('dimensions','normA','normB','cosine','euclidean')
    for field in fields:
        a=native['data'][field]; b=worker['data'][field]
        assert math.isclose(a,b,rel_tol=2e-14,abs_tol=2e-14),(field,a,b)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ('native','worker','module','evidence','scratch'):
        parser.add_argument('--'+key,type=Path,required=True)
    parser.add_argument('--case',action='append',default=[])
    args=parser.parse_args()
    module=args.module.read_bytes()
    args.scratch.mkdir(parents=True,exist_ok=True)
    cases=[]; failures=[]
    with TemporaryDirectory(dir=args.scratch) as directory:
        for case in fixtures():
            if args.case and case['name'] not in args.case:
                continue
            paths=[]
            for index,raw in enumerate(case['inputs']):
                path=Path(directory)/f'input-{index}.json'
                path.write_bytes(raw)
                paths.append(path)
            local,worker_envelope=envelopes(case,module)
            native_status,native=invoke([args.native,*paths],local)
            worker_status,worker=invoke([args.worker,'--zicode-plugin-worker'],worker_envelope)
            size=[len(raw) for raw in case['inputs']]
            try:
                assert native_status==0 and worker_status==0 and worker['ok'],(native_status,worker_status,worker)
                check(native,case)
                check(worker['result'],case)
                compare_native(native,worker['result'])
                cases.append(dict(case=case['name'],inputBytes=size,inputSha256=[digest(raw) for raw in case['inputs']],
                                  resultBytes=len(compact(worker['result'])),result=worker['result']))
            except (AssertionError,KeyError,TypeError) as failure:
                failures.append(dict(case=case['name'],inputBytes=size,reason=str(failure)[:780]))
    proof=dict(workerProfile=PROFILE,workerSha256=digest(args.worker.read_bytes()),moduleBytes=len(module),
                wasmSha256=digest(module),cases=cases,passed=len(cases),failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(dict(passed=len(cases),failures=failures,moduleBytes=len(module))))
    return int(bool(failures))


if __name__=='__main__':
    raise SystemExit(main())
