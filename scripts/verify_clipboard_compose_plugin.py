"""Verify clipboard result files through real Windows Worker and the native adapter."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN="com.zicode.devtools.clipboard-compose"
CAPABILITY="devtools.clipboard.text.compose"
TITLE="剪贴片段组合"
INPUT="resource.wasi.input.draft2"
OUTPUT="resource.wasi.output.draft1"
JSON="application/json"
TEXT="text/plain"
OCTET="application/octet-stream"


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def fixtures():
    sample=['第一行\r\n续行\n','含,"引号"',"  =1+2",'\t前缀',"  @示例", "同项", "同项"]
    document=compact(sample)
    records=compact([dict(order=i+1,text=text,source='PRIVATE_SOURCE.exe',time='PRIVATE_TIME',labels=['PRIVATE_LABEL'])
                     for i,text in enumerate(sample)])
    output={
        'paragraphs':('text/plain',"第一行\r\n续行\n\n\n含,\"引号\"\n\n  =1+2\n\n\t前缀\n\n  @示例\n\n同项\n\n同项".encode()),
        'numbered':('text/plain',"1. 第一行\r\n续行\n\n2. 含,\"引号\"\n3.   =1+2\n4. \t前缀\n5.   @示例\n6. 同项\n7. 同项".encode()),
        'csv':('text/csv',"text\n\"第一行\r\n续行\n\"\n\"含,\"\"引号\"\"\"\n'  =1+2\n'\t前缀\n'  @示例\n同项\n同项\n".encode()),
        'markdown':('text/markdown',"- 第一行\n  续行\n  \n- 含,\"引号\"\n-   =1+2\n- \t前缀\n-   @示例\n- 同项\n- 同项".encode()),
    }
    for format,(mime,value) in output.items():
        yield f'{format}-array',document,JSON,dict(format=format),(len(sample),mime,value)
        yield f'{format}-native-records',records,JSON,dict(format=format),(len(sample),mime,value)
    source=compact(['one\rtwo\rthree\r'])
    yield 'markdown-lone-cr',source,TEXT,dict(format='markdown'),(1,'text/markdown',b'- one\n  two\n  three\n  ')
    yield 'utf8-bom',b'\xef\xbb\xbf'+document,OCTET,dict(format='paragraphs'),(len(sample),*output['paragraphs'])
    formula=['+1',' -2','\u202f@示例','\rtext',' ordinary',' ="X,Y"']
    expected="text\n'+1\n' -2\n'\u202f@示例\n\"'\rtext\"\n ordinary\n\"' =\"\"X,Y\"\"\"\n".encode()
    yield 'csv-native-formula-and-whitespace',compact(formula),JSON,dict(format='csv'),(len(formula),'text/csv',expected)
    names=[f'段{i}' for i in range(64)]
    markdown='\n'.join(f'- {name}' for name in names).encode()
    yield 'exact-64-texts',compact(names),JSON,dict(format='markdown'),(64,'text/markdown',markdown)
    maximum=compact(['尾'+('x'*(32*1024-7))])
    assert len(maximum)==32*1024
    yield 'exact-32-kib-source',maximum,JSON,dict(format='paragraphs'),(1,TEXT,('尾'+('x'*(32*1024-7))).encode())
    dense=compact(['\n'*(16*1024-2)])
    assert len(dense)==32*1024
    yield 'dense-max-lines',dense,JSON,dict(format='markdown'),(1,'text/markdown',b'- '+b'\n  '*(16*1024-2))
    left=compact([dict(order=2,text='x',source='fixture',time='fixture',labels=[])])
    escape=b'[{"order":1,"order":1,"text":"x","source":"x","time":"x","labels":[]}]'
    for name,raw,mime,option,error in [
        ('too-many-entries',compact(names+['another']),JSON,dict(format='csv'),'ITEM_LIMIT'),
        ('source-one-byte-over',maximum+b' ',JSON,dict(format='csv'),'INPUT_LIMIT'),
        ('unordered-record',left,JSON,dict(format='csv'),'RECORD_ORDER'),
        ('duplicate-order-field',escape,JSON,dict(format='csv'),'INVALID_JSON'),
        ('invalid-record-shape',b'[{"order":1,"text":"one","metadata":{}}]',JSON,dict(format='markdown'),'INVALID_JSON'),
        ('mixed-records-and-plain',b'["plain",{"order":2,"text":"record","source":"s","time":"t","labels":[]}]',JSON,dict(format='markdown'),'INVALID_JSON'),
        ('invalid-utf8',b'["\xff"]',JSON,dict(format='paragraphs'),'INVALID_JSON'),
        ('empty-source',b'[]',JSON,dict(format='csv'),'ITEM_LIMIT'),
        ('empty-text',b'[""]',JSON,dict(format='csv'),'ITEM_LIMIT'),
        ('unknown-format',document,JSON,dict(format='html'),'INVALID_REQUEST'),
        ('unknown-mime',document,'image/png',dict(format='markdown'),'INVALID_RESOURCE'),
    ]:
        yield name,raw,mime,option,error


def verify(case,worker,native,module,scratch):
    name,source,mime,options,expected=case
    report,blob=run_worker(worker,module,case)
    native_report,native_blob=run_native(native,case,scratch)
    assert report==native_report and blob==native_blob,report
    if isinstance(expected,str):
        assert not report['ok'] and report['error']['code']==expected and blob is None,report
        failure=expected
    else:
        count,media,exact=expected
        assert report['ok'] and report['error'] is None,report
        assert report['data']['count']==count and report['data']['mimeType']==media,report
        assert report['data']['byteLength']==len(blob) and blob==exact,(blob[:250],exact[:250])
        assert all(token not in blob for token in (b'PRIVATE_SOURCE',b'PRIVATE_TIME',b'PRIVATE_LABEL'))
        failure=None
    return dict(case=name,mime=mime,inputBytes=len(source),sourceSha256=digest(source),options=options,code=failure,
                mimeOutput=report['data']['mimeType'] if blob else None,
                outputBytes=len(blob) if blob else 0,outputSha256=digest(blob) if blob else None)


def adapter_header(case):
    _,source,mime,opts,_=case
    resource=dict(resourceId="4fad13fe-807b-431b-8bbd-f032b26ee77e",mimeType=mime,byteLength=len(source),contentVersion=digest(source))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input=opts,resourceOutputProfile=OUTPUT)
    return request,resource


def run_native(native,case,scratch):
    request,resource=adapter_header(case)
    source,target=scratch/"source.json",scratch/"target.txt"
    source.write_bytes(case[1])
    envelope=dict(**request,commandId=None,resourceInputProfile=INPUT,resources=[dict(index=0,**resource)])
    process=subprocess.run([native,source,target],input=compact(envelope),capture_output=True,check=True,
                           timeout=12,creationflags=subprocess.CREATE_NO_WINDOW)
    assert source.read_bytes()==case[1]
    return json.loads(process.stdout),target.read_bytes() if target.exists() else None


def run_worker(worker,module,case):
    request,resource=adapter_header(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),resources=[resource],
                        module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case[1]
    process=subprocess.run([worker,"--zicode-plugin-worker"],input=frame,capture_output=True,
                           timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert process.returncode==0 and process.stdout.startswith(b"ZIWO\0\0\0\x01"),process.stdout[:350]
    end=12+struct.unpack_from("<I",process.stdout,8)[0]
    wire=json.loads(process.stdout[12:end])
    blob=process.stdout[end:]
    if wire["resources"]:
        resource,=wire["resources"]
        assert resource["mimeType"] in (TEXT,"text/csv","text/markdown") and resource["mimeType"]==wire["result"]["data"]["mimeType"] and resource["byteLength"]==len(blob) and resource["contentVersion"]==digest(blob)
    else:
        assert not blob
    return wire["result"],blob if blob else None


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            scratch=Path(temporary)/case[0]
            scratch.mkdir()
            try:
                cases.append(verify(case,args.worker,args.native,module,scratch))
            except (AssertionError,subprocess.SubprocessError,ValueError,TypeError) as error:
                failures.append(dict(case=case[0],message=str(error)[:500]))
    proof=dict(wasmSha256=digest(module),wasmBytes=len(module),workerSha256=digest(args.worker.read_bytes()),
               inputProfile=INPUT,outputProfile=OUTPUT,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(cases),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
