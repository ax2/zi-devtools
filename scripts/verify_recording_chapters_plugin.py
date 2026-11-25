"""Compare Windows native/Worker chapter files and independently parse their logical lines."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN="com.zicode.devtools.recording-chapters"
CAPABILITY="devtools.recording.chapters.export"
TITLE="录屏章节元数据"
INPUT="resource.wasi.input.draft2"
OUTPUT="resource.wasi.output.draft1"
JSON="application/json"
TEXT="text/plain"
OCTET="application/octet-stream"


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def chapters(duration, markers):
    return compact(dict(durationMs=duration,chapters=[dict(startMs=position,title=title) for position,title in markers]))


def parse_metadata(raw):
    # Parsing escaped logical lines independently also prevents injected tags
    # or [CHAPTER] text inside a multiline title from going unnoticed.
    lines=[]
    offset=[]
    escaped=False
    for value in raw.decode("utf-8"):
        if escaped:
            offset.append(value)
            escaped=False
        elif value=="\\":
            escaped=True
        elif value=="\n":
            lines.append("".join(offset))
            offset=[]
        else:
            offset.append(value)
    assert not escaped and not offset
    assert lines[0]==";FFMETADATA1" and (len(lines)-1)%5==0
    result=[]
    for index in range(1,len(lines),5):
        section,base,begin,end,title=lines[index:index+5]
        assert section=="[CHAPTER]" and base=="TIMEBASE=1/1000"
        assert begin.startswith("START=") and end.startswith("END=") and title.startswith("title=")
        result.append((int(begin[6:]),int(end[4:]),title[6:]))
    return result


def fixtures():
    document=chapters(90500,[(0,"开场 / Intro"),(1200,"标题=1; #片段\\\r\n[CHAPTER]\nEND=999"),(90499," ")])
    expected=[(0,1200,"开场 / Intro"),(1200,90499,"标题=1; #片段\\\n[CHAPTER]\nEND=999"),(90499,90500,"第3章")]
    yield "escaped-chapter-titles",document,JSON,expected,0
    yield "utf8-bom",b"\xef\xbb\xbf"+document,OCTET,expected,0
    yield "optional-title",b'{"durationMs":5,"chapters":[{"startMs":0}]}',TEXT,[(0,5,"第1章")],0
    yield "same-native-skipped-order",chapters(10,[(10,"end"),(0,"A"),(600,"beyond"),(3,"B"),(10,"same end")]),JSON,[(0,3,"A"),(3,10,"B")],3
    yield "long-unicode-title-exact",chapters(100,[(0,"测"*170+"AB")]),JSON,[(0,100,"测"*170+"AB")],0
    yield "maximum-duration",chapters(2678400000,[(2678399999,"末章")]),JSON,[(2678399999,2678400000,"末章")],0
    yield "full-256-chapters",chapters(256,[(i,f"片段{i}") for i in range(256)]),JSON,[(i,i+1,f"片段{i}") for i in range(256)],0
    exact=document+b" "*(32*1024-len(document))
    yield "exact-32-kib",exact,JSON,expected,0
    intense="\\#;="*128
    escaped=chapters(33,[(i,intense) for i in range(31)])
    yield "dense-escaping",escaped+b" "*(32*1024-len(escaped)),JSON,[(i,i+1 if i<30 else 33,intense) for i in range(31)],0
    rows=48
    overhead=len(";FFMETADATA1\n"+"".join(f"[CHAPTER]\nTIMEBASE=1/1000\nSTART={i}\nEND={i+1}\ntitle=\n" for i in range(rows)))
    remaining=48*1024-overhead
    exact_titles=[]
    for index in range(rows):
        chars=min(1024,remaining-(rows-index-1))
        exact_titles.append("#"*(chars//2)+"a"*(chars%2))
        remaining-=chars
    assert remaining==0 and all(0<len(title)<=512 for title in exact_titles)
    yield "exact-48-kib-output",chapters(rows,list(enumerate(exact_titles))),JSON,[(i,i+1,title) for i,title in enumerate(exact_titles)],0
    for name,raw,mime,code in [
        ("input-limit-plus-one",exact+b" ",JSON,"INPUT_LIMIT"),
        ("resource-type",document,"image/png","INVALID_RESOURCE"),
        ("empty-file",b"",JSON,"INVALID_PLAN"),
        ("invalid-utf8",b'{"durationMs":1,"chapters":[{"startMs":0,"title":"\xff"}]}',JSON,"INVALID_PLAN"),
        ("wrong-schema",b'{"durationMs":100,"chapters":[],"options":true}',JSON,"INVALID_PLAN"),
        ("duplicate-field",b'{"durationMs":1,"durationMs":2,"chapters":[]}',JSON,"INVALID_PLAN"),
        ("negative-start",chapters(2,[(-1,"x")]),JSON,"INVALID_PLAN"),
        ("nonintegral-start",chapters(2,[(0.5,"x")]),JSON,"INVALID_PLAN"),
        ("duration-zero",chapters(0,[(0,"x")]),JSON,"DURATION_LIMIT"),
        ("duration-plus-one",chapters(2678400001,[(0,"x")]),JSON,"DURATION_LIMIT"),
        ("empty-chapters",chapters(50,[]),JSON,"NO_CHAPTERS"),
        ("all-outside-video",chapters(3,[(3,"x"),(3000,"y")]),JSON,"NO_CHAPTERS"),
        ("too-many-chapters",chapters(300,[(i,"x") for i in range(257)]),JSON,"CHAPTER_LIMIT"),
        ("same-millisecond",chapters(3,[(1,"x"),(1,"x")]),JSON,"CHAPTER_ORDER"),
        ("backwards-in-duration",chapters(3,[(2,"x"),(1,"y")]),JSON,"CHAPTER_ORDER"),
        ("utf8-title-limit-plus-one",chapters(2,[(1,"测"*171)]),JSON,"INVALID_TITLE"),
        ("title-controls",chapters(2,[(1,"st\0op")]),JSON,"INVALID_TITLE"),
        ("escaped-output-limit",chapters(60,[(i,"#;="*170) for i in range(53)]),JSON,"OUTPUT_LIMIT"),
    ]:
        yield name,raw,mime,code,0


def adapter_header(case):
    _,source,mime,_,_=case
    resource=dict(resourceId="4fad13fe-807b-431b-8bbd-f032b26ee77e",mimeType=mime,byteLength=len(source),contentVersion=digest(source))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input={},resourceOutputProfile=OUTPUT)
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
        assert resource["mimeType"]==TEXT and resource["byteLength"]==len(blob) and resource["contentVersion"]==digest(blob)
    else:
        assert not blob
    return wire["result"],blob if blob else None


def verify(case,worker,native,module,scratch):
    name,source,mime,expected,skipped=case
    report,blob=run_worker(worker,module,case)
    assert (report,blob)==run_native(native,case,scratch),report
    if isinstance(expected,str):
        assert not report["ok"] and report["error"]["code"]==expected and blob is None,report
        error=expected
    else:
        assert report["ok"] and parse_metadata(blob)==expected,report
        data=report["data"]
        assert data["skippedCount"]==skipped and data["chapterCount"]==len(expected)
        assert data["firstStartMs"]==expected[0][0] and data["durationMs"]==expected[-1][1]
        assert data["byteLength"]==len(blob)
        if name=="exact-48-kib-output":
            assert len(blob)==48*1024
        error=None
    return dict(case=name,mime=mime,inputBytes=len(source),sourceSha256=digest(source),code=error,
                outputBytes=len(blob) if blob else 0,outputSha256=digest(blob) if blob else None)


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
