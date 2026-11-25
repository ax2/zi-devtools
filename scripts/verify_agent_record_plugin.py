"""Replay synthetic Agent exports through the native and production Windows readers."""

import argparse
from copy import deepcopy
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.agent-record"
CAPABILITY = "devtools.agent-record.inspect"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_FILE = 64 * 1024
MAX_RESULT = 4096
SECRET = "HIDDEN-SYNTHETIC-RECORD"


def provenance(version="1.2.3"):
    app, os, architecture, profile = "0.12.1", "windows", "x86_64", "release"
    canonical = "\0".join(["ZiDevTools runtime environment v1", app, os, architecture, profile])
    fingerprint = dict(schema=1, appVersion=app, os=os, architecture=architecture,
                       buildProfile=profile, sha256=digest(canonical.encode()))
    return dict(toolId="agent-runner", toolVersion=version, runtimeFingerprint=fingerprint)


def step(tool=f"{SECRET}-TOOL-1", **options):
    data=dict(tool=tool,elapsed_ms=314,content_items=2,response_bytes=1524,
              model_excerpt_bytes=126,is_error=False)
    data.update(options)
    return data


def reference(index=None, **options):
    result=dict(source_id=f"{SECRET}-ORIGIN",source_name=f"{SECRET}-SOURCE",
                relative_path=f"preview/{SECRET}-PATH.md",location=f"{SECRET}-LOCATION",
                file_sha256="a"*64,chunk_sha256="b"*64)
    if index is not None:
        result["citation_id"]=index
    result.update(options)
    return result


def record(version=1, **options):
    result=dict(schema="zi-devtools-agent-run",schema_version=version,
                finished_at_utc="2026-11-25T08:30:00Z",status="completed",
                model=f"{SECRET}/MODEL",approved=True,
                allowed_tools=[f"{SECRET}-TOOL-1","search_knowledge",f"{SECRET}-TOOL-3"],
                max_calls=4,calls_made=1,model_tokens_reported=18446744073709551615,
                steps=[step()],content=None)
    if version==5:
        result["generated_by"]=provenance()
    result.update(options)
    return result


def body(**options):
    result=dict(goal=f"{SECRET}-GOAL",plan=f"{SECRET}-PLAN",answer=f"{SECRET}-ANSWER",error=None)
    result.update(options)
    return result


def cited(version=3):
    options=dict(steps=[step("search_knowledge",references=[reference(1),reference(2)])],
                 content=body(answer=f"{SECRET}-ANSWER [K2], [K1], [K1]",citations=[1,2]))
    if version==4:
        options["content"]["context"]=[dict(source=f"{SECRET}-CONTEXT-PATH",content=f"{SECRET}-CONTEXT")]
    return record(version,**options)


def mutate(value, field, new):
    result=deepcopy(value)
    result[field]=new
    return result


