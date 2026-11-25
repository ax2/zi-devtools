"""Exercise collection preview with synthetic exports through Windows production and native adapters."""

import argparse
import json
from pathlib import Path
import struct
from tempfile import TemporaryDirectory

from verify_flow_recipe_plugin import compact, digest, invoke

PLUGIN = "com.zicode.devtools.tool-collections"
CAPABILITY = "devtools.collections.preview"
PROFILE = "resource.wasi.input.draft2"
CONTRACT = "1.0.0-rc.1"
MIME = "application/octet-stream"
MAX_FILE = 128 * 1024
MAX_RESULT = 24 * 1024
PAGE_SIZE = 16


def collection(name, *tool_ids):
    return dict(name=name, toolIds=list(tool_ids))


def document(*items, **overrides):
    source = dict(schema="zi-devtools/tool-collections",schemaVersion=1,collections=list(items))
    source.update(overrides)
    return compact(source)


def accepted(rows, *, comparisons=0, start=1, member_start=1, renamed=None):
    renamed = renamed or {}
    page = rows[start-1:start-1+PAGE_SIZE]
    ids = []
    wire = 0
    for tool in page[0][1][member_start-1:member_start+15]:
        wire += len(compact(tool))
        if wire > 4096:
            break
        ids.append(tool)
    member_end = member_start + len(ids) - 1
    next_member = member_end + 1 if member_end < len(page[0][1]) else None
    return dict(collectionCount=len(rows),toolCount=sum(len(ids) for _,ids in rows),
                comparisonNameCount=comparisons,renamedCount=len(renamed),
                startCollection=start,endCollection=start+len(page)-1,
                collections=[dict(sourceName=name,resolvedName=renamed.get(name,name),
                                  renamed=name in renamed,toolCount=len(members))
                             for name,members in page],
                memberPreview=dict(collection=start,toolCount=len(page[0][1]),toolIds=ids,
                                   startMember=member_start,endMember=member_end,nextMember=next_member,
                                   truncated=member_start>1 or next_member is not None))


