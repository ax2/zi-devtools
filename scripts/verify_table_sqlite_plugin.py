"""Check native and actual Windows Worker SQLite results using SQLite's read-only API."""

import argparse
from contextlib import closing
import csv
import hashlib
from io import StringIO
import json
from pathlib import Path
import sqlite3
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.table-sqlite"
CAPABILITY = "devtools.data.sqlite.export"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
MIME = "application/x-zicode-sqlite"
TITLE = "表格另存 SQLite"
FORMAT_CSV = "text/csv"
FORMAT_JSON = "application/json"
OCTET = "application/octet-stream"
MAX_SOURCE = 16 * 1024


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


def as_csv(headers, rows, delimiter=","):
    output = StringIO(newline="")
    writer = csv.writer(output, delimiter=delimiter)
    writer.writerow(headers)
    writer.writerows(rows)
    return output.getvalue().encode()


def fixtures():
    headers = ["订单号", '名称"中文', "数量"]
    rows = [("001", "汉字🙂, 地点", "3"), ("0002", "跨行\n 留空", "")]
    table = '支出"; DROP TABLE other;--'
    yield "bom-csv-text-exact", b"\xef\xbb\xbf" + as_csv(headers, rows), FORMAT_CSV, dict(format="csv", tableName=table), headers, rows
    rows = [("0003", "\t内含制表", "007")]
    yield "tsv-text-exact", as_csv(headers, rows, "\t"), OCTET, dict(format="tsv"), headers, rows
    yield "cleared-table-name-uses-default", as_csv(headers, rows, "\t"), OCTET, dict(format="tsv",tableName=""), headers, rows
    yield "header-only", as_csv(headers, []), FORMAT_CSV, dict(format="csv"), headers, []
    yield "empty-column-json-table", compact(dict(headers=headers, rows=[])), FORMAT_JSON, dict(format="json"), headers, []
    cells = {"ordered_bool": True, "null": None, "object": {"字": "𠀀", "x": 5},
             "high": 18446744073709551615, "low": -9223372036854775808, "real": 2.125,
             "array": ["a", 2], "string": "0006\0"}
    wanted = {"ordered_bool": 1, "null": None, "object": '{"x":5,"字":"𠀀"}',
              "high": "18446744073709551615", "low": -9223372036854775808, "real": 2.125,
              "array": '["a",2]', "string": "0006\0"}
    headers = sorted(cells)
    second = {"ordered_bool": False, "null": ""}
    yield "json-mixed-and-missing", compact([cells, second]), FORMAT_JSON, dict(format="json"), headers, [
        tuple(wanted.get(key) for key in headers),
        tuple(0 if key == "ordered_bool" else "" if key == "null" else None for key in headers),
    ]
    row = "x" * (MAX_SOURCE - len(b"v\n"))
    yield "exact-16-kib-single-cell", b"v\n" + row.encode(), FORMAT_CSV, dict(format="csv"), ["v"], [(row,)]
    exact_json = compact(dict(headers=["data"], rows=[["🟢 "]]))
    exact_json = exact_json.ljust(MAX_SOURCE, b" ")
    yield "exact-16-kib-json", exact_json, FORMAT_JSON, dict(format="json"), ["data"], [("🟢 ",)]
    headers = [f"c{i}" for i in range(16)]
    rows = [tuple("x" for _ in headers) for n in range(128)]
    yield "exact-128-by-16-csv", as_csv(headers, rows), FORMAT_CSV, dict(format="csv"), headers, rows
    wide_rows = rows[:-1] + [rows[-1][:-1] + ("x"*(1 + MAX_SOURCE-len(as_csv(headers,rows))),)]
    combined = as_csv(headers, wide_rows)
    assert len(combined) == MAX_SOURCE
    yield "combined-dense-csv-max-bytes-and-cells", combined, FORMAT_CSV, dict(format="csv"), headers, wide_rows
    json_rows = [tuple(i * 0.25 - j for j in range(16)) for i in range(128)]
    yield "exact-128-by-16-json", compact(dict(headers=headers, rows=json_rows)), FORMAT_JSON, dict(format="json"), headers, json_rows
    nested = [[] for _ in range(5300)]
    nested_file = compact(dict(headers=["data"],rows=[[nested]])).ljust(MAX_SOURCE,b" ")
    assert len(nested_file) == MAX_SOURCE
    yield "dense-16-kib-nested-json", nested_file, FORMAT_JSON, dict(format="json"), ["data"], [(compact(nested).decode(),)]
    large_name = "\U00020000" * 128
    yield "exact-512-byte-unicode-table-name", as_csv(["a"], [("v",)]), FORMAT_CSV, dict(format="csv", tableName=large_name), ["a"], [("v",)]
    for name, payload, fmt, options, issue in [
        ("plus-one-input-byte", exact_json+b" ", FORMAT_JSON, dict(format="json"), "INPUT_TOO_LARGE"),
        ("plus-one-row", as_csv(headers, rows + rows[:1]), FORMAT_CSV, dict(format="csv"), "INVALID_TABLE"),
        ("plus-one-column", as_csv(headers+["extra"], []), FORMAT_CSV, dict(format="csv"), "INVALID_TABLE"),
        ("jagged-csv", b"first,second\nfirst\n", FORMAT_CSV, dict(format="csv"), "INVALID_TABLE"),
        ("jagged-json", compact(dict(headers=["a","b"],rows=[[1]])), FORMAT_JSON, dict(format="json"), "INVALID_TABLE"),
        ("objects-expand-to-17-columns", compact([dict(zip(headers,range(16))),{"last":1}]), FORMAT_JSON, dict(format="json"), "INVALID_TABLE"),
        ("unsupported-empty-json-array", b"[]", FORMAT_JSON, dict(format="json"), "INVALID_TABLE"),
        ("unplanned-guest-command", b"one\n1\n", FORMAT_CSV, dict(format="csv", command="write"), "INVALID_REQUEST"),
        ("wrong-format", b"one\n1\n", FORMAT_CSV, dict(format="sql"), "INVALID_REQUEST"),
        ("invalid-utf8", b"one\n\xff", FORMAT_CSV, dict(format="csv"), "INVALID_UTF8"),
        ("unsupported-resource", b"one\n1\n", "image/png", dict(format="csv"), "UNSUPPORTED_MIME"),
        ("invalid-table-prefix", b"one\n1\n", FORMAT_CSV, dict(format="csv", tableName="SQLite_hidden"), "INVALID_IDENTIFIERS"),
        ("whitespace-table-name", b"one\n1\n", FORMAT_CSV, dict(format="csv", tableName=" \t"), "INVALID_IDENTIFIERS"),
        ("identifier-nul", b"one\n1\n", FORMAT_CSV, dict(format="csv", tableName="name\0"), "INVALID_IDENTIFIERS"),
        ("513-byte-table-name", b"one\n1\n", FORMAT_CSV, dict(format="csv", tableName=large_name+"x"), "INVALID_IDENTIFIERS"),
        ("ascii-equivalent-columns", b"ID,id\n01,02\n", FORMAT_CSV, dict(format="csv"), "INVALID_IDENTIFIERS"),
    ]:
        yield name,payload,fmt,options,issue,None


