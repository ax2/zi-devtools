"""Verify original table-recipe semantics, resource ordering and protected CSV in the Windows Worker."""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.table-pipeline"
CAPABILITY = "devtools.table.pipeline.replay"
TITLE = "运行表格管线并保存 CSV"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
CSV = "text/csv"
OCTET = "application/octet-stream"
PLAIN = "text/plain"
FILE_BYTES = 16 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode()


def column(name, operation, value=""):
    return dict(action="column", column=name, operation=operation, value=value)


def select(*names):
    return dict(action="select_columns", columns=names)


def match(name, predicate, value="", case=True):
    return dict(action="filter", column=name, predicate=predicate, value=value, case_sensitive=case)


def sort(*columns):
    return dict(action="sort", keys=[dict(column=name, descending=descending) for name, descending in columns])


def recipe(steps, version=2, output=None, name="数据演示"):
    data = dict(version=version, name=name, steps=steps)
    if output is not None:
        data["output"] = output
    return compact(data)


CSV_POLICY = dict(format="csv", version=1, protect_formulas=True)
SMALL = b"id,city,n,note\na,  Tokyo ,2,=1+1\nb, Osaka ,10,word\na, Seattle ,1,ignored\nc, x ,9,ignore\nd, Paris ,2,\"hello,world\"\n"
FULL_RECIPE = recipe([column("city", "trim"), column("n", "to_integer"),
    match("city", "not_contains", "X", False), sort(("n", True), ("city", False)),
    dict(action="deduplicate", columns=["id"]), select("note", "city", "n"),
    column("city", "rename", "城市")], version=3, output=CSV_POLICY)
FULL_RESULT = [["城市", "n", "note"], ["Osaka", "10", "word"], ["Paris", "2", "hello,world"],
    ["Tokyo", "2", "'=1+1"]]


