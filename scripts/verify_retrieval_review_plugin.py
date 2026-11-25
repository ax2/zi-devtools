"""Check local retrieval-report review using synthetic exports in the Windows production Worker."""

import argparse
import copy
import json
import math
from pathlib import Path
from tempfile import TemporaryDirectory
import struct

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.retrieval-review"
CAPABILITY = "devtools.rag.report.review"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_FILE = 64 * 1024
MAX_RESULT = 8 * 1024

SOURCE_MARKER = "FIXTURE_PRIVATE_SOURCE"
PATH_MARKER = "FIXTURE_PRIVATE_PATH.md"
TEMP_MARKER = "FIXTURE_PRIVATE_FAILURE"
REDACTED = (SOURCE_MARKER, PATH_MARKER, TEMP_MARKER, "FIXTURE_PRIVATE_QUESTION",
            "FIXTURE_PRIVATE_MODEL", "FIXTURE_PRIVATE_TERMS", "FIXTURE_PRIVATE_FRAGMENT",
            "FIXTURE_PRIVATE_ID", "FIXTURE_PRIVATE_LOCATION")


def evidence(position, correct=False, *, source=None, relative=None):
    return dict(id=position, source=source or (SOURCE_MARKER if correct else "其他资料"),
                relative=relative or (PATH_MARKER if correct else "不匹配.md"),
                location="FIXTURE_PRIVATE_LOCATION", file_sha256="abc123ef"*8)


def row(number, *hits, rank=None, failed=False, literal=False):
    return dict(id=f"FIXTURE_PRIVATE_ID-{number}",question="FIXTURE_PRIVATE_QUESTION",
                terms="FIXTURE_PRIVATE_TERMS",expected_source=SOURCE_MARKER,expected_relative=PATH_MARKER,
                expected_answer_contains="FIXTURE_PRIVATE_FRAGMENT" if literal else None,
                evidence_count=len(hits),evidence=list(hits),expected_rank=rank,cited_ids=[],
                expected_cited=None,answer_contains_expected=None,answer=None,
                error=TEMP_MARKER if failed else None,elapsed_ms=(1<<128)-1)


def single(*rows, **overrides):
    report = dict(schema="zi-devtools-rag-eval",version=2,created_at="2026-11-25T00:00:00Z",
                  mode="retrieval-only",retrieval="keyword",embedding_model=None,keyword_weight=None,
                  model=None,top_k=3,total=len(rows),completed=len(rows),
                  retrieval_hits=sum(item["expected_rank"] is not None for item in rows),
                  cited_expected=0,literal_expected_cases=sum(item["expected_answer_contains"] is not None
                                                              for item in rows),
                  literal_answer_matches=0,cancelled=False,results=list(rows))
    report.update(overrides)
    return report


def pair(number, kw=None, mixed=None, *, keyword_failed=False, hybrid_failed=False):
    return dict(id=f"FIXTURE_PRIVATE_ID-{number}",expected_source=SOURCE_MARKER,expected_relative=PATH_MARKER,
                keyword_rank=kw,hybrid_rank=mixed,keyword_ms=(1<<128)-1,hybrid_ms=0,
                keyword_error=TEMP_MARKER if keyword_failed else None,
                hybrid_error=TEMP_MARKER if hybrid_failed else None)


def comparison(*rows, **overrides):
    report = dict(schema="zi-devtools-rag-compare",version=1,created_at="2026-11-25T00:00:00Z",top_k=5,
                  embedding_model="FIXTURE_PRIVATE_MODEL",keyword_weight=50,total=len(rows),
                  completed=len(rows),paired=0,keyword_hits=0,hybrid_hits=0,keyword_only=0,
                  hybrid_only=0,both=0,neither=0,keyword_mrr=0.0,hybrid_mrr=0.0,
                  cancelled=False,results=list(rows))
    report.update(overrides)
    return report


def records():
    base_single = single(row(1,evidence(1),evidence(2,True),evidence(3,True),rank=2,literal=True),
                         row(2,evidence(1,source="其他来源",relative=PATH_MARKER),
                             evidence(2,source=SOURCE_MARKER,relative="另一个文件.md")),
                         row(3,failed=True))
    base_compare = comparison(pair(1,1,5),pair(2,2),pair(3,None,3),pair(4),pair(5,1,hybrid_failed=True),
                              pair(6,None,1,keyword_failed=True),
                              paired=4,keyword_hits=2,hybrid_hits=2,
                              keyword_only=1,hybrid_only=1,both=1,neither=1,
                              keyword_mrr=(1.0+.5)/4,hybrid_mrr=(.2+1.0/3)/4)
    return base_single, base_compare