def run_case(worker, native, module, directory, case):
    name, payload, mime, options, headers, rows = case
    src, dst, app = (directory/item for item in ("source","native.sqlite","worker.sqlite"))
    src.write_bytes(payload)
    if dst.exists(): dst.unlink()
    if app.exists(): app.unlink()
    resource = dict(resourceId="12345678-1234-4234-8234-987654321010", mimeType=mime,
                    contentVersion=digest(payload), byteLength=len(payload))
    request = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming",
                   resourceOutputProfile=OUTPUT, input=options)
    native_input = compact(dict(**request, commandId=None, resourceInputProfile=INPUT,
                                resources=[dict(index=0,**resource)]))
    local = subprocess.run([native,src,dst], input=native_input, capture_output=True,
                           timeout=15, check=True, creationflags=subprocess.CREATE_NO_WINDOW)
    own = json.loads(local.stdout)
    frame = compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),
                 resources=[resource], module=dict(byteLength=len(module),sha256=digest(module))))
    cloud = subprocess.run([worker,"--zicode-plugin-worker"],
             input=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(frame))+frame+module+payload,
             capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    if cloud.returncode or not cloud.stdout.startswith(b"ZIWO\0\0\0\x01"):
        raise RuntimeError(cloud.stdout.decode(errors="replace")[:350])
    end=12+struct.unpack_from("<I",cloud.stdout,8)[0]
    actual=json.loads(cloud.stdout[12:end])
    blob=cloud.stdout[end:]
    assert actual["result"] == own
    assert src.read_bytes() == payload
    if isinstance(headers,str):
        assert own["ok"] is False and own["error"]["code"]==headers
        assert not actual["resources"] and not blob and not dst.exists()
        return dict(case=name,sourceSha256=digest(payload),mime=mime,options=options,code=headers)
    assert own["ok"] is True
    output, = actual["resources"]
    table = options.get("tableName") or "data"
    assert output["mimeType"] == MIME
    assert output["byteLength"] == len(blob) == own["data"]["byteLength"]
    assert own["data"]["tableName"] == table and own["data"]["rows"] == len(rows)
    assert own["data"]["columns"] == len(headers) and len(blob) <= 128 * 1024
    app.write_bytes(blob)
    for value in (app, dst):
        quoted = '"' + table.replace('"','""') + '"'
        with closing(sqlite3.connect(value.as_uri()+"?mode=ro&immutable=1",uri=True)) as db:
            assert db.execute("PRAGMA integrity_check").fetchone() == ("ok",)
            assert db.execute("SELECT name FROM sqlite_schema ORDER BY name").fetchall() == [(table,)]
            cursor = db.execute(f"SELECT * FROM {quoted} ORDER BY rowid")
            assert [d[0] for d in cursor.description] == headers
            read = cursor.fetchall()
            assert read == rows
            assert [[type(cell) for cell in row] for row in read] == [[type(cell) for cell in row] for row in rows]
    assert blob == dst.read_bytes(), "provider and production Worker SQLite file bytes differ"
    return dict(case=name,sourceSha256=digest(payload),mime=mime,options=options,code=None,
                rows=len(rows),columns=len(headers),bytes=len(blob),sha256=digest(blob))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for parameter in ("worker","native","module","evidence"):
        parser.add_argument("--"+parameter,required=True,type=Path)
    args=parser.parse_args()
    module=args.module.read_bytes()
    rows, failures = [], []
    with TemporaryDirectory(dir=args.evidence.resolve().parent) as scratch:
        for case in fixtures():
            try:
                rows.append(run_case(args.worker,args.native,module,Path(scratch),case))
            except (AssertionError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
                failures.append(dict(case=case[0],failure=str(error)))
    proof=dict(workerSha256=digest(args.worker.read_bytes()),wasmSha256=digest(module),
               wasmBytes=len(module),sqliteReader=sqlite3.sqlite_version,
               profile=INPUT,outputProfile=OUTPUT,cases=rows,failures=failures)
    args.evidence.write_text(json.dumps(proof,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    print(json.dumps({"passed":len(rows),"failures":failures}))
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