def fixtures():
    def case(name, definition, table, expected, steps=1, format="csv", mime=CSV, options=None, **extras):
        return dict(name=name, sources=[(OCTET, definition), (mime, table)],
                    options=dict(format=format) if options is None else options, expected=expected,
                    steps=steps, **extras)

    simple = recipe([select("value")], version=1)
    yield case("seven-step-production", FULL_RECIPE, SMALL, FULL_RESULT, 7)
    yield case("v1-bom-and-crlf", b"\xef\xbb\xbf"+simple,
        b"\xef\xbb\xbfvalue\r\n001\r\n002\r\n", [["value"],["001"],["002"]], options={})
    yield case("v2-tsv", recipe([column("kind", "upper")]),
        "value\tkind\r\n42\t你好ß\r\n".encode(), [["value","kind"],["42","你好SS"]],format="tsv",mime=OCTET)
    yield case("json-big-numbers-stable-sort", recipe([sort(("n", True))]),
        b'{"headers":["n","id"],"rows":[[9007199254740993,"one"],[9007199254740992,"two"],[9007199254740993,"three"],[-2,"negative"]]}',
        [["n","id"],["9007199254740993","one"],["9007199254740993","three"],["9007199254740992","two"],["-2","negative"]],format="json",mime=OCTET)
    yield case("json-array-null-fill", recipe([column("a", "fill_null", "中文"),column("b", "empty_to_null")]),
        b'[{"b":""},{"a":"a","b":false}]', [["a","b"],["中文",""],["a","false"]],2,format="json",mime=OCTET)
    yield case("typed-empty-table", recipe([sort(("value", False))]), b'{"headers":["value"],"rows":[]}',
        [["value"]], format="json",mime=OCTET)
    yield case("empty-result-keeps-csv-header", recipe([match("value", "is_null")]),
        b"value\na\nb\n", [["value"]])
    yield case("formula-prefixes-and-typed-number", recipe([select("=col", "other")]),
        compact(dict(headers=["=col","other"],rows=[[" \t=1",-2],["+2","-2"],["@f", "\ncheck"],["\tcheck",True]])),
        [["'=col","other"],["' \t=1","-2"],["'+2","'-2"],["'@f","'\ncheck"],["'\tcheck","true"]],format="json",mime=OCTET)
    padded = simple+b" "*(FILE_BYTES-len(simple)); limit_table = b"value\n"+b"a"*(FILE_BYTES-7)+b"\n"
    assert len(padded)==len(limit_table)==FILE_BYTES
    yield case("exact-both-file-byte-limits", padded,limit_table,[["value"],["a"*(FILE_BYTES-7)]])
    yield case("both-file-limits-plus-one", padded+b" ",limit_table+b"\n","FILE_LIMIT")
    yield case("recipe-file-limit-plus-one", padded+b" ",b"value\na\n","FILE_LIMIT")
    yield case("table-file-limit-plus-one", simple,limit_table+b"\n","FILE_LIMIT")
    yield case("max-steps-with-small-table",recipe([select("value")]*32),b"value\na\n",[["value"],["a"]],32)
    yield case("too-many-steps",recipe([select("value")]*33),b"value\na\n","INVALID_RECIPE")
    yield case("unsupported-version",recipe([select("value")],version=4),b"value\na\n","INVALID_RECIPE")
    yield case("row-op-in-old-version",recipe([sort(("value",False))],version=1),b"value\na\n","INVALID_RECIPE")
    yield case("invalid-recipe-field",simple[:-1]+b',"cmd":"synthetic"}',b"value\na\n","INVALID_RECIPE")
    yield case("recipe-input-encoding",b"\xff",b"value\na\n","INVALID_RECIPE")
    yield case("data-input-encoding",simple,b"value\n\xff","INVALID_TABLE")
    yield case("unsupported-table-format",simple,b"value\na\n","INVALID_REQUEST",format="xml")
    yield case("bad-csv-header",simple,b"value,value\na,b\n","INVALID_TABLE")
    yield case("wrong-row-width",simple,b"value,key\na\n","INVALID_TABLE")
    yield case("missing-column-at-step-two",recipe([select("value"),select("missing")]),b"value\na\n","STEP_FAILED",error_step=2)
    yield case("invalid-conversion-at-step-two",recipe([select("value"),column("value","to_integer")]),b"value\na\n","STEP_FAILED",error_step=2)
    yield case("reject-csv-without-formula-protection",recipe([select("value")],version=3,
        output=dict(format="csv",version=1,protect_formulas=False)),b"value\n=1+1\n","CSV_POLICY")
    yield case("reject-sqlite-recipe-output",recipe([select("value")],version=3,
        output=dict(format="sqlite",version=1,table="value")),b"value\na\n","CSV_POLICY")
    yield case("one-resource",simple,b"value\na\n","INVALID_RESOURCE",resource_count=1)
    yield case("same-resource-id",simple,b"value\na\n","INVALID_RESOURCE",same_id=True, host_error="Invalid resource Worker frame")
    yield case("swapped-resources",b"value\na\n",simple,"INVALID_RECIPE",mime=OCTET)
    reverse=case("explicit-role-selects-second-file", FULL_RECIPE, SMALL,FULL_RESULT,7,options=dict(recipePosition=2))
    reverse["sources"].reverse()
    yield reverse
    yield case("invalid-zero-role",simple,b"value\na\n","INVALID_REQUEST",options=dict(recipePosition=0))
    yield case("invalid-extra-role",simple,b"value\na\n","INVALID_REQUEST",options=dict(recipePosition=3))
    yield case("wrong-recipe-content-version",simple,b"value\na\n","INVALID_RESOURCE",wrong_hash=0, host_error="Resource Worker content mismatch")
    yield case("wrong-table-content-version",simple,b"value\na\n","INVALID_RESOURCE",wrong_hash=1, host_error="Resource Worker content mismatch")
    counts=["c"+str(index) for index in range(16)]
    row_values=[f"k{i:02}-{j%10}" for i in range(96) for j in range(16)]
    matrix=[row_values[row*16:row*16+16] for row in range(96)]
    full=[counts,*matrix]
    table=("\n".join(",".join(row) for row in full)+"\n").encode()
    yield case("maximum-row-and-column-counts",recipe([select(*counts)]),table,full)
    yield case("maximum-rows-plus-one",simple,b"value\n"+b"a\n"*97,"INVALID_TABLE")
    yield case("maximum-columns-plus-one",simple,(",".join([*counts,"extra"])+"\n").encode(),"INVALID_TABLE")
    grown=compact(dict(headers=["value"],rows=[[None] for _ in range(96)]))
    yield case("expanded-intermediate-limit",recipe([column("value","fill_null","a"*512)]),grown,"WORK_LIMIT",format="json",mime=OCTET)
    unicode_value="资料ß"*2000
    unicode_table=("value\n"+unicode_value+"\n").encode()
    changes = [column("value","upper"),column("value","lower")]
    yield case("one-large-utf8-uppercase",recipe(changes[:1]),unicode_table, [["value"],["资料SS"*2000]])
    short_unicode=("value\n"+"资料ß"*1000+"\n").encode()
    yield case("two-shorter-utf8-case-conversions",recipe(changes),short_unicode,[["value"],["资料ss"*1000]],2)
    yield case("reject-dense-utf8-case-conversions",recipe(changes*4),unicode_table,"WORK_LIMIT")
    exact="a"*(16*1024-15)
    exact_table=("value\n"+exact+"\n").encode()
    yield case("eight-steps-at-exact-cumulative-limit",recipe([select("value")]*8),exact_table,[["value"],[exact]],8)
    yield case("one-step-over-cumulative-limit",recipe([select("value")]*9),exact_table,"WORK_LIMIT")
    yield case("five-steps-with-max-row-and-column-counts",recipe([select(*counts),
        column("c0","upper"),column("c0","lower"),sort(("c0",False)),match("c0","contains","k")]),table,full,5)
    yield case("six-dense-steps-rejected-before-runtime-budget",recipe([column("c0","trim")]*6),table,"WORK_LIMIT")
    typed_rows=[[9007199254740993-index, 123456789+index, None, False, f"编号{index}"] for index in range(96)]
    typed_table=compact(dict(headers=["a","b","c","d","e"],rows=typed_rows))
    typed_expected=[["a","b","c","d","e"],*[[str(a),str(b),"","false",e] for a,b,_,_,e in typed_rows]]
    keys=(("a",True),("b",False),("c",True),("d",False))
    yield case("typed-multikey-repeated",recipe([sort(*keys)]*8),typed_table,typed_expected,8,format="json",mime=OCTET)
    yield case("reject-dense-multikey-sort",recipe([sort(*keys)]*10),typed_table,"WORK_LIMIT",format="json",mime=OCTET)
    long_above = recipe([column("value","fill_null","你"*13),column("value","upper")])
    yield case("transformation-growth-stays-within-budget",long_above,grown,[["value"]]+[["你"*13]]*96,2,format="json",mime=OCTET)


