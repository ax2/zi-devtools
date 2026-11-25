"""Check the shared native query and actual Windows Worker against synthetic ZiDevTools indexes."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from hashlib import sha256
import json
from pathlib import Path
import re
import sqlite3
import struct
import subprocess
from tempfile import TemporaryDirectory

ROOT=Path(__file__).resolve().parents[1]
PLUGIN="com.zicode.devtools.keyword-index"
CAPABILITY="devtools.knowledge.keyword.snapshot.search"
TITLE="本机知识索引关键词检索"
INPUT="resource.wasi.input.draft2"
OUTPUT="resource.wasi.output.draft1"
SQLITE="application/x-zicode-sqlite"
OCTET="application/octet-stream"
TEXT="text/plain"
DDL=re.search(r'pub const INDEX_DDL: &str = r#"(.*?)"#;',
              (ROOT/"crates/zi-knowledge-keyword-core/src/lib.rs").read_text(encoding="utf-8"),re.S).group(1)
SMALL=[
    ("alpha-src","a-long.txt",0,"alpha "+("other unrelated words "*22)+" beta"),
    ("alpha-src","z-short.txt",0,"alpha beta alpha beta"),
    ("han-src","guide/中文.txt",0,"样本中的桌面检索测试文本"),
    ("han-src","quick.txt",0,"离线桌面 工具"),
    ("other","word.txt",0,"gamma beta"),
]


def sha(data):
    return sha256(data).hexdigest()


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def populate(db, rows):
    db.executescript(DDL)
    db.execute("PRAGMA foreign_keys=ON")
    for index,(source,path,ordinal,text) in enumerate(rows,1):
        raw=text.encode()
        checksum=sha(raw)
        db.execute("INSERT OR IGNORE INTO indexed_files VALUES(?,?,?,?,?,?,?)",
                   (source,path,checksum,len(raw),len(raw),1,"2001-01-01T00:00:00Z"))
        db.execute("INSERT INTO chunks VALUES(?,?,?,?,?,?,?)",
                   (index,source,path,ordinal,"line:1",checksum,text))
    db.commit()


def make_fixture(scratch, name, rows=SMALL, change=None):
    path=scratch/(name+".sqlite")
    with closing(sqlite3.connect(path)) as db:
        populate(db,rows)
        if change:
            db.executescript(change)
        db.commit()
    return path.read_bytes()


def dense_rows(count):
    return [("dense","dense.txt",index,"alpha beta gamma delta epsilon 中文 " + "extra terms "*2)
            for index in range(count)]


def limit_fixture(scratch):
    rows=dense_rows(32)
    for padding in range(0,128*1024,4096):
        file=scratch/f"padding-{padding}.sqlite"
        with closing(sqlite3.connect(file)) as db:
            populate(db,rows)
            if padding:
                db.execute("CREATE TABLE temporary_padding (content)")
                db.execute("INSERT INTO temporary_padding VALUES(zeroblob(?))",(padding,))
                db.execute("DROP TABLE temporary_padding")
                db.commit()
        data=file.read_bytes()
        if len(data)==128*1024:
            return data
        if len(data)>128*1024:
            break
    raise AssertionError("fixture could not hit exact size")


def fixtures(scratch):
    small=make_fixture(scratch,"simple")
    exact=limit_fixture(scratch)
    same_data=len(SMALL)
    assert same_data==5
    cases=[
        ("bm25-not-alphabetical",small,SQLITE,dict(query="alpha beta"),["z-short.txt","a-long.txt"]),
        ("all-terms-no-hit",small,SQLITE,dict(query="alpha gamma",mode="all"),[]),
        ("any-terms",small,SQLITE,dict(query="alpha gamma",mode="any"),["word.txt","z-short.txt","a-long.txt"]),
        ("source-bound-selection",small,OCTET,dict(query="beta",sourceId="other"),["word.txt"]),
        ("cjk-auto-exact",small,SQLITE,dict(query="桌面检索",mode="auto"),["guide/中文.txt"]),
        ("literal-phrase",small,SQLITE,dict(query="桌面 工具",mode="phrase"),["quick.txt"]),
        ("explicit-ascii-phrase",small,SQLITE,dict(query="alpha beta",mode="phrase"),["z-short.txt"]),
        ("escaped-quote-token",small,SQLITE,dict(query='alpha "beta"',mode="all"),["z-short.txt","a-long.txt"]),
        ("one-result-cap",small,SQLITE,dict(query="alpha",limit=1),["z-short.txt"]),
        ("unknown-source",small,SQLITE,dict(query="alpha",sourceId="missing"),[]),
        ("empty-index",make_fixture(scratch,"empty",[]),SQLITE,dict(query="alpha"),[]),
        ("dense-and-exact-128-kib",exact,SQLITE,dict(query="alpha beta",limit=10),["dense.txt"]*10),
        ("dense-cjk-and-exact-128-kib",exact,OCTET,dict(query="中文",limit=10),["dense.txt"]*10),
        ("dense-4-terms",exact,SQLITE,dict(query="alpha beta gamma one",mode="any",limit=10),["dense.txt"]*10),
        ("chunk-length-limit",make_fixture(scratch,"chunk-long",[("a","long.txt",0,"alpha "+"x"*4091)]),SQLITE,dict(query="alpha"),"INDEX_ROW_LIMIT"),
        ("too-many-chunks",make_fixture(scratch,"row-limit",dense_rows(33)),SQLITE,dict(query="alpha"),"INDEX_ROW_LIMIT"),
        ("empty-source-filter",small,SQLITE,dict(query="alpha beta",sourceId=""),["z-short.txt","a-long.txt"]),
        ("4-kib-all-one-token",make_fixture(scratch,"one-token",
            [("repeat","repeated.txt",i,"a "*64) for i in range(32)]),SQLITE,
            dict(query="a a a",limit=10),["repeated.txt"]*10),
        ("3-kib-four-terms",make_fixture(scratch,"three-kib",
            [("repeat","repeated.txt",i,"a "*48) for i in range(32)]),SQLITE,
            dict(query="a a a a",mode="any",limit=10),["repeated.txt"]*10),
        ("12-fts-terms-rejected",small,SQLITE,
            dict(query="a a a a a a a a a a a a",mode="all",limit=10),"INVALID_QUERY"),
        ("punctuation-does-not-bypass-term-limit",small,SQLITE,
            dict(query="alpha-beta-gamma-delta-epsilon",mode="all"),"INVALID_QUERY"),
        ("12-literal-words",make_fixture(scratch,"literal-twelve",
            [("repeat","twelve.txt",0,"a b c d e f g h i j k l")]),SQLITE,
            dict(query="a b c d e f g h i j k l",mode="phrase"),["twelve.txt"]),
        ("dense-term-combination-rejected",make_fixture(scratch,"four-max-corpus",
            [("repeat","repeated.txt",i,"a "*64) for i in range(32)]),SQLITE,
            dict(query="a a a a",mode="any"),"SEARCH_LIMIT"),
        ("4-kib-cjk",make_fixture(scratch,"cjk-max",
            [("repeat","中文.txt",i,"桌"*42+"aa") for i in range(32)]),SQLITE,
            dict(query="桌桌",limit=10),["中文.txt"]*10),
        ("4-kib-plus-one",make_fixture(scratch,"corpus-budget",
            [("a","max.txt",0,"x"*2048),("a","max.txt",1,"z"*2048),("a","max.txt",2,"a")]),SQLITE,
            dict(query="a",limit=10),"INDEX_ROW_LIMIT"),
        ("missing-docsize",make_fixture(scratch,"docsize",change="DELETE FROM chunks_fts_docsize WHERE id=1"),SQLITE,dict(query="alpha"),"INVALID_DATABASE"),
        ("mismatched-docsize",make_fixture(scratch,"docsize-bad",change="UPDATE chunks_fts_docsize SET sz=X'00' WHERE id=2"),SQLITE,dict(query="alpha"),"INVALID_DATABASE"),
        ("mismatched-postings",make_fixture(scratch,"outdated-fts",change=
            "DROP TRIGGER chunks_au; UPDATE chunks SET text='outdated' WHERE id=1; CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN "
            "INSERT INTO chunks_fts(chunks_fts, rowid, text) VALUES('delete', old.id, old.text); "
            "INSERT INTO chunks_fts(rowid, text) VALUES(new.id, new.text); END;"),
            SQLITE,dict(query="alpha"),"INVALID_DATABASE"),
        ("wrong-fts-format",make_fixture(scratch,"fts-version",change="INSERT INTO chunks_fts_config VALUES('pgsz',128)"),SQLITE,dict(query="alpha"),"INVALID_INDEX_SCHEMA"),
        ("extra-trigger",make_fixture(scratch,"trigger",change="CREATE TRIGGER extra AFTER INSERT ON indexed_files BEGIN SELECT 1; END"),SQLITE,dict(query="alpha"),"INVALID_INDEX_SCHEMA"),
        ("extra-view",make_fixture(scratch,"view",change="CREATE VIEW extra AS SELECT relative FROM indexed_files"),SQLITE,dict(query="alpha"),"INVALID_INDEX_SCHEMA"),
        ("wrong-content-table",make_fixture(scratch,"external",change="DROP TABLE chunks_fts; CREATE VIRTUAL TABLE chunks_fts USING fts5(text,content='indexed_files');"),SQLITE,dict(query="alpha"),"INVALID_INDEX_SCHEMA"),
        ("wrong-schema-version",make_fixture(scratch,"index-version",change="PRAGMA user_version=2"),SQLITE,dict(query="alpha"),"INVALID_INDEX_SCHEMA"),
        ("wal-needs-companion",make_fixture(scratch,"wal",change="PRAGMA journal_mode=WAL"),SQLITE,dict(query="alpha"),"WAL_SNAPSHOT"),
        ("relative-ref-only",make_fixture(scratch,"absolute",[("a","X:/fixture-only/item",0,"alpha")]),SQLITE,dict(query="alpha"),"INVALID_INDEX_RECORD"),
        ("no-traversal",make_fixture(scratch,"traversal",[("a","a/../b.txt",0,"alpha")]),SQLITE,dict(query="alpha"),"INVALID_INDEX_RECORD"),
        ("request-bad-source",small,SQLITE,dict(query="alpha",sourceId="../other"),"INVALID_QUERY"),
        ("request-too-many-terms",small,SQLITE,dict(query="alpha "*13),"INVALID_QUERY"),
        ("request-limit-over",small,SQLITE,dict(query="alpha",limit=11),"INVALID_QUERY"),
        ("incorrect-mime",small,"text/plain",dict(query="alpha"),"INVALID_RESOURCE"),
        ("larger-file",exact+b"\0",SQLITE,dict(query="alpha"),"INPUT_LIMIT"),
        ("partial-file",small[:-4096],SQLITE,dict(query="alpha"),"INVALID_DATABASE"),
        ("non-sqlite",b"plain fixture",SQLITE,dict(query="alpha"),"INVALID_DATABASE"),
    ]
    return cases


def wire(case):
    _,source,mime,inputs,_=case
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",commandId=None,
                 resourceOutputProfile=OUTPUT,input=inputs)
    resource=dict(resourceId="8bb27a8a-139b-42a0-812b-383123234556",mimeType=mime,
                  byteLength=len(source),contentVersion=sha(source))
    return request,resource


def run_native(executable,case,scratch):
    request,resource=wire(case)
    path=scratch/"selected.sqlite"
    target=scratch/"report.txt"
    path.write_bytes(case[1])
    envelope=dict(**request,resourceInputProfile=INPUT,resources=[dict(index=0,**resource)])
    process=subprocess.run([executable,path,target],input=compact(envelope),capture_output=True,check=True,
                           timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert path.read_bytes()==case[1]
    return json.loads(process.stdout),target.read_bytes() if target.exists() else None


def run_worker(executable,module,case):
    request,resource=wire(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),
                        resources=[resource],module=dict(byteLength=len(module),sha256=sha(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case[1]
    process=subprocess.run([executable,"--zicode-plugin-worker"],input=frame,capture_output=True,
                           timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    if process.returncode!=0 or not process.stdout.startswith(b"ZIWO\0\0\0\x01"):
        raise AssertionError((process.returncode,process.stdout[:400],process.stderr[:200]))
    end=12+struct.unpack_from("<I",process.stdout,8)[0]
    result=json.loads(process.stdout[12:end])
    text=process.stdout[end:]
    if result["resources"]:
        meta,=result["resources"]
        assert meta["mimeType"]==TEXT and meta["byteLength"]==len(text) and meta["contentVersion"]==sha(text)
    else:
        assert not text
    return result["result"],text or None


def validate(report,blob,wanted):
    if isinstance(wanted,str):
        assert not report["ok"] and report["error"]["code"]==wanted and not blob,report
        return
    assert report["ok"] and report["error"] is None and blob,report
    assert report["data"]["text"].encode("utf-8")==blob
    assert [hit["relative"] for hit in report["data"]["hits"]]==wanted,report
    assert all(len(hit["excerpt"])<=262 for hit in report["data"]["hits"])
    assert len(blob)<=24*1024 and len(compact(report))<=48*1024


def main():
    parser=argparse.ArgumentParser()
    for flag in ("worker","native","module","evidence","scratch"):
        parser.add_argument("--"+flag,type=Path,required=True)
    parser.add_argument("--only",nargs="+",help="Only run named cases during diagnosis")
    args=parser.parse_args()
    module=args.module.read_bytes()
    checks,failures=[],[]
    with TemporaryDirectory(dir=args.scratch) as temporary:
        root=Path(temporary)
        vectors=fixtures(root)
        for case in vectors:
            name,source,mime,inputs,wanted=case
            if args.only and name not in args.only:
                continue
            directory=root/name
            directory.mkdir()
            try:
                result,blob=run_worker(args.worker,module,case)
                validate(result,blob,wanted)
                assert (result,blob)==run_native(args.native,case,directory),"Native and Worker mismatch"
                checks.append(dict(case=name,inputBytes=len(source),mime=mime,inputSha256=sha(source),
                                   options=inputs,code=wanted if isinstance(wanted,str) else None,
                                   outputBytes=len(blob) if blob else 0, outputSha256=sha(blob) if blob else None))
            except (AssertionError,ValueError,subprocess.SubprocessError) as error:
                failures.append(dict(case=name,error=repr(error)[:900]))
        overlap=[] if args.only else [vectors[0],vectors[4],vectors[5]]
        try:
            with ThreadPoolExecutor(max_workers=3) as pool:
                for case,(report,blob) in zip(overlap,pool.map(lambda x:run_worker(args.worker,module,x),overlap)):
                    validate(report,blob,case[4])
            concurrent=bool(overlap)
        except (AssertionError,ValueError,subprocess.SubprocessError) as error:
            concurrent=False
            failures.append(dict(case="separate-concurrent-worker",error=repr(error)[:900]))
    proof=dict(inputProfile=INPUT,outputProfile=OUTPUT,wasmSha256=sha(module),wasmBytes=len(module),
               workerSha256=sha(args.worker.read_bytes()),cases=checks,failures=failures,
               threeConcurrentIndependentWorkers=concurrent)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(checks),failures=failures,concurrent=concurrent),ensure_ascii=False))
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
