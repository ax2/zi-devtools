"""Verify SQLite browse and page CSV with the native adapter and production Windows Worker."""

import argparse
from contextlib import closing
import csv
from hashlib import sha256
from io import StringIO
import json
from pathlib import Path
import sqlite3
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.sqlite-browser"
CAPABILITY = "devtools.sqlite.snapshot.browse"
TITLE = "SQLite 浏览器"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
SQLITE = "application/x-zicode-sqlite"
OCTET = "application/octet-stream"
MAX_INPUT = 128 * 1024


def compact(data):
    return json.dumps(data,ensure_ascii=False,separators=(",",":")).encode()


def digest(data):
    return sha256(data).hexdigest()


def make_db(scratch,name,sql,start=None,fill=None):
    path=scratch/(name+".sqlite")
    with closing(sqlite3.connect(path)) as db:
        if start is not None:
            start(db)
        db.executescript(sql)
        if fill is not None:
            fill(db)
        db.commit()
    return path.read_bytes()


def exact_database(scratch):
    path=scratch/"exact.sqlite"
    with closing(sqlite3.connect(path)) as db:
        db.executescript("PRAGMA page_size=512; CREATE TABLE fixed (payload)")
        db.commit()
        lower,upper=120000,131072
        while lower<=upper:
            size=(lower+upper)//2
            db.execute("DELETE FROM fixed")
            db.execute("INSERT INTO fixed VALUES (zeroblob(?))",(size,))
            db.commit()
            db.execute("VACUUM")
            pages=db.execute("PRAGMA page_count").fetchone()[0]
            if pages==256:
                break
            if pages<256:
                lower=size+1
            else:
                upper=size-1
        else:
            raise AssertionError("No exact SQLite boundary fixture")
    payload=path.read_bytes()
    assert len(payload)==MAX_INPUT
    return payload


def csv_bytes(rows):
    stream=StringIO(newline="")
    csv.writer(stream,lineterminator="\n").writerows(rows)
    return stream.getvalue().encode()