def fixtures():
    rows=[("桌面排查",["plugin:offline/unknown","json","Java","java"]),("空集合",[])]
    source=document(*(collection(name,*ids) for name,ids in rows))
    yield "unknown-ids-order-and-empty-collection", source, {}, MIME, accepted(rows)
    yield "json-mime", source, {}, "application/json", accepted(rows)
    yield "text-mime", source, {}, "text/plain", accepted(rows)
    yield "first-order-and-select-second", source, {"startCollection":2}, MIME, accepted(rows,start=2)
    yield "index-zero", source, {"startCollection":0}, MIME, "UNKNOWN_COLLECTION"
    yield "index-outside-file", source, {"startCollection":3}, MIME, "UNKNOWN_COLLECTION"
    fancy="星"*38+"AB"
    clipped=fancy[:40-len("（导入 1）")]+"（导入 1）"
    assert len(clipped)==40
    yield "unicode-suffix-preserves-forty-characters", document(collection(f" {fancy} ","a")), {
        "compareNames":[fancy.lower()]}, MIME, accepted([(fancy,["a"])],comparisons=1,renamed={fancy:clipped})
    rows=[("Équipe",["a"]),("Équipe（导入 1）",["b"])]
    yield "rename-cascade-reserves-predicted-names", document(*(collection(name,*ids) for name,ids in rows)), {
        "compareNames":[" équipe "]}, MIME, accepted(rows,comparisons=1,
            renamed={"Équipe":"Équipe（导入 1）","Équipe（导入 1）":"Équipe（导入 1）（导入 1）"})
    yield "unicode-duplicate-names", document(collection(" Équipe ","a"),collection("équipe","b")), {}, MIME, "DUPLICATE_NAME"
    yield "ordered-suffix-skip", document(collection("Dev","b","a")), {
        "compareNames":["DEV","Dev（导入 1）","Dev（导入 2）"]}, MIME, accepted(
            [("Dev",["b","a"])],comparisons=3,renamed={"Dev":"Dev（导入 3）"})
    collisions=["Dev"]+[f"Dev（导入 {n}）" for n in range(1,127)]
    yield "combined-capacity-and-deep-suffix", document(collection("Dev","a")), {
        "compareNames":collisions}, MIME, accepted([("Dev",["a"])],comparisons=127,renamed={"Dev":"Dev（导入 127）"})
    yield "combined-capacity-over", document(collection("Dev","a")), {
        "compareNames":collisions+["extra"]}, MIME, "COLLECTION_LIMIT"
    yield "comparison-names-reject-duplicate-after-trim", source, {"compareNames":[" DEV ","dev"]}, MIME, "INVALID_COMPARISON_NAMES"
    yield "invalid-comparison-name", source, {"compareNames":["\x00"]}, MIME, "INVALID_COMPARISON_NAMES"
    yield "unknown-input-field", source, {"save":True}, MIME, "INVALID_REQUEST"
    yield "extra-preferences-field", document(collection("a","b"),favorites=["hidden"]), {}, MIME, "INVALID_FILE"
    yield "extra-collection-field", document(dict(name="a",toolIds=[],uuid="hidden")), {}, MIME, "INVALID_FILE"
    yield "duplicate-JSON-schema-version-key", b'{"schema":"zi-devtools/tool-collections","schemaVersion":1,"schemaVersion":2,"collections":[]}', {}, MIME, "INVALID_FILE"
    yield "unsupported-schema", document(collection("a"),schema="other"), {}, MIME, "UNSUPPORTED_VERSION"
    yield "unsupported-version", document(collection("a"),schemaVersion=2), {}, MIME, "UNSUPPORTED_VERSION"
    yield "empty-document", b"", {}, MIME, "INVALID_FILE"
    yield "bom-is-not-native-json", b"\xef\xbb\xbf"+source, {}, MIME, "INVALID_FILE"
    yield "invalid-utf8", b'\xff', {}, MIME, "INVALID_FILE"
    yield "no-collections", document(), {}, MIME, "INVALID_COLLECTION_COUNT"
    yield "empty-name", document(collection(" \t ")), {}, MIME, "INVALID_NAME"
    yield "forty-one-characters", document(collection("星"*41)), {}, MIME, "INVALID_NAME"
    yield "control-in-name", document(collection("abc\x00")), {}, MIME, "INVALID_NAME"
    yield "duplicate-tool-id", document(collection("a","json","json")), {}, MIME, "DUPLICATE_TOOL_ID"
    yield "tool-id-empty", document(collection("a","")), {}, MIME, "INVALID_TOOL_ID"
    yield "tool-id-one-byte-over", document(collection("a","é"*256+"x")), {}, MIME, "INVALID_TOOL_ID"
    yield "tool-id-at-512", document(collection("a","é"*256)), {}, MIME, accepted([("a",["é"*256])])
    yield "tool-id-with-unknown-unicode-and-control-is-raw-data", document(collection("a","插件\nA\x00")), {}, MIME, accepted(
        [("a",["插件\nA\x00"])])
    yield "maximally-escaped-512-byte-id-fits-on-a-page", document(collection("a","\x01"*512,"after")), {}, MIME, accepted(
        [("a",["\x01"*512,"after"])])
    members=[f"plugin:test/{n:03}" for n in range(512)]
    yield "exact-members-limit", document(collection("a",*members)), {}, MIME, accepted([("a",members)])
    yield "members-seventeenth-following-first-page", document(collection("a",*members)), {
        "startMember":17}, MIME, accepted([("a",members)],member_start=17)
    yield "members-last-is-accessible", document(collection("a",*members)), {
        "startMember":512}, MIME, accepted([("a",members)],member_start=512)
    yield "member-position-past-end", document(collection("a",*members)), {
        "startMember":513}, MIME, "UNKNOWN_TOOL_POSITION"
    yield "member-position-zero", document(collection("a",*members)), {
        "startMember":0}, MIME, "UNKNOWN_TOOL_POSITION"
    yield "empty-collection-only-default-position", document(collection("a")), {
        "startMember":2}, MIME, "UNKNOWN_TOOL_POSITION"
    yield "select-last-of-several-very-long-ids", document(collection("a","\x01"*512,"\x02"*512)), {
        "startMember":2}, MIME, accepted([("a",["\x01"*512,"\x02"*512])],member_start=2)
    yield "members-one-over", document(collection("a",*members,"tool:over")), {}, MIME, "TOOL_LIMIT"
    many=[("星"*36+f"{n:04}",[f"plugin:tmp/{n:03}"]) for n in range(128)]
    big=document(*(collection(name,*ids) for name,ids in many))
    yield "128-collections-at-full-name-length", big, {}, MIME, accepted(many)
    yield "last-page-remains-accessible-in-source-order", big, {"startCollection":123}, MIME, accepted(many,start=123)
    yield "collection-count-one-over", document(*(collection(name,*ids) for name,ids in many),collection("last")), {}, MIME, "INVALID_COLLECTION_COUNT"
    dense=[f"plugin:measured/{n:03}".ljust(252,"x") for n in range(512)]
    near_bound=document(collection("密集",*dense))
    assert len(near_bound)<MAX_FILE
    exact=near_bound.ljust(MAX_FILE,b" ")
    yield "exact-128-kib-and-512-distinct-members", exact, {}, MIME, accepted([("密集",dense)])
    yield "input-one-byte-over", exact+b" ", {}, MIME, "INPUT_TOO_LARGE"
    yield "wrong-mime", source, {}, "image/png", "UNSUPPORTED_MIME"


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
        local=Path(folder)/"collections.json"
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
                    assert data["text"].endswith("各入口是否可用请在目标工具中心核对。")
                    assert {key:value for key,value in data.items() if key!="text"}==expected
                    if data["memberPreview"]["nextMember"] is not None:
                        assert "继续查看时" in data["text"]
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
