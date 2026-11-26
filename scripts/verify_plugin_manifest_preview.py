"""Check the local-only ZiDevTools manifest preview in the actual Windows Worker."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN = 'com.zicode.devtools.plugin-manifest-preview'
CAPABILITY = 'devtools.plugin-manifest.preview'
PROFILE = 'resource.wasi.input.draft2'
CONTRACT = '1.0.0-rc.1'
MAX_FILE = 256 * 1024
MAX_RESULT = 8192
SECRET = 'SYNTHETIC_UNDISCLOSED_MARKER'
ROOT = Path(__file__).resolve().parents[1]


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode('utf-8')


def built(id='tool-one', name='行处理'):
    return dict(id=id, name=name, description=SECRET, category='文本与编码', aliases=[SECRET],
                sample=SECRET, model=SECRET,
                adapter=dict(type='builtin', tool='lines', action=0, pattern=SECRET))


def http(id='request'):
    return dict(id=id, name='请求工具', description=SECRET, category='网络与接口',
                adapter=dict(type='http', method='POST', url=f'https://service.invalid/{SECRET}',
                             body={'opaque':SECRET}, response_pointer='/'+SECRET))


def sample(tools=None, **fields):
    row = dict(schema_version=1, id='demo-extension', name='示例插件', version='1.0',
               description=SECRET,
               tools=[built(),http()] if tools is None else tools)
    row.update(fields)
    return row


def maximum(count=64):
    return sample([built(f'tool-{i:02d}', '试'*42+f'{i:02d}') for i in range(1,count+1)])


def filled(raw, size=MAX_FILE, fill=b' '):
    assert len(raw) <= size
    count = size-len(raw)
    return raw+(fill*((count+len(fill)-1)//len(fill)))[:count]


def vector(name, source, expected, *, start=None, mime='application/octet-stream'):
    return dict(name=name, raw=source if isinstance(source,bytes) else compact(source), expected=expected,
                start=start, mime=mime)


def fixtures(samples=ROOT/'plugins-examples'):
    for name, kinds in [('local-text',(2,0)),('ollama',(0,2)),('openai-compatible',(0,2))]:
        yield vector('provider-example-'+name,(samples/(name+'.json')).read_bytes(),
                     dict(totalTools=sum(kinds),builtinTools=kinds[0],httpTools=kinds[1]))
    yield vector('mixed-json-mime',sample(),dict(totalTools=2,builtinTools=1,httpTools=1),mime='application/json')
    source=maximum()
    yield vector('64-tools-first',source,dict(totalTools=64,toolStart=1,nextToolStart=9))
    yield vector('64-tools-middle',source,dict(toolStart=57,nextToolStart=None),start=57)
    yield vector('64-tools-final',source,dict(toolStart=64,nextToolStart=None),start=64)
    yield vector('beyond-final-tool',source,'INVALID_TOOL_START',start=65)
    yield vector('zero-page',source,'INVALID_TOOL_START',start=0)
    yield vector('too-many-tools',maximum(65),'INVALID_MANIFEST')
    yield vector('visible-controls-stripped',sample([built(name='Visible\u0000名称')],name='A\tB'),dict(name='AB'))
    yield vector('outside-padding-256k',filled(compact(source),fill=b'\t \n\r'),dict(totalTools=64))
    yield vector('file-limit-plus-one',filled(compact(source),MAX_FILE+1),'INPUT_TOO_LARGE')
    dense=sample([built(f'bounded-{i:02d}') for i in range(1,17)])
    for tool in dense['tools']:
        tool['sample']=('A'+SECRET)*400
    for tool in dense['tools']:
        tool['sample']+='x'*min(16384-len(tool['sample']), MAX_FILE-len(compact(dense)))
    assert len(compact(dense))==MAX_FILE
    yield vector('literal-text-at-256k',dense,dict(totalTools=16))
    heavy=sample([http()]); heavy['tools'][0]['adapter']['body']={f'key-{i:05d}':i for i in range(11000)}
    yield vector('dense-json-body-at-256k',filled(compact(heavy)),'DOCUMENT_TOO_COMPLEX')
    escaping=sample([http()]); escaping['tools'][0]['adapter']['body']={'escaped':SECRET+'\t'*100000}
    yield vector('dense-json-escapes-at-256k',filled(compact(escaping)),'DOCUMENT_TOO_COMPLEX')
    intern=compact(sample()).replace(b'"tools":',b'"tools":'+b' '*250000,1)
    yield vector('internal-padding-at-256k',filled(intern),'DOCUMENT_TOO_COMPLEX')
    structure=sample([http()]); body=structure['tools'][0]['adapter']['body']
    body.update({f'k{i:04d}':i for i in range(3850)})
    body['escaped']='\t'*8192
    body['large']='x'*(MAX_FILE-len(compact(structure))-len(b',"large":""'))
    assert len(compact(structure))==MAX_FILE
    yield vector('combined-heavy-original',structure,'DOCUMENT_TOO_COMPLEX')
    valid_spaces=compact(sample()).replace(b'"tools":',b'"tools":'+b' '*(31*1024),1)
    yield vector('pretty-spacing-within-limit',filled(valid_spaces),dict(totalTools=2))
    url_case=sample([http()]); adapter=url_case['tools'][0]['adapter']
    adapter['url']+='p'*(MAX_FILE-len(compact(url_case)))
    assert len(compact(url_case))==MAX_FILE
    yield vector('long-http-path-at-256k',url_case,'DOCUMENT_TOO_COMPLEX')
    accepted_url=sample([http()]); endpoint=accepted_url['tools'][0]['adapter']
    endpoint['url']+='p'*(2048-len(endpoint['url']))
    yield vector('http-path-limit',accepted_url,dict(httpTools=1))
    endpoint['url']+='p'
    yield vector('http-path-limit-plus-one',accepted_url,'DOCUMENT_TOO_COMPLEX')
    for count,escapes,spaced,expected in [
        (1940,0,False,'DOCUMENT_TOO_COMPLEX'),(950,2000,True,'DOCUMENT_TOO_COMPLEX'),
        (950,0,False,'DOCUMENT_TOO_COMPLEX'),(470,2000,True,'DOCUMENT_TOO_COMPLEX'),
        (237,0,False,dict(httpTools=1)),(238,0,False,'DOCUMENT_TOO_COMPLEX'),
        (0,4000,False,dict(httpTools=1)),(220,2000,True,dict(httpTools=1)),
    ]:
        near=sample([http()]); payload=near['tools'][0]['adapter']['body']
        payload.update({f'k{i:04d}':i for i in range(count)})
        payload['escapes']='\t'*escapes
        padding=25000 if spaced else 0
        filler=MAX_FILE-len(compact(near))-len(b',"filler":""')-padding
        assert filler>0
        payload['filler']='x'*filler
        raw=compact(near)
        if spaced:
            raw=raw.replace(b'"tools":',b'"tools":'+b' '*padding,1)
        assert len(raw)==MAX_FILE
        yield vector(f'{count}-keys-{escapes}-escapes-at-256k',raw,expected)
    yield vector('unknown-schema',sample(schema_version=2),'INVALID_MANIFEST')
    yield vector('studio-manifest-is-distinct',dict(schemaVersion=2,id='com.example.studio'),'INVALID_MANIFEST')
    yield vector('unknown-field-secret',sample(**{SECRET:'omit'}),'INVALID_MANIFEST')
    yield vector('malformed-source-secret',b'{"'+SECRET.encode()+b'":]}','INVALID_JSON')
    valid=compact(sample())
    yield vector('duplicate-top-key',valid.replace(b'"schema_version":1',b'"schema_version":1,"schema_version":1'),'INVALID_JSON')
    yield vector('duplicate-nested-decoded-key',valid.replace(b'"opaque":',br'"op\u0061que":1,"opaque":',1),'INVALID_JSON')
    yield vector('duplicate-tool-id',sample([built(), built()]),'INVALID_MANIFEST')
    builtin=built(); builtin['adapter'].update(tool=SECRET)
    yield vector('private-unknown-builtin',sample([builtin]),'INVALID_MANIFEST')
    builtin=built(); builtin['adapter']['action']=200
    yield vector('action-no-longer-available',sample([builtin]),'INVALID_MANIFEST')
    remote=http(); remote['adapter']['url']='http://service.invalid/'+SECRET
    yield vector('plain-remote-http-rejected',sample([remote]),'INVALID_MANIFEST')
    forbidden=http(); forbidden['adapter']['url']='https://service.invalid/check?token='+SECRET
    yield vector('secret-url-details-rejected',sample([forbidden]),'INVALID_MANIFEST')
    loop=http(); loop['adapter']['url']='http://127.0.0.1:12001/'+SECRET
    yield vector('loopback-declaration-only',sample([loop]),dict(httpTools=1))
    signed=sample(); signed.update(signature=SECRET)
    yield vector('signed-envelope-not-a-manifest',signed,'INVALID_MANIFEST')
    wrong=built(); wrong.update(version='1.0.1',version_history=[dict(version='1.0.1',date='2026-02-30',changes=SECRET)])
    yield vector('invalid-version-history-date',sample([wrong]),'INVALID_MANIFEST')
    history=built(); history.update(version='1.0.1',version_history=[dict(version='1.0.0',date='2026-02-27',changes=SECRET),
                                                                  dict(version='1.0.1',date='2026-02-28',changes=SECRET)])
    yield vector('valid-version-history',sample([history]),dict(totalTools=1))
    yield vector('wrong-mime',sample(),'UNSUPPORTED_MIME',mime='image/png')


def envelope(case, module):
    raw=case['raw']
    material=dict(resourceId='27e08592-5d0d-4f77-a4c0-7de5711e4dce',
                  mimeType=case['mime'],byteLength=len(raw),contentVersion=digest(raw))
    payload={} if case['start'] is None else dict(toolStart=case['start'])
    request=dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId='programming',input=payload)
    local=compact(dict(commandId=None,resourceInputProfile=PROFILE,resources=[dict(index=0,**material)],**request))
    header=compact(dict(profile=PROFILE,request=dict(workerKind='wasi',workerPath='',**request),
                        module=dict(byteLength=len(module),sha256=digest(module)),resources=[material]))
    return local, b'ZIRW\x00\x00\x00\x02'+struct.pack('<I',len(header))+header+module+raw


def invoke(arguments, request):
    result=subprocess.run(arguments,input=request,capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        decoded=json.loads(result.stdout)
    except ValueError:
        decoded=dict(stdout=result.stdout[-180:].decode(errors='replace'), stderr=result.stderr[-350:].decode(errors='replace'))
    return result.returncode, decoded


def check(result, case):
    expected=case['expected']; text=compact(result)
    assert SECRET.encode() not in text and b'service.invalid' not in text and b'127.0.0.1' not in text
    assert len(text)<=MAX_RESULT and result['contractVersion']==CONTRACT
    if isinstance(expected,str):
        assert result['ok'] is False and result['data'] is None and result['error']['code']==expected, result
        return
    assert result['ok'] is True and result['error'] is None
    data=result['data']; view=data['preview']
    assert data['sha256']==digest(case['raw']) and data['byteLength']==len(case['raw'])
    assert all(view[key]==value for key,value in expected.items()), (view,expected)
    assert [row['ordinal'] for row in view['tools']]==list(range(view['toolStart'],min(view['totalTools']+1, view['toolStart']+8)))
    assert all(set(row['adapter'])==({'type','tool','action'} if row['adapter']['type']=='builtin' else {'type','method'}) for row in view['tools'])


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('worker','module','native','evidence','scratch','samples'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--case',action='append',default=[],help='Run only the named probe cases')
    args=parser.parse_args()
    module=args.module.read_bytes()
    args.scratch.mkdir(parents=True,exist_ok=True)
    rows=[]; failures=[]
    with TemporaryDirectory(dir=args.scratch) as tmp:
        path=Path(tmp)/'synthetic-manifest.json'
        for case in fixtures(args.samples):
            if args.case and case['name'] not in args.case:
                continue
            path.write_bytes(case['raw'])
            local, frame=envelope(case,module)
            native_code,native=invoke([args.native,path],local)
            worker_code, worker=invoke([args.worker,'--zicode-plugin-worker'],frame)
            try:
                assert native_code==0 and worker_code==0 and worker['ok'], (native_code,worker_code,worker)
                assert worker['result']==native, (worker['result'],native)
                check(native,case)
                rows.append(dict(case=case['name'],inputBytes=len(case['raw']),inputSha256=digest(case['raw']),
                                 resultBytes=len(compact(native)),result=native))
            except (AssertionError,KeyError,TypeError) as failure:
                failures.append(dict(case=case['name'],inputBytes=len(case['raw']),reason=str(failure)[:650]))
    result=dict(cases=rows,failures=failures,passed=len(rows),moduleBytes=len(module),wasmSha256=digest(module),
                workerSha256=digest(args.worker.read_bytes()),workerProfile=PROFILE)
    args.evidence.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps({'passed':len(rows),'failures':failures,'moduleBytes':len(module)}))
    return int(bool(failures))


if __name__=='__main__':
    raise SystemExit(main())