def fixtures(scratch):
    rows=[(i,f"{i:03} · 行🧾",None if i%7==0 else i/8) for i in range(1,137)]
    database=make_db(scratch,"many","""
        CREATE TABLE entries(id INTEGER PRIMARY KEY, label, amount);
        CREATE TABLE blank (name);
        CREATE VIEW derived_view AS SELECT id FROM entries;
        CREATE TABLE generated (a, b GENERATED ALWAYS AS (a+1));
        CREATE TABLE tokens (v);
        CREATE TABLE "带""引号" ("=列", token, count, note, optional);
        CREATE TABLE preview_only (oversize, bytes);
        CREATE VIRTUAL TABLE indexed_text USING fts5(body);
        CREATE TABLE indexed_text_backup (legal_but_conservatively_excluded);
        CREATE TABLE community_data (v);
    """,start=lambda db:db.execute("PRAGMA page_size=512"),fill=lambda db:[
        db.executemany("INSERT INTO entries VALUES (?,?,?)", rows),
        db.execute('INSERT INTO "带""引号" VALUES (?,?, ?, ?, ?)',("=2+3","0001",-9223372036854775808,"跨行\n中文",None)),
        db.execute("INSERT INTO preview_only VALUES (?, ?)",("🔹"*150,bytes(range(48)))),
        db.execute("INSERT INTO community_data VALUES ('local-positive')"),
    ])
    assert len(database)>16*512
    expected_catalog=dict(kind="catalog",rowCount=9,hasNext=False)
    yield "catalog",database,SQLITE,{},expected_catalog,["普通表 · \"entries\"","视图 · \"derived_view\"","虚拟表 · \"indexed_text\""],None
    yield "catalog-cleared-name",database,OCTET,dict(tableName=""),expected_catalog,["SQLite 目录"],None
    for page,count in ((1,16),(2,16),(4,16),(9,8),(10,0),(16,0)):
        options=dict(tableName="entries",page=page)
        snippets=[f"第 {page} 页 · {count} 行", '列 · "id" | "label" | "amount"']
        if count:
            first=rows[(page-1)*16]
            snippets.append(first[1])
        want=dict(kind="table",page=page,rowCount=count,columnCount=3,hasNext=page<9)
        yield f"page-{page}",database,SQLITE,options,want,snippets,None
    tagged=csv_bytes([["'=列","token","count","note","optional"],["'=2+3","0001","-9223372036854775808","跨行\n中文",""]])
    yield "quoted-table-formula-protection",database,SQLITE,dict(tableName='带"引号',exportCsv=True),dict(
        kind="table",page=1,rowCount=1,columnCount=5,csvBytes=len(tagged)),["-9223372036854775808","当前页 CSV"],tagged
    third=csv_bytes([["id","label","amount"]] + [
        [i,label,None if amount is None else format(amount,"g")] for i,label,amount in rows[32:48]
    ])
    yield ("page-3-only-csv",database,OCTET,dict(tableName="entries",page=3,exportCsv=True),
           dict(kind="table",page=3,rowCount=16,hasNext=True,csvBytes=len(third)),["033 · 行","048 · 行"],third)
    yield "empty-table",database,SQLITE,dict(tableName="blank"),dict(kind="table",rowCount=0,hasNext=False),["0 行"],None
    yield ("bounded-text-and-blob-preview",database,SQLITE,dict(tableName="preview_only"),
           dict(kind="table",rowCount=1,truncatedCells=2),["BLOB 48 B","仅显示部分内容"],None)
    yield ("ordinary-underscore-table",database,SQLITE,dict(tableName="community_data"),
           dict(kind="table",rowCount=1),["local-positive"],None)
    schema=make_db(scratch,"catalog_pages","".join(f"CREATE TABLE table{i:02}(value);" for i in range(20)),
                   start=lambda db:db.execute("PRAGMA page_size=512"))
    yield ("catalog-page-1",schema,SQLITE,{},dict(kind="catalog",rowCount=16,hasNext=True),["table00","table15"],None)
    yield ("catalog-page-2",schema,SQLITE,dict(page=2),dict(kind="catalog",rowCount=4,hasNext=False),["table16","table19"],None)
    dense_schema=make_db(scratch,"dense_schema","CREATE TABLE arbitrary (a);"+
                         "".join(f"CREATE VIEW view{i:03} AS SELECT a FROM arbitrary;" for i in range(256)))
    limit_schema=make_db(scratch,"limit_schema","CREATE TABLE arbitrary (a);"+
                         "".join(f"CREATE VIEW view{i:03} AS SELECT a FROM arbitrary;" for i in range(63)),
                         start=lambda db:db.execute("PRAGMA page_size=512"))
    over_schema=make_db(scratch,"over_schema","CREATE TABLE arbitrary (a);"+
                        "".join(f"CREATE TABLE table{i:03} (a);" for i in range(64)),
                        start=lambda db:db.execute("PRAGMA page_size=512"))
    yield ("catalog-exact-object-limit",limit_schema,SQLITE,dict(page=4),
           dict(kind="catalog",rowCount=16,hasNext=False),["view047","view062"],None)
    exact=exact_database(scratch)
    yield "exact-128-kib",exact,SQLITE,dict(tableName="fixed"),dict(kind="table",rowCount=1,hasNext=False),["BLOB"],None
    wide=make_db(scratch,"wide","CREATE TABLE wide ("+",".join(["c"+str(i) for i in range(17)])+")")
    utf16=make_db(scratch,"utf16","CREATE TABLE t (a)",start=lambda db:db.execute("PRAGMA encoding='UTF-16le'"))
    assert utf16[56:60]==b"\x00\x00\x00\x02"
    wal=make_db(scratch,"wal","CREATE TABLE sample(v)",start=lambda db:db.execute("PRAGMA journal_mode=WAL"))
    assert wal[18:20]==b"\x02\x02"
    for name,data,mime,options,code in [
        ("byte-limit-plus-one",exact+b"x",SQLITE,{},"INPUT_TOO_LARGE"),
        ("invalid-file",b"not SQLite",OCTET,{},"INVALID_DATABASE"),
        ("partial-file",database[:-1],SQLITE,{},"INVALID_DATABASE"),
        ("shorter-whole-page",database[:-512],SQLITE,{},"INVALID_DATABASE"),
        ("wal-mode",wal,SQLITE,{},"WAL_SNAPSHOT"),
        ("utf16-database",utf16,SQLITE,{},"UNSUPPORTED_ENCODING"),
        ("no-such-table",database,SQLITE,dict(tableName="missing"),"UNSUPPORTED_TABLE"),
        ("sqlite-internal-table",database,SQLITE,dict(tableName="sqlite_schema"),"UNSUPPORTED_TABLE"),
        ("view-not-executed",database,SQLITE,dict(tableName="derived_view"),"UNSUPPORTED_TABLE"),
        ("virtual-table-unsupported",database,SQLITE,dict(tableName="indexed_text"),"UNSUPPORTED_TABLE"),
        ("fts-shadow-not-ordinary",database,SQLITE,dict(tableName="indexed_text_data"),"UNSUPPORTED_TABLE"),
        ("same-virtual-prefix-conservatively-excluded",database,SQLITE,dict(tableName="indexed_text_backup"),"UNSUPPORTED_TABLE"),
        ("object-limit",dense_schema,SQLITE,{},"SCHEMA_LIMIT"),
        ("object-limit-plus-one",over_schema,SQLITE,{},"SCHEMA_LIMIT"),
        ("generated-column-unsupported",database,SQLITE,dict(tableName="generated"),"UNSUPPORTED_TABLE"),
        ("over-column-limit",wide,SQLITE,dict(tableName="wide"),"PAGE_READ_FAILED"),
        ("over-page-limit",database,SQLITE,dict(tableName="entries",page=17),"PAGE_LIMIT"),
        ("page-zero",database,SQLITE,dict(tableName="entries",page=0),"PAGE_LIMIT"),
        ("open-ended-sql-rejected",database,SQLITE,dict(sql="SELECT 1"),"INVALID_REQUEST"),
        ("unsupported-mime",database,"image/png",{},"UNSUPPORTED_MIME"),
        ("empty-page-with-csv",database,SQLITE,dict(tableName="blank",exportCsv=True),"CSV_PAGE_UNAVAILABLE"),
        ("lossy-values-never-exported",database,SQLITE,dict(tableName="preview_only",exportCsv=True),"CSV_PAGE_UNAVAILABLE"),
        ("catalog-csv-rejected",database,SQLITE,dict(exportCsv=True),"CSV_NEEDS_TABLE"),
    ]:
        yield name,data,mime,options,code,[],None


