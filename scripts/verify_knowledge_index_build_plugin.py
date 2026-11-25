"""Verify new v1 indices through the Windows Worker and frozen keyword-search module."""

import argparse
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import struct
import subprocess
from tempfile import TemporaryDirectory

from verify_keyword_index_plugin import (
    CAPABILITY as SEARCH_CAPABILITY,
    INPUT, OUTPUT, SQLITE, compact, run_worker as search_worker, sha,
)

PLUGIN="com.zicode.devtools.knowledge-index-build"
CAPABILITY="devtools.knowledge.index.snapshot.create"
TITLE="从文本创建静态知识索引"
SOURCE_ID="studio-snapshot"
LABEL="演示笔记.txt"
TEXT="text/plain"
OCTET="application/octet-stream"
PATH="author-seed.txt"
STANDARD=("alpha "+("unrelated other words "*20)+" beta"
          + "\n\nalpha beta alpha beta"
          + "\n\n样本中的桌面检索资料").encode()


def full_searchable_text():
    prefix=("alpha beta "+("unrelated other words "*80)).encode()
    large=prefix+b"x"*(2201-len(prefix))
    short=b"alpha beta alpha beta "
    chinese="样本中的桌面检索资料".encode()
    fixed=[large,short+b"x"*(57-len(short)),chinese+b"z"*(57-len(chinese))]
    body=b"\n\n".join(fixed+[b"z"*57]*25+[b"z"*56])
    full=body+b" "*(4096-len(body))
    assert len(full)==4096
    return full


def fixtures():
    upper_file=b"\n"*4090+b"alpha "
    assert len(upper_file)==4096
    upper_body=("x"*2201+"\n\n"+"y"*797+"\n\n"+"z"*798).encode()
    corpus=("a "*63+"a")+"\n\n"
    dense=(corpus*31+"a "*48+"a").encode()
    assert len(dense)<=4096
    for name, raw, mime, label in [
        ("english-chinese-ranking",STANDARD,TEXT,LABEL),
        ("full-size-32-chunk-search-index",full_searchable_text(),TEXT,LABEL),
        ("bom-and-octet",b"\xef\xbb\xbf"+"中文资料\n\nalpha".encode(),OCTET,"资料 V2.TXT"),
        ("crlf-native-boundaries","first\r\n\r\n第二行".encode(),TEXT,"windows.txt"),
        ("label-unicode-spaced-apostrophe",b"alpha",TEXT,"引用 title's copy.txt"),
        ("maximum-input-4096",upper_file,TEXT,"limit.txt"),
        ("exact-indexed-4096-with-overlap",upper_body,TEXT,"body.txt"),
        ("all-32-chunks",b"\n\n".join(b"part" for _ in range(32)),TEXT,"count.txt"),
        ("dense-32-chunks",dense,TEXT,"dense.txt"),
        ("maximum-label-128-bytes",b"alpha",TEXT,"x"*124+".txt"),
        ("ordinary-crlf-with-tab",b"alpha\tbeta\r\ngamma",TEXT,"tabs.txt"),
    ]:
        yield name,raw,mime,dict(fileLabel=label),None
    for name,raw,mime,label,code in [
        ("input-over-4-kib",upper_file+b"Z",TEXT,LABEL,"INPUT_LIMIT"),
        ("overlap-expands-over-limit",b"a"*4096,TEXT,LABEL,"INDEX_LIMIT"),
        ("33-paragraphs",b"\n\n".join(b"part" for _ in range(33)),TEXT,LABEL,"INDEX_LIMIT"),
        ("empty-input",b"",TEXT,LABEL,"EMPTY_TEXT"),
        ("whitespace-only",b"\n \t\r"*32,TEXT,LABEL,"EMPTY_TEXT"),
        ("invalid-utf8",b"\xff",TEXT,LABEL,"INVALID_TEXT"),
        ("bad-control",b"alpha\0beta",TEXT,LABEL,"INVALID_TEXT"),
        ("wrong-media",STANDARD,"image/png",LABEL,"INVALID_RESOURCE"),
        ("absolute-reference",STANDARD,TEXT,r"C:\data\file.txt","INVALID_LABEL"),
        ("relative-directory-ref",STANDARD,TEXT,"inner/outer.txt","INVALID_LABEL"),
        ("dot-file-as-label",STANDARD,TEXT,"..txt","INVALID_LABEL"),
        ("python-extension-reference",STANDARD,TEXT,"file.py","INVALID_LABEL"),
        ("source-reference-over-128",STANDARD,TEXT,"x"*125+".txt","INVALID_LABEL"),
        ("reference-control",STANDARD,TEXT,"unicode\u202ebad.txt","INVALID_LABEL"),
        ("empty-label",STANDARD,TEXT,"","INVALID_LABEL"),
    ]:
        yield name,raw,mime,dict(fileLabel=label),code
    yield "extra-setting",STANDARD,TEXT,dict(fileLabel=LABEL,sourceId="other"),"INVALID_REQUEST"


def wire(case):
    _,source,mime,params,_=case
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",
                 commandId=None,resourceOutputProfile=OUTPUT,input=params)
    resource=dict(resourceId="8bb27a8a-139b-42a0-812b-383123234556",mimeType=mime,
                  byteLength=len(source),contentVersion=sha(source))
    return request,resource


