"""Verify portable shortcut-profile semantics with native and production Windows Worker adapters."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.shortcut-profile"
CAPABILITY = "devtools.shortcuts.profile.merge"
TITLE = "比较并合并快捷配置"
INPUT = "resource.wasi.input.draft2"
EXECUTION = "resource.wasi.shortcuts.profile.v1"
OUTPUT = "resource.wasi.output.draft1"
JSON = "application/json"
OCTET = "application/octet-stream"
PLAIN = "text/plain"
FILE_LIMIT = 16 * 1024
BASE = [("open:json","Q J"),("open:files",""),("open:plugin:sample/tool","Z O")]
INCOMING = [("open:json"," q k "),("open:data","F A"),("open:files","")]
MERGED = {"open:data":"F A","open:files":"","open:json":"Q K","open:plugin:sample/tool":"Z O"}
REPLACED = {"open:data":"F A","open:files":"","open:json":"Q K"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compact(data):
    return json.dumps(data,ensure_ascii=False,separators=(",",":")).encode()


def profile(bindings,version=1):
    return compact(dict(version=version,bindings=[dict(command=name,sequence=keys) for name,keys in bindings]))


def fixtures():
    def case(name,base,incoming,expected,changes=0,conflicts=0,options=None,**extra):
        sources=[(OCTET,base if isinstance(base,bytes) else profile(base)),(OCTET,incoming if isinstance(incoming,bytes) else profile(incoming))]
        return dict(name=name,sources=sources,expected=expected,changes=changes,conflicts=conflicts,
                    options={} if options is None else options,**extra)
    yield case("native-merge-controls-and-dormant",BASE,INCOMING,MERGED,changes=2)
    yield case("native-replace",BASE,INCOMING,REPLACED,changes=3,options={"mode":"replace"})
    yield case("explicit-incoming-first",INCOMING,BASE,MERGED,changes=2,options={"incomingPosition":1})
    yield case("default-merge-with-empty",[],[],{})
    yield case("replace-removes-when-empty",BASE,[],{},changes=3,options={"mode":"replace"})
    yield case("same-file-contents-with-distinct-ids",BASE,BASE,dict(BASE))
    yield case("import-explicit-disable",[("open:json","A")],[("open:json","")],{"open:json":""},changes=1)
    yield case("different-inactive-commands-allowed",[("open:json","")],[("open:files","")],{"open:json":"","open:files":""},changes=1)
    yield case("native-unicode-whitespace",[],[("open:json"," a\u3000 c \n b \t D ")],{"open:json":"A C B D"},changes=1)
    yield case("retains-fixed-search",[("search","k")],[("open:tool","Q")],{"search":"K","open:tool":"Q"},changes=1)
    yield case("legacy-recorder-controls",[],[("recorder:start","R S"),("recorder:pause","R P"),("recorder:stop","R E")],{"recorder:start":"R S","recorder:pause":"R P","recorder:stop":"R E"},changes=3)
    yield case("ui-reserved-conflict",BASE,[("open:reserved","K O")],None,changes=1,conflicts=1)
    yield case("prefix-conflict-between-files",[("open:a","A")],[("open:b","A Z")],None,changes=1,conflicts=1)
    yield case("equal-conflict-between-files",[("open:a","A Z")],[("open:b","A Z")],None,changes=1,conflicts=1)
    yield case("dormant-prefix-conflict",[("open:plugin:sample/tool","A")],[("open:b","A Z")],None,changes=1,conflicts=1)
    yield case("reserved-search-prefix-in-both-modes",[],[("open:a","K O"),("open:b","K")],None,changes=2,conflicts=3,options={"mode":"replace"})
    yield case("replace-removes-conflict-in-base",[("open:a","Z")],[("open:b","Z B")],{"open:b":"Z B"},changes=2,options={"mode":"replace"})
    yield case("empty-removes-conflict-in-merge",[("open:a","A")],[("open:a",""),("open:b","A Z")],{"open:a":"","open:b":"A Z"},changes=2)
    yield case("same-starting-key-is-not-prefix",[("open:a","A B")],[("open:b","A Z")],{"open:a":"A B","open:b":"A Z"},changes=1)
    yield case("exact-command-length-512",[],[("open:"+"x"*507,"A B")],{"open:"+"x"*507:"A B"},changes=1)
    for key,value in [("empty-open-command",("open:","C")),("unsupported-id",("run-script","C")),
                      ("unsupported-sequence",("open:a","A A A A A")),("non-ascii-letter",("open:a","中")),
                      ("wrong-fixed-k",("search","S")),("command-over-512",("open:"+"x"*508,"A B"))]:
        yield case(key,[],[value],"INVALID_INCOMING")
    yield case("no-base-schema",b"{",[],"INVALID_BASE")
    yield case("wrong-incoming-version",[],profile([],2),"INVALID_INCOMING")
    yield case("unknown-incoming-field",[],b'{"version":1,"bindings":[],"systemPrefix":"ctrl"}',"INVALID_INCOMING")
    yield case("unknown-binding-field",[],b'{"version":1,"bindings":[{"command":"open:a","sequence":"A","tool":true}]}',"INVALID_INCOMING")
    yield case("duplicate-command-rejected",[],[("open:a","A"),("open:a","B")],"INVALID_INCOMING")
    yield case("duplicate-json-keys",[],b'{"version":1,"version":1,"bindings":[]}',"INVALID_INCOMING")
    yield case("invalid-utf8",[],b"\xff","INVALID_INCOMING")
    yield case("unsupported-mode",[],[],"INVALID_REQUEST",options={"mode":"activate"},host_error="invalid_parameters")
    yield case("unsupported-position",[],[],"INVALID_REQUEST",options={"incomingPosition":3},host_error="invalid_parameters")
    yield case("forbidden-parameters",[],[],"INVALID_REQUEST",options={"activeFile":"path"},host_error="invalid_parameters")
    yield case("one-source",[],[],"INVALID_RESOURCE",count=1,host_error="resource_profile_unsupported")
    yield case("same-resource-id",[],[],"INVALID_RESOURCE",duplicate=True,host_error="Invalid resource Worker frame")
    yield case("altered-resource-digest",[],[],"INVALID_RESOURCE",bad_digest=1,host_error="Resource Worker content mismatch")
    yield case("foreign-plugin",[],[],"INVALID_REQUEST",request_patch={"pluginId":"com.example.foreign"},host_error="resource_profile_unsupported")
    yield case("foreign-capability",[],[],"INVALID_REQUEST",request_patch={"capabilityId":"devtools.shortcuts.other"},host_error="resource_profile_unsupported")
    yield case("command-is-disallowed",[],[],"INVALID_REQUEST",request_patch={"commandId":"shortcut-profile"},host_error="Shortcut profile requires authorized resources")
    plain=case("plain-normalization",BASE,INCOMING,MERGED,changes=2)
    plain["sources"][0]=(PLAIN,plain["sources"][0][1]); yield plain
    image=case("reject-image",[],[],"INVALID_RESOURCE",host_error="resource_input_type_unsupported"); image["sources"][0]=("image/png",image["sources"][0][1]); yield image
    left=[(f"open:base-{i:02}-"+"x"*448,f"A {chr(65+i//26)} {chr(65+i%26)}") for i in range(32)]
    right=[(f"open:incoming-{i:02}-"+"x"*448,f"B {chr(65+i//26)} {chr(65+i%26)}") for i in range(32)]
    one,two=profile(left),profile(right)
    one=one+b" "*(FILE_LIMIT-len(one)); two=two+b" "*(FILE_LIMIT-len(two))
    assert len(one)==len(two)==FILE_LIMIT
    yield case("exact-two-file-byte-and-binding-limits",one,two,dict([*left,*right]),changes=32)
    yield case("byte-limit-plus-one",one+b" ",two,"FILE_LIMIT",host_error="resource_input_too_large")
    yield case("binding-limit-plus-one",left+[('open:extra','')],[],"BINDING_LIMIT")
    yield case("dense-conflicts-are-counted-not-truncated",[(f"open:a{i}","Z") for i in range(32)],
        [(f"open:b{i}","Z") for i in range(32)],None,changes=32,conflicts=2016)


def wire(case):
    payload=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",commandId=None,
                 resourceOutputProfile=OUTPUT,input=case["options"])
    payload.update(case.get("request_patch",{}))
    sources=case["sources"][:case.get("count",2)]
    resources=[dict(resourceId=f"8bb27a8a-139b-42a0-812b-38312323455{0 if case.get('duplicate') else i}",mimeType=mime,
        byteLength=len(raw),contentVersion="0"*64 if case.get("bad_digest")==i else digest(raw)) for i,(mime,raw) in enumerate(sources)]
    return payload,resources,sources


def run_native(executable,case,folder):
    request,resources,_=wire(case); files=[]
    for index,(_,raw) in enumerate(case["sources"]):
        path=folder/f"source-{index}.json"; path.write_bytes(raw); files.append(path)
    output=folder/"result.json"
    envelope=dict(**request,resourceInputProfile=INPUT,resources=[dict(index=i,**r) for i,r in enumerate(resources)])
    done=subprocess.run([executable,*files,output],input=compact(envelope),capture_output=True,check=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert [path.read_bytes() for path in files]==[raw for _,raw in case["sources"]]
    return json.loads(done.stdout),output.read_bytes() if output.exists() else None


def run_worker(executable,module,case,execution=EXECUTION):
    request,resources,sources=wire(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",executionProfile=execution,**request),
                        resources=resources,module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+b"".join(raw for _,raw in sources)
    done=subprocess.run([executable,"--zicode-plugin-worker"],input=frame,capture_output=True,timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    if error:=case.get("host_error"):
        assert done.returncode!=0 and json.loads(done.stdout)==dict(ok=False,result=None,error=error)
        return None
    assert done.returncode==0 and done.stdout.startswith(b"ZIWO\0\0\0\x01"),(done.stdout[:500],done.stderr[:350])
    end=12+struct.unpack_from("<I",done.stdout,8)[0]; result=json.loads(done.stdout[12:end]); data=done.stdout[end:]
    if not result["resources"]:
        assert not data
        return result["result"],None
    item,=result["resources"]
    assert item["index"]==0 and item["mimeType"]==JSON and item["byteLength"]==len(data) and item["contentVersion"]==digest(data)
    return result["result"],data


def verify(case,result,data):
    expected=case["expected"]
    if isinstance(expected,str):
        assert not result["ok"] and result["error"]["code"]==expected and data is None,result
        return
    assert result["ok"] and result["error"] is None,result
    report=result["data"]
    assert report["changedCount"]==case["changes"] and report["conflictPairs"]==case["conflicts"],report
    if expected is None:
        assert data is None and report["outputBytes"]==0,report
    else:
        assert data and report["resultCount"]==len(expected) and report["outputBytes"]==len(data),report
        independently=json.loads(data)
        assert independently==dict(version=1,bindings=[dict(command=name,sequence=seq) for name,seq in sorted(expected.items())]),independently


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,required=True,type=Path)
    parser.add_argument('--scratch',type=Path,help='Existing folder accessible to the verifier and native adapter')
    args=parser.parse_args(); module=args.module.read_bytes(); results=[]; failures=[]
    scratch=(args.scratch or args.evidence.parent).resolve()
    if not scratch.is_dir():
        parser.error('scratch must be an existing directory')
    with TemporaryDirectory(dir=scratch) as temporary:
        for case in fixtures():
            folder=Path(temporary)/case["name"]; folder.mkdir()
            try:
                windows=run_worker(args.worker,module,case); native=run_native(args.native,case,folder)
                if windows is not None:
                    assert windows==native,str(windows[0])[:350]
                info,data=native; verify(case,info,data)
                results.append(dict(case=case["name"],sources=[dict(mime=mime,bytes=len(raw),sha256=digest(raw)) for mime,raw in case["sources"]],
                    options=case["options"],error=info["error"],hostError=case.get("host_error"),report=info["data"],
                    output=None if not data else dict(mime=JSON,bytes=len(data),sha256=digest(data))))
            except (AssertionError,KeyError,TypeError,ValueError,subprocess.SubprocessError) as error:
                failures.append(dict(case=case["name"],failure=str(error)[:500]))
    generic_json_denied=False
    try:
        result,data=run_worker(args.worker,module,next(fixtures()),execution=None)
        assert result["error"]["code"]=="OUTPUT_FAILED" and data is None, result
        generic_json_denied=True
    except (AssertionError,KeyError,TypeError,ValueError,subprocess.SubprocessError) as error:
        failures.append(dict(case="generic-draft1-cannot-produce-json",failure=str(error)[:500]))
    proof=dict(workerSha256=digest(args.worker.read_bytes()),wasmSha256=digest(module),wasmBytes=len(module),
        inputProfile=INPUT,outputProfile=OUTPUT,executionProfile=EXECUTION,genericJsonDenied=generic_json_denied,cases=results,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print(json.dumps(dict(passed=len(results),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