def fixtures():
    one=record()
    two=record(2,steps=[step("search_knowledge",references=[reference()])],content=body())
    three=cited()
    four=cited(4)
    five=cited(5)
    yield "v1-numeric-and-order-privacy",compact(one),{},MIME,dict(version=1,toolNumbers=[1],refs=0,citations=0)
    yield "v2-source-count-without-path",compact(two),{},"application/json",dict(version=2,toolNumbers=[2],refs=1,citations=0)
    yield "v3-numbered-sources",compact(three),{},MIME,dict(version=3,toolNumbers=[2],refs=2,citations=2)
    yield "v4-context-and-citations-suppressed",compact(four),{},MIME,dict(version=4,toolNumbers=[2],refs=2,citations=2)
    yield "v5-provenance-and-citations",compact(five),{},MIME,dict(version=5,toolNumbers=[2],refs=2,citations=2)
    yield "v5-metadata-only-export",compact(record(5)),{},MIME,dict(version=5,toolNumbers=[1],refs=0,citations=0)
    cancelled=record(5,status="cancelled",calls_made=0,steps=[],model_tokens_reported=None)
    yield "cancelled-no-recorded-model-usage",compact(cancelled),{},MIME,dict(version=5,toolNumbers=[],status="cancelled",tokens=None)
    failed=record(5,status="failed",steps=[step(f"{SECRET}-TOOL-3",is_error=True)],model_tokens_reported=None)
    yield "native-failed-step",compact(failed),{},MIME,dict(version=5,toolNumbers=[3],status="failed",tokens=None,errors=[True])
    multi=record(5,calls_made=4,steps=[step(f"{SECRET}-TOOL-3"),step(),step(f"{SECRET}-TOOL-3"),step("search_knowledge")])
    yield "multi-step-order-without-labels",compact(multi),{},MIME,dict(version=5,toolNumbers=[3,1,3,2])
    canonical=mutate(one,"finished_at_utc","2026-11-25T08:30:00.1000000000000000+00:00")
    yield "canonical-timestamp-omits-extra-precision",compact(canonical),{},MIME,dict(version=1,toolNumbers=[1],finished="2026-11-25T08:30:00.100Z")
    escaped=record(5,content=body(plan=("中\n"*5800)))
    escaped_bytes=json.dumps(escaped,ensure_ascii=True,separators=(",",":")).encode()
    assert len(escaped_bytes)<=MAX_FILE
    at_limit=escaped_bytes+b" "*(MAX_FILE-len(escaped_bytes))
    yield "escaped-content-exact-64-kib",at_limit,{},MIME,dict(version=5,toolNumbers=[1],refs=0,citations=0)
    copies=(MAX_FILE-len(compact(three)))//5-1
    repeated=mutate(three,"content",body(answer="[K1]"*copies+" [K2]",citations=[1,2]))
    repeated_bytes=compact(repeated)
    assert len(repeated_bytes)<=MAX_FILE
    yield "repeated-citation-matches-near-64-kib",repeated_bytes,{},MIME,dict(version=3,toolNumbers=[2],refs=2,citations=2)
    seed=mutate(three,"content",body(answer=" [K2]",citations=[1,2]))
    occurrences=(MAX_FILE-len(compact(seed)))//len("[K1]")
    full=compact(mutate(three,"content",body(answer="[K1]"*occurrences+" [K2]",citations=[1,2])))
    full+=b" "*(MAX_FILE-len(full))
    assert len(full)==MAX_FILE
    yield "dense-citation-matches-exact-64-kib",full,{},MIME,dict(version=3,toolNumbers=[2],refs=2,citations=2)
    yield "near-limit-many-citations-that-do-not-match",compact(mutate(three,"content",body(answer="[K00000x]"*6900+" [K1] [K2]",citations=[1,2]))),{},MIME,dict(version=3,toolNumbers=[2],refs=2,citations=2)
    yield "file-one-byte-over",at_limit+b" ",{},MIME,"INPUT_TOO_LARGE"
    yield "empty-file",b"",{},MIME,"INVALID_RECORD"
    yield "broken-json",b'{"schema": [}',{},MIME,"INVALID_RECORD"
    yield "broken-utf8",b'{"model":"\xff"}',{},MIME,"INVALID_RECORD"
    yield "native-bom-not-accepted",b'\xef\xbb\xbf'+compact(one),{},MIME,"INVALID_RECORD"
    yield "extra-untrusted-root-field-is-not-echoed",compact(mutate(one,SECRET,"secret")),{},MIME,"INVALID_RECORD"
    yield "unknown-schema-version",compact(mutate(one,"schema_version",6)),{},MIME,"INVALID_RECORD"
    yield "mcp-like-json-is-not-a-record",compact({"jsonrpc":"2.0","operation":"offline-fixture"}),{},MIME,"INVALID_RECORD"
    yield "v5-provenance-required",compact(mutate(five,"generated_by",None)),{},MIME,"INVALID_RECORD"
    wrong=deepcopy(five)
    wrong["generated_by"]["runtimeFingerprint"]["sha256"]="0"*64
    yield "fingerprint-digest-mismatch",compact(wrong),{},MIME,"INVALID_RECORD"
    wrong=deepcopy(five)
    wrong["generated_by"]["toolId"]="random"
    yield "wrong-generator-tool-id",compact(wrong),{},MIME,"INVALID_RECORD"
    yield "older-version-cannot-claim-v5-provenance",compact(mutate(three,"generated_by",provenance())),{},MIME,"INVALID_RECORD"
    yield "valid-rfc3339-but-non-utc-offset",compact(mutate(one,"finished_at_utc","2026-11-25T08:30:00+02:00")),{},MIME,"INVALID_RECORD"
    yield "invalid-calendar-date",compact(mutate(one,"finished_at_utc","2026-11-31T08:30:00Z")),{},MIME,"INVALID_RECORD"
    yield "approval-marker-required",compact(mutate(one,"approved",False)),{},MIME,"INVALID_RECORD"
    yield "allowed-names-cannot-repeat",compact(mutate(one,"allowed_tools",[f"{SECRET}-TOOL-1"]*2)),{},MIME,"INVALID_RECORD"
    yield "step-outside-allowlist",compact(mutate(one,"steps",[step("unauthorized_fixture")])),{},MIME,"INVALID_RECORD"
    yield "steps-count-is-checked",compact(mutate(one,"calls_made",2)),{},MIME,"INVALID_RECORD"
    yield "elapsed-time-bound",compact(mutate(one,"steps",[step(elapsed_ms=86_400_001)])),{},MIME,"INVALID_RECORD"
    yield "byte-count-bound",compact(mutate(one,"steps",[step(response_bytes=4*1024*1024+1)])),{},MIME,"INVALID_RECORD"
    yield "completed-cannot-report-tool-error",compact(mutate(one,"steps",[step(is_error=True)])),{},MIME,"INVALID_RECORD"
    yield "cancelled-cannot-declare-complete-usage",compact(mutate(one,"status","cancelled")),{},MIME,"INVALID_RECORD"
    yield "references-on-unrelated-tool-forbidden",compact(mutate(three,"steps",[step(references=[reference(1)])])),{},MIME,"INVALID_RECORD"
    yield "referenced-path-must-be-relative",compact(mutate(two,"steps",[step("search_knowledge",references=[reference(relative_path="../synthetic.txt")])])),{},MIME,"INVALID_RECORD"
    yield "v3-numbering-gap",compact(mutate(three,"steps",[step("search_knowledge",references=[reference(1),reference(3)])])),{},MIME,"INVALID_RECORD"
    yield "v3-citation-content-mismatch",compact(mutate(three,"content",body(answer="[K2]",citations=[1,2]))),{},MIME,"INVALID_RECORD"
    yield "v4-context-budget",compact(mutate(four,"content",body(answer="[K1] [K2]",citations=[1,2],context=[dict(source="s",content="x"*(32*1024))]))),{},MIME,"INVALID_RECORD"
    yield "wrong-file-type",compact(one),{},"text/plain","UNSUPPORTED_MIME"
    yield "parameters-do-not-accept-path-or-execution",compact(one),{"execute":True},MIME,"INVALID_REQUEST"


