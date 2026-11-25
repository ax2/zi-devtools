"""Verify backup memo extraction against independent exact Markdown in the production Windows Worker."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.memo-backup"
CAPABILITY = "devtools.memos.backup.markdown"
PROFILE = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
MIME = "application/octet-stream"
MD = "text/markdown"
CONTRACT = "1.0.0-rc.1"
MAX_FILE = 128 * 1024
MAX_RESULT = 40 * 1024

FIRST = "# 记录\r\n\r\n需要保持精确字节、空格  \n🙂\t\n"
SKIPPED = "FIXTURE_PRIVATE_EXCLUDED"


def record(number, *, title="本机备忘", body=FIRST, **overrides):
    base=dict(id=f"12345678-1234-4234-8234-{number:012d}",revision=1,updated=0,title=title,body=body,
              pinned=False,trash=False,schedule=None)
    base.update(overrides)
    return base


def backup(*records, **overrides):
    value=dict(format="zi-devtools-planner",version=1,created_at=1_700_000_000,records=list(records))
    value.update(overrides)
    return compact(value)


def choose(total, eligible, *, number=None, body=None):
    return dict(totalRecords=total,eligibleMemos=eligible,memoNumber=number,
                byteLength=None if body is None else len(body.encode("utf-8")))


def fixtures():
    documents=backup(
        record(1,title="旅行记录",tags=["hashtag","本机"]),
        record(2,title=SKIPPED,body=SKIPPED,schedule={"start":"2026-11-25T09:00:00","repeat":"Once"},
               calendar_uid="fixture-calendar"),
        record(3,title=SKIPPED,body=SKIPPED,trash=True),
        record(4,title="备用页",body=""),
    )
    yield "list-normal-memos-by-backup-index",documents,{},MIME,(choose(4,2),None)
    yield "first-Markdown-exact",documents,{"memoNumber":1},MIME,(choose(4,2,number=1,body=FIRST),FIRST)
    yield "json-snapshot-mime",documents,{"memoNumber":1},"application/json",(choose(4,2,number=1,body=FIRST),FIRST)
    yield "strictly-ordinary-memo",documents,{"memoNumber":2},MIME,"MEMO_NOT_EXPORTABLE"
    yield "trash-excluded",documents,{"memoNumber":3},MIME,"MEMO_NOT_EXPORTABLE"
    yield "empty-markdown-is-legal",documents,{"memoNumber":4},MIME,(choose(4,2,number=4,body=""),"")
    yield "zero-index-outside",documents,{"memoNumber":0},MIME,"MEMO_NOT_FOUND"
    yield "past-end-index",documents,{"memoNumber":5},MIME,"MEMO_NOT_FOUND"
    yield "incorrect-position-type",documents,{"memoNumber":"1"},MIME,"INVALID_REQUEST"
    yield "no-provider-auto-action-input",documents,{"apply":True},MIME,"INVALID_REQUEST"
    yield "unknown-mime",documents,{},"image/png","UNSUPPORTED_MIME"
    yield "empty-valid-backup",backup(),{},MIME,(choose(0,0),None)
    yield "all-records-are-excluded",backup(record(1,title=SKIPPED,trash=True),record(2,schedule={})),{},MIME,(choose(2,0),None)
    yield "notice-does-not-validate-calendar-rules",backup(record(1,schedule={"unexpected":"field"}),record(2)),{"memoNumber":2},MIME,(choose(2,1,number=2,body=FIRST),FIRST)
    yield "legacy-missing-nullable-schedule",backup({k:v for k,v in record(1).items() if k!="schedule"}),{"memoNumber":1},MIME,(choose(1,1,number=1,body=FIRST),FIRST)
    yield "legacy-missing-tags",backup(record(1,title="普通备忘")),{},MIME,(choose(1,1),None)
    for label,payload in (
        ("duplicate-id",backup(record(1),record(1))),
        ("unsaved-revision-reject",backup(record(1,revision=0))),
        ("negative-revision-reject",backup(record(1,revision=-1))),
        ("misformed-id",backup(record(1,id="abc-???"))),
        ("overlong-title",backup(record(1,title="题"*121))),
        ("multiline-title",backup(record(1,title="one\ntwo"))),
        ("duplicate-tags",backup(record(1,tags=["Tag","tag"]))),
        ("invalid-tag",backup(record(1,tags=["#local"]))),
        ("calendar-uid-requires-schedule",backup(record(1,calendar_uid="unexpected"))),
        ("invalid-created-time",backup(record(1),created_at=253402300800)),
        ("invalid-updated-time",backup(record(1,updated=-62135596801))),
        ("unknown-record-field",backup(record(1,action="no"))),
        ("unknown-root-field",backup(record(1),action="no")),
        ("non-utf8",b"\xff"),
        ("empty-file",b""),
        ("invalid-json",b"{x"),
    ):
        yield label,payload,{},MIME,"INVALID_BACKUP"
    yield "min-max-date",backup(record(1,updated=-62135596800),created_at=253402300799),{},MIME,(choose(1,1),None)
    yield "unsupported-backup-version",backup(record(1),version=2),{},MIME,"UNSUPPORTED_VERSION"
    yield "unsupported-backup-format",backup(record(1),format="other"),{},MIME,"UNSUPPORTED_VERSION"
    maxrows=[record(i+1,title="𠀀"*116+f"-{i:03}",body="") for i in range(64)]
    yield "max-64-titles-fit-without-trimming",backup(*maxrows),{},MIME,(choose(64,64),None)
    yield "last-record-reachable",backup(*maxrows),{"memoNumber":64},MIME,(choose(64,64,number=64,body=""),"")
    yield "over-record-limit",backup(*maxrows,record(65)),{},MIME,"RECORD_LIMIT"
    big="# 多块精确恢复\r\n"+"文"*36000+"\t \r\n"
    at=backup(record(1,body=big)).ljust(MAX_FILE,b" ")
    assert len(at)==MAX_FILE
    yield "exact-128-kib-over-two-host-chunks",at,{"memoNumber":1},MIME,(choose(1,1,number=1,body=big),big)
    yield "one-byte-over",at+b" ",{},MIME,"INPUT_TOO_LARGE"
    escaped='"\n\\\t'*13000
    at=backup(record(1,body=escaped)).ljust(MAX_FILE,b" ")
    yield "dense-json-escapes",at,{"memoNumber":1},MIME,(choose(1,1,number=1,body=escaped),escaped)
    body="raw NUL \0; keep final whitespace  \r\n"
    yield "preserve-native-body-bytes",backup(record(1,body=body)),{"memoNumber":1},MIME,(choose(1,1,number=1,body=body),body)


def verify(worker, native, module, path, output, payload, options, mime, expected):
    resource=dict(resourceId="12345678-1234-4234-8234-987654321010",
                  contentVersion=digest(payload),mimeType=mime,byteLength=len(payload))
    attrs=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input=options,
               resourceOutputProfile=OUTPUT)
    stdin=compact(dict(**attrs,commandId=None,resourceInputProfile=PROFILE,resources=[dict(index=0,**resource)]))
    direct=invoke([native,path,output],stdin)
    header=compact(dict(profile=PROFILE,request=dict(workerPath="",workerKind="wasi",**attrs),
                       resources=[resource],module=dict(byteLength=len(module),sha256=digest(module))))
    result=subprocess.run([str(worker),"--zicode-plugin-worker"],input=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+payload,
                          capture_output=True,timeout=12,creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode != 0 or not result.stdout.startswith(b"ZIWO\0\0\0\x01"):
        raise RuntimeError(result.stdout[:256].decode(errors="replace"))
    end=12+struct.unpack_from("<I",result.stdout,8)[0]
    wire=json.loads(result.stdout[12:end])
    report=wire["result"]
    assert report==direct,(report,direct)
    assert wire["profile"]==OUTPUT and wire["schemaVersion"]==1 and report["contractVersion"]==CONTRACT
    actual=result.stdout[end:]
    wanted=None
    if isinstance(expected,str):
        assert not report["ok"] and report["data"] is None and report["error"]["code"]==expected,report
    else:
        fields,wanted=expected
        assert report["ok"] and report["error"] is None,report
        data=report["data"]
        assert {key:value for key,value in data.items() if key!="text"}==fields,(data,fields)
        if fields["totalRecords"]==4 and fields["memoNumber"] is None:
            assert data["text"].endswith("1 · 旅行记录\n4 · 备用页"),data
        if fields["totalRecords"]==64 and fields["memoNumber"] is None:
            assert data["text"].split("\n")[-1].endswith("-063")
        assert SKIPPED not in data["text"]
    if wanted is None:
        assert not wire["resources"] and not actual and not output.exists()
    else:
        exact=wanted.encode("utf-8")
        assert actual==exact==output.read_bytes(),(len(actual),len(exact))
        assert wire["resources"]==[dict(index=0,contentVersion=digest(exact),mimeType=MD,byteLength=len(exact))]
    assert len(compact(report))<=MAX_RESULT
    return dict(ok=report["ok"],resultSha256=digest(compact(report)),
                outputSha256=digest(actual) if wanted is not None else None,
                outputBytes=len(actual) if wanted is not None else None)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for key in ("worker","native","module","evidence"):
        parser.add_argument("--"+key,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    rows,failures=[],[]
    with TemporaryDirectory() as work:
        root=Path(work)
        src=root/"memo-backup.json"
        for index,(name,payload,options,mime,expected) in enumerate(fixtures()):
            src.write_bytes(payload)
            try:
                result=verify(args.worker,args.native,module,src,root/f"{index}.md",payload,options,mime,expected)
            except Exception as err:
                failures.append(dict(case=name,error=repr(err)[:500]))
                print("FAIL",name,repr(err)[:140],flush=True)
            else:
                rows.append(dict(case=name,inputSha256=digest(payload),inputBytes=len(payload),mime=mime,**result))
                print("PASS",name,flush=True)
    receipt=dict(passed=len(rows),moduleSha256=digest(module),wasmSha256=digest(module),
                 workerSha256=digest(args.worker.read_bytes()),workerProfile=PROFILE,outputProfile=OUTPUT,cases=rows,failures=failures)
    args.evidence.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