def wire(case):
    payload = dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming", commandId=None,
                   resourceOutputProfile=OUTPUT,input=case["options"])
    sources = case["sources"][:case.get("resource_count",2)]
    resources = []
    for index,(mime,raw) in enumerate(sources):
        resources.append(dict(resourceId=f"8bb27a8a-139b-42a0-812b-38312323455{0 if case.get('same_id') else index}",mimeType=mime,
                              byteLength=len(raw),contentVersion="0"*64 if case.get("wrong_hash")==index else digest(raw)))
    return payload,resources,sources


def run_native(executable, case, scratch):
    request,resources,_=wire(case)
    files=[]
    for index,(_,blob) in enumerate(case["sources"]):
        path=scratch/f"source-{index}"; path.write_bytes(blob); files.append(path)
    target=scratch/"result.csv"
    envelope=dict(**request,resourceInputProfile=INPUT,resources=[dict(index=index,**r) for index,r in enumerate(resources)])
    done=subprocess.run([executable,*files,target],input=compact(envelope),capture_output=True,check=True,timeout=15,
                        creationflags=subprocess.CREATE_NO_WINDOW)
    assert [path.read_bytes() for path in files]==[blob for _,blob in case["sources"]]
    return json.loads(done.stdout),target.read_bytes() if target.exists() else None


def run_worker(executable,module,case):
    request,resources,sources=wire(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),
                        resources=resources,module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+b"".join(raw for _,raw in sources)
    done=subprocess.run([executable,"--zicode-plugin-worker"],input=frame,capture_output=True,timeout=15,
                        creationflags=subprocess.CREATE_NO_WINDOW)
    if host_error := case.get("host_error"):
        assert done.returncode != 0
        assert json.loads(done.stdout) == dict(ok=False, result=None, error=host_error)
        return None
    assert done.returncode==0 and done.stdout.startswith(b"ZIWO\0\0\0\x01"),(done.stdout[:550],done.stderr[:350])
    end=12+struct.unpack_from("<I",done.stdout,8)[0]
    decoded=json.loads(done.stdout[12:end]); resources=decoded["resources"]
    if not resources:
        assert end==len(done.stdout)
        return decoded["result"],None
    item,=resources; data=done.stdout[end:]
    assert item["index"]==0 and item["mimeType"]==CSV and item["byteLength"]==len(data) and item["contentVersion"]==digest(data)
    return decoded["result"],data


def rows_of(data):
    return list(csv.reader(io.StringIO(data.decode(),newline="")))


def verify(case, report, data):
    expected=case["expected"]
    if isinstance(expected,str):
        assert report["ok"] is False and report["error"]["code"]==expected,report
        assert report["error"].get("step")==case.get("error_step"),report
        assert data is None
    else:
        assert report["ok"] and report["data"]["steps"]==case["steps"],report
        assert report["data"]["columns"]==len(expected[0]) and report["data"]["rowsAfter"]==len(expected)-1
        assert report["data"]["csvBytes"]==len(data) and report["data"]["formulaProtection"]
        got=rows_of(data)
        assert got==expected,(str(got[:3])[:250],str(expected[:3])[:250])


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args=parser.parse_args(); module=args.module.read_bytes(); passed=[]; failures=[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            folder=Path(temporary)/case["name"]; folder.mkdir()
            try:
                windows=run_worker(args.worker,module,case); native=run_native(args.native,case,folder)
                if windows is not None:
                    assert windows==native,str(windows[0])[:400]
                report,data=native
                verify(case,report,data)
                passed.append(dict(case=case["name"],sources=[dict(mime=mime,bytes=len(raw),sha256=digest(raw)) for mime,raw in case["sources"]],
                    options=case["options"],error=report["error"],hostError=case.get("host_error"), output=None if data is None else dict(mime=CSV,bytes=len(data),sha256=digest(data),data=report["data"])))
            except (AssertionError, KeyError, TypeError, ValueError, subprocess.SubprocessError) as error:
                failures.append(dict(case=case["name"],failure=str(error)[:550]))
    proof=dict(workerSha256=digest(args.worker.read_bytes()),wasmSha256=digest(module),wasmBytes=len(module),
        inputProfile=INPUT,outputProfile=OUTPUT,cases=passed,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print(json.dumps(dict(passed=len(passed),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