def envelopes(module_path, payload, options, mime):
    module=module_path.read_bytes()
    resource=dict(resourceId="12121212-1212-4212-8212-121212121212",byteLength=len(payload),
                  mimeType=mime,contentVersion=digest(payload))
    base=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input=options)
    direct=compact(dict(**base,commandId=None,resourceInputProfile=PROFILE,resources=[dict(index=0,**resource)]))
    header=compact(dict(profile=PROFILE,request=dict(workerKind="wasi",workerPath="",**base),
                 module=dict(byteLength=len(module),sha256=digest(module)),resources=[resource]))
    return direct,b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+payload


def check(result,payload,expected):
    assert result["contractVersion"]==CONTRACT
    assert SECRET.encode() not in compact(result), "private fixture text escaped the report"
    if isinstance(expected,str):
        assert not result["ok"] and result["error"]["code"]==expected, result
        return
    assert result["ok"] and result["error"] is None, result
    data=result["data"]
    assert data["byteLength"]==len(payload) and data["approvalRecorded"] is True,data
    assert data["recordSchemaVersion"]==expected["version"],data
    assert data["finishedAtUtc"]==expected.get("finished","2026-11-25T08:30:00Z"),data
    assert data["status"]==expected.get("status","completed"),data
    assert data["allowedToolCount"]==3 and data["maxCalls"]==4,data
    assert data["modelTokensReported"]==expected.get("tokens","18446744073709551615"),data
    assert data["sourceDeclarationCount"]==expected.get("refs",0),data
    assert data["numberedCitationCount"]==expected.get("citations",0),data
    assert data["fingerprintStructurallyVerified"]==(expected["version"]==5),data
    assert data["generatorToolVersion"]==("1.2.3" if expected["version"]==5 else None),data
    assert data["callsMade"]==len(expected["toolNumbers"]),data
    assert [entry["toolNumber"] for entry in data["steps"]]==expected["toolNumbers"],data
    assert [entry["isError"] for entry in data["steps"]]==expected.get("errors",[False]*len(data["steps"])),data
    assert all(entry["elapsedMs"]==314 and entry["responseBytes"]==1524 for entry in data["steps"]),data


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("native","module","worker","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    args=parser.parse_args()
    cases,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.parent) as temp:
        fixture=Path(temp)/"owned-agent-record.json"
        for case,payload,options,mime,expected in fixtures():
            try:
                fixture.write_bytes(payload)
                direct,wire=envelopes(args.module,payload,options,mime)
                native=invoke([args.native,fixture],direct)
                worker=invoke([args.worker,"--zicode-plugin-worker"],wire)
                assert worker["ok"],worker
                result=worker["result"]
                assert result==native,(case,"native/WASI mismatch")
                check(result,payload,expected)
                wire_result=compact(result)
                assert len(wire_result)<=MAX_RESULT
                assert fixture.read_bytes()==payload
                cases.append(dict(case=case,capabilityId=CAPABILITY,inputSha256=digest(payload),
                                  inputBytes=len(payload),mime=mime,ok=result["ok"],resultSha256=digest(wire_result)))
                print(case,"passed",flush=True)
            except Exception as problem:
                failures.append(dict(case=case,message=repr(problem)))
                print(case,repr(problem)[:850],flush=True)
    evidence=dict(passed=len(cases),wasmSha256=digest(args.module.read_bytes()),
                  workerSha256=digest(args.worker.read_bytes()),workerProfile=PROFILE,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(evidence,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    if failures:
        raise SystemExit(f"{len(failures)} conformance cases failed")


if __name__=="__main__":
    main()