def run_native(executable,case,scratch):
    request,resource=wire(case)
    path=scratch/PATH
    target=scratch/"index.sqlite"
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
    bundle=json.loads(process.stdout[12:end])
    raw=process.stdout[end:]
    if bundle["resources"]:
        meta,=bundle["resources"]
        assert meta["mimeType"]==SQLITE and meta["byteLength"]==len(raw) and meta["contentVersion"]==sha(raw)
    else:
        assert not raw
    return bundle["result"],raw or None


def expected_sections(source):
    results=[]
    for i,paragraph in enumerate(source.decode("utf-8-sig").split("\n\n"),1):
        text=paragraph.strip()
        start=0
        while start<len(text):
            end=min(len(text),start+800)
            excerpt=text[start:end]
            results.append((len(results)+1,f"段落 {i}",sha(excerpt.encode()),excerpt))
            if end==len(text):
                break
            start=end-100
    return results


def validate_database(raw,source,label):
    assert 36*1024<=len(raw)<=128*1024 and raw.startswith(b"SQLite format 3\0")
    assert raw[16:20]==b"\x10\x00\x01\x01"
    chunks=expected_sections(source)
    corpus=sum(len(text.encode()) for _,_,_,text in chunks)
    with closing(sqlite3.connect(":memory:")) as db:
        db.deserialize(raw)
        assert db.execute("PRAGMA integrity_check").fetchall()==[("ok",)]
        assert db.execute("PRAGMA user_version").fetchone()==(1,)
        assert db.execute("SELECT source_id,relative,sha256,bytes,text_bytes,chunk_count,indexed_at FROM indexed_files").fetchall()==[
            (SOURCE_ID,label,sha(source),len(source),corpus,len(chunks),"time not recorded")
        ]
        assert db.execute("SELECT ordinal,location,sha256,text FROM chunks ORDER BY ordinal").fetchall()==chunks
        assert db.execute("SELECT count(*) FROM chunks_fts_docsize").fetchone()==(len(chunks),)
        db.execute("INSERT INTO chunks_fts(chunks_fts,rank) VALUES('integrity-check',1)")
        db.rollback()
    return len(chunks),corpus


def search_cases(source, full=False):
    suffix="full" if full else "basic"
    return [
        ("new-index-real-bm25-"+suffix,source,SQLITE,dict(query="alpha beta",mode="all"),None,[5,1] if full else [2,1]),
        ("new-index-cjk-literal-"+suffix,source,SQLITE,dict(query="桌面检索",mode="auto"),None,[6] if full else [3]),
        ("new-index-source-filter-"+suffix,source,SQLITE,dict(query="alpha",sourceId="other"),None,[]),
    ]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for flag in ("worker","native","module","search-module","evidence","scratch"):
        parser.add_argument("--"+flag,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    keyword=args.search_module.read_bytes()
    assert sha(keyword)=="4994a96cd134a140fcda44b8f87e0c07772a980b9332c6aaac5b7e31b727ad94"
    checks,followup,failures=[],[],[]
    with TemporaryDirectory(dir=args.scratch) as temporary:
        scratch=Path(temporary)
        for case in fixtures():
            name,source,mime,params,wanted=case
            work=scratch/name
            work.mkdir()
            try:
                result,sqlite=run_worker(args.worker,module,case)
                native,db=run_native(args.native,case,work)
                assert result==native,(result,native)
                if wanted is None:
                    assert result["ok"] and sqlite and db,result
                    data=result["data"]
                    proof=validate_database(sqlite,source,params["fileLabel"])
                    assert validate_database(db,source,params["fileLabel"])==proof
                    assert data["sourceSha256"]==sha(source) and data["sourceBytes"]==len(source)
                    assert data["fileLabel"]==params["fileLabel"] and data["sourceId"]==SOURCE_ID
                    assert (data["chunks"],data["textBytes"])==proof
                    assert data["sqliteBytes"]==len(sqlite)
                    if name=="exact-indexed-4096-with-overlap":
                        assert data["textBytes"]==4096
                    if name in ("english-chinese-ranking","full-size-32-chunk-search-index"):
                        full=name=="full-size-32-chunk-search-index"
                        if full:
                            assert (data["sourceBytes"],data["chunks"],data["textBytes"])==(4096,32,4096)
                        for followup_case in search_cases(sqlite,full=full):
                            search,txt=search_worker(args.worker,keyword,followup_case[:5])
                            assert search["ok"] and txt
                            assert [hit["ordinal"] for hit in search["data"]["hits"]]==followup_case[5]
                            followup.append(dict(case=followup_case[0],passed=True,
                                               sqliteSha256=sha(sqlite),outputSha256=sha(txt),
                                               ordinals=followup_case[5]))
                else:
                    assert not result["ok"] and result["error"]["code"]==wanted and not sqlite and not db,result
                checks.append(dict(case=name,sourceSha256=sha(source),inputBytes=len(source),mime=mime,options=params,
                                   code=wanted,outputBytes=len(sqlite) if sqlite else 0,
                                   outputSha256=sha(sqlite) if sqlite else None))
            except (AssertionError,ValueError,sqlite3.Error,subprocess.SubprocessError) as exc:
                failures.append(dict(case=name,error=repr(exc)[:900]))
    proof=dict(inputProfile=INPUT,outputProfile=OUTPUT,wasmSha256=sha(module),wasmBytes=len(module),
               workerSha256=sha(args.worker.read_bytes()),searchWasmSha256=sha(keyword),
               cases=checks,followup=followup,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(checks),followup=len(followup),failures=failures),ensure_ascii=False))
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