def fixtures():
    a,b=records()
    expected = {"format":"retrieval-only","topK":3,"total":3,"completed":3,"matchesReport":True,
                "single.retrieval":"keyword","single.failed":1,"single.hits":1,"single.mrr":1/6,
                "single.rows.0.firstRank":2,"single.rows.1.firstRank":None,"single.rows.2.failed":True,
                "comparison":None}
    for label,mime in (("binary",MIME),("json","application/json"),("text","text/plain")):
        yield f"single-derived-rank-and-mrr-{label}",compact(a),{},mime,expected
    expected_compare = {"format":"comparison","topK":5,"total":6,"matchesReport":True,"single":None,
                        "comparison.paired":4,"comparison.excluded":2,
                        "comparison.keywordHits":2,"comparison.hybridHits":2,
                        "comparison.keywordMrr":.375,"comparison.hybridMrr":(.2+1/3)/4,
                        "comparison.keywordOnly":1,"comparison.hybridOnly":1,
                        "comparison.both":1,"comparison.neither":1,
                        "comparison.rows.4.keywordRank":1,"comparison.rows.4.included":False,
                        "comparison.rows.5.hybridRank":1,"comparison.rows.5.keywordFailed":True}
    yield "four-buckets-and-two-excluded",compact(b),{},MIME,expected_compare
    yield "zero-comparable-pairs",compact(comparison(pair(1,keyword_failed=True))),{},MIME,{
        "matchesReport":True,"comparison.paired":0,"comparison.excluded":1,"comparison.keywordMrr":None}
    yield "cancelled-comparison-before-first-case",compact(comparison(total=3,cancelled=True)),{},MIME,{
        "matchesReport":True,"comparison.paired":0,"comparison.hybridMrr":None,"cancelled":True}
    yield "cancelled-single-before-first-case",compact(single(total=3,cancelled=True)),{},MIME,{
        "matchesReport":True,"single.mrr":None,"cancelled":True}
    yield "cancelled-prefix-only",compact(single(a["results"][0],total=10,cancelled=True)),{},MIME,{
        "matchesReport":True,"total":10,"completed":1,"single.mrr":.5,"cancelled":True}
    yield "hybrid-single-top-eight",compact(single(
        row(1,*(evidence(i,i==8) for i in range(1,9)),rank=8),top_k=8,retrieval="hybrid",
        embedding_model="FIXTURE_PRIVATE_MODEL",keyword_weight=75)),{},MIME,{
        "matchesReport":True,"topK":8,"single.retrieval":"hybrid","single.mrr":.125}
    yield "ten-completed-in-export-order",compact(single(*(row(i) for i in range(1,11)))),{},MIME,{
        "matchesReport":True,"total":10,"single.rows.9.caseNumber":10,"single.mrr":0}
    altered=copy.deepcopy(a)
    altered.update(retrieval_hits=3,literal_expected_cases=0,cited_expected=1)
    altered["results"][0]["expected_rank"]=1
    yield "single-reported-rank-counts-and-model-counter-drift",compact(altered),{},MIME,{
        "matchesReport":False,"single.rows.0.firstRank":2,"single.rows.0.recordedRank":1,
        "differences":["expected_rank","retrieval_hits","literal_expected_cases","model_only_counters"]}
    yield "comparison-counts-and-mrr-drift",compact(dict(b,paired=5,hybrid_hits=1,neither=0,hybrid_mrr=0)),{},MIME,{
        "matchesReport":False,"comparison.paired":4,"differences":["paired","hybrid_hits","neither","hybrid_mrr"]}
    for label,source in (
        ("top-k-outside-native-options",dict(a,top_k=4)),
        ("unfinished-without-cancelled-flag",dict(a,total=4)),
        ("completed-count-inconsistent",dict(a,completed=2)),
        ("too-many-rows",single(*(row(i) for i in range(11)))),
        ("duplicated-case-id",single(row(1),row(1))),
        ("blank-case-terms",single(dict(row(1),terms=" "))),
        ("control-in-question",single(dict(row(1),question="question\u0000"))),
        ("incorrect-evidence-numbering",single(row(1,evidence(2,True),rank=2))),
        ("malformed-evidence-hash",single(row(1,dict(evidence(1,True),file_sha256="00"),rank=1))),
        ("evidence-count-mismatch",single(dict(row(1),evidence_count=1))),
        ("evidence-outside-top-k",single(row(1,*(evidence(i) for i in range(1,5))))),
        ("failed-single-has-evidence",single(row(1,evidence(1),failed=True))),
        ("found-rank-outside-top-k",single(row(1,rank=4))),
        ("found-rank-zero",single(row(1,rank=0))),
        ("keyword-mode-with-hybrid-parameters",dict(a,keyword_weight=75)),
        ("hybrid-weight-outside-native-options",dict(a,retrieval="hybrid",embedding_model="e",keyword_weight=60)),
        ("null-able-key-absent",single({key:value for key,value in row(1).items() if key!="error"})),
        ("failed-lane-with-own-rank",comparison(pair(1,1,keyword_failed=True))),
        ("compare-rank-outside-window",comparison(pair(1,None,6))),
        ("compare-weight-outside-native-options",dict(b,keyword_weight=60)),
        ("unexpected-single-field",dict(a,action="save")),
        ("unexpected-compare-case-field",comparison(dict(pair(1),task="not part of format"))),
    ):
        yield label,compact(source),{},MIME,"INVALID_REPORT"
    yield "model-export-not-supported",compact(dict(a,mode="local-model",model=REDACTED[4])),{},MIME,"UNSUPPORTED_MODE"
    yield "old-single-version",compact(dict(a,version=1)),{},MIME,"UNSUPPORTED_VERSION"
    yield "old-comparison-version",compact(dict(b,version=2)),{},MIME,"UNSUPPORTED_VERSION"
    yield "fixture-dataset-is-not-report",compact(dict(version=1,cases=[])),{},MIME,"INVALID_REPORT"
    yield "invalid-json",b'{not-json',{},MIME,"INVALID_REPORT"
    yield "unsupported-file-encoding",b'\xff',{},MIME,"INVALID_REPORT"
    yield "duplicate-version-field",compact(a).replace(b'"version":2',b'"version":2,"version":2'),{},MIME,"INVALID_REPORT"
    yield "wrong-mime",compact(a),{},"image/png","UNSUPPORTED_MIME"
    yield "reject-extra-manual-input",compact(a),{"run":True},MIME,"INVALID_REQUEST"
    padded=compact(a).ljust(MAX_FILE,b" ")
    yield "exact-64-kib-single",padded,{},MIME,expected
    yield "one-over-64-kib",padded+b" ",{},MIME,"INPUT_TOO_LARGE"
    dense=copy.deepcopy(a)
    dense["results"][0]["question"]="星"*999
    dense["results"][0]["evidence"][0]["location"]="\x01"*(9000)
    dense["results"][2]["error"]="\x00"*500
    dense=compact(dense)
    assert len(dense)<MAX_FILE
    yield "escaped-multibyte-at-64-kib",dense.ljust(MAX_FILE,b" "),{},MIME,expected
    yield "exact-64-kib-comparison",compact(b).ljust(MAX_FILE,b"\n"),{},MIME,expected_compare