def run_case(case, worker, native, module, temporary):
    name,data,mime,options,expected,snippets,expected_csv=case
    source,csv_path=temporary/"source.sqlite",temporary/"result.csv"
    source.write_bytes(data)
    csv_path.unlink(missing_ok=True)
    descriptor=dict(resourceId="12345678-1234-4234-8234-987654321010", mimeType=mime,
                    byteLength=len(data),contentVersion=digest(data))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",
                 input=options,resourceOutputProfile=OUTPUT)
    envelope=dict(**request,commandId=None,resourceInputProfile=INPUT,resources=[dict(index=0,**descriptor)])
    native_run=subprocess.run([native,source,csv_path],input=compact(envelope),capture_output=True,
                    check=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    expected_native=json.loads(native_run.stdout)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),
                        resources=[descriptor],module=dict(byteLength=len(module),sha256=digest(module))))
    production=subprocess.run([worker,"--zicode-plugin-worker"],
                input=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+data,
                capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    if production.returncode or not production.stdout.startswith(b"ZIWO\0\0\0\x01"):
        raise AssertionError(production.stdout.decode(errors="replace")[:250])
    end=12+struct.unpack_from("<I",production.stdout,8)[0]
    wire=json.loads(production.stdout[12:end])
    report=wire["result"]
    body=production.stdout[end:]
    assert report==expected_native,(report,expected_native)
    assert source.read_bytes()==data
    if isinstance(expected,str):
        assert not report["ok"] and report["error"]["code"]==expected,report
    else:
        assert report["ok"],report
        assert all(report["data"].get(key)==value for key,value in expected.items()),report
        assert all(text in report["data"]["text"] for text in snippets),report
    if expected_csv is None:
        assert not wire["resources"] and not body and not csv_path.exists()
    else:
        output,=wire["resources"]
        assert output["mimeType"]=="text/csv" and output["byteLength"]==len(expected_csv)
        assert body==expected_csv==csv_path.read_bytes(),(body,expected_csv)
    return dict(case=name,sourceSha256=digest(data),inputBytes=len(data),mime=mime,options=options,
                code=expected if isinstance(expected,str) else None,
                outputSha256=digest(expected_csv) if expected_csv is not None else None)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    passes,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.resolve().parent) as location:
        directory=Path(location)
        for case in fixtures(directory):
            try:
                passes.append(run_case(case,args.worker,args.native,module,directory))
            except (AssertionError,subprocess.SubprocessError,ValueError) as error:
                failures.append(dict(case=case[0],failure=str(error)))
    receipt=dict(wasmSha256=digest(module),wasmBytes=len(module),workerSha256=digest(args.worker.read_bytes()),
                 sqliteReader=sqlite3.sqlite_version,profile=INPUT,outputProfile=OUTPUT,cases=passes,failures=failures)
    args.evidence.write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(passes),failures=failures),ensure_ascii=True))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