def envelopes(module, payload, inputs, mime):
    resource = dict(resourceId="12121212-1212-4212-8212-121212121212", byteLength=len(payload),
                    mimeType=mime, contentVersion=digest(payload))
    common = dict(pluginId=PLUGIN, capabilityId=CAPABILITY, sceneId="programming", input=inputs)
    direct = compact(dict(**common, commandId=None, resourceInputProfile=PROFILE,
                          resources=[dict(index=0, **resource)]))
    header = compact(dict(profile=PROFILE, request=dict(workerKind="wasi", workerPath="", **common),
                          module=dict(byteLength=len(module),sha256=digest(module)),resources=[resource]))
    return direct, b"ZIRW\0\0\0\x02" + struct.pack("<I",len(header)) + header + module + payload


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases, failures=[], []
    with TemporaryDirectory() as folder:
        local=Path(folder)/"report.json"
        for name,payload,options,mime,expected in fixtures():
            local.write_bytes(payload)
            direct,wire=envelopes(module,payload,options,mime)
            try:
                native=invoke([args.native,local],direct)
                response=invoke([args.worker,"--zicode-plugin-worker"],wire)
                assert response["ok"], response
                worker=response["result"]
                assert worker==native,(name,"native/WASI mismatch")
                assert worker["contractVersion"]==CONTRACT
                data=worker["data"]
                if isinstance(expected,str):
                    assert not worker["ok"] and worker["error"]["code"]==expected, worker
                    assert data is None and worker["error"]["message"]
                else:
                    assert worker["ok"] and worker["error"] is None, worker
                    assert data["text"]
                    if "matchesReport" in expected and not expected["matchesReport"]:
                        assert "发现 " in data["text"]
                    if name == "single-reported-rank-counts-and-model-counter-drift":
                        assert "首个正确名次 2（报告记录 1）" in data["text"]
                    if name == "comparison-counts-and-mrr-drift":
                        assert data["text"].endswith("配对题数 · 混合命中数 · 两路均未命中数 · 混合 MRR")
                    for path,wanted in expected.items():
                        value=data
                        for key in path.split("."):
                            value=value[int(key)] if isinstance(value,list) else value[key]
                        if isinstance(wanted,float):
                            assert math.isclose(value,wanted,abs_tol=1e-12),(path,value,wanted)
                        else:
                            assert value==wanted,(path,value,wanted)
                assert all(marker.encode() not in compact(worker) for marker in REDACTED)
                assert len(compact(worker))<=MAX_RESULT
                cases.append(dict(case=name,inputSha256=digest(payload),inputBytes=len(payload),
                                  mime=mime,ok=worker["ok"],resultSha256=digest(compact(worker))))
            except Exception as error:
                failures.append(dict(case=name,error=repr(error)[:1000]))
                print(f"FAIL {name} {repr(error)[:160]}",flush=True)
            else:
                print(f"PASS {name}",flush=True)
    proof=dict(passed=len(cases),wasmSha256=digest(module),workerSha256=digest(args.worker.read_bytes()),
               workerProfile=PROFILE,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    raise SystemExit(bool(failures))


if __name__=="__main__":
    main()
