"""Verify standalone chat JSON and fenced Markdown bytes using the production Windows Worker."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess
from tempfile import TemporaryDirectory

PLUGIN = "com.zicode.devtools.conversation-markdown"
CAPABILITY = "devtools.chat.transcript.to_markdown"
TITLE = "会话文件转 Markdown"
INPUT = "resource.wasi.input.draft2"
OUTPUT = "resource.wasi.output.draft1"
OCTET = "application/octet-stream"
TEXT = "text/plain"
MARKDOWN = "text/markdown"
FILE_BYTES = 64 * 1024
ESCAPE_COUNT = 12 * 1024
SCHEMA = "zi-devtools-conversation"
HEADER = "# Zi DevTools 会话\n\n仅包含成功完成的问答。\n"
SAMPLE = [("如何整理记录？\r\n原始符号```", "结论😀\n````\n<script>sample</script>\n![保留源文字](https://example.invalid/image)"),
          ("第二轮\n---", "包含 ` 行内文字和$input，不连接模型。")]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def compact(data):
    return json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode()


def document(turns):
    return dict(schema=SCHEMA, version=1, messages=[dict(role=role, content=content)
        for question,answer in turns for role,content in (("user",question),("assistant",answer))])


def fitting_turn(fill):
    baseline=len(compact(document([("q", "")])))
    encoded_size=(len(compact(document([("q", fill)])))-baseline)
    content=fill*((FILE_BYTES-baseline)//encoded_size)
    turns=[("q",content)]
    raw=compact(document(turns)); raw=raw+b" "*(FILE_BYTES-len(raw))
    assert len(raw)==FILE_BYTES
    return raw,turns


def fixtures():
    def good(name, turns, mime=OCTET, source=None):
        return dict(name=name, raw=compact(document(turns)) if source is None else source,
                    mime=mime, turns=turns, options={})

    def bad(name, raw, error="INVALID_CONVERSATION", **options):
        return dict(name=name, raw=raw, mime=options.pop("mime",OCTET), error=error, options={}, **options)

    sample=compact(document(SAMPLE))
    yield good("portable-two-turns", SAMPLE)
    yield good("portable-bom-and-whitespace", SAMPLE, TEXT, b"\xef\xbb\xbf  \r\n"+sample+b"\r\n")
    yield good("all-16-turns", [(f"第{turn}轮问题", f"第{turn}轮回答 😀") for turn in range(1,17)])
    yield good("literal-html-and-special-characters",[("~~~\n```````\n\r\n\t# 标题", "&lt; <b>test</b>\n   ``````\n```text\n# 假标题\n\u2028")] )
    for name,fill in [("exact-64k-plain","a"),("exact-64k-utf8","测试😀"),
                      ("exact-64k-longest-fence","`"),("exact-64k-alternating-fence","`a`")]:
        raw,turns=fitting_turn(fill)
        yield good(name, turns, source=raw)
    for name,content in [("quotes",'"'*ESCAPE_COUNT), ("backslashes","\\"*ESCAPE_COUNT),
                         ("mixed",'"\\'*(ESCAPE_COUNT//2)), ("newlines","\n"*ESCAPE_COUNT)]:
        turns=[("q","a"+content)]; raw=compact(document(turns))
        assert len(raw)<FILE_BYTES
        yield good("exact-escape-limit-"+name, turns, source=raw+b" "*(FILE_BYTES-len(raw)))
    yield bad("escape-limit-plus-one",compact(document([("q",'"'*(ESCAPE_COUNT+1))])),"ESCAPE_LIMIT")
    over,_=fitting_turn('"\\')
    yield bad("previous-64k-escape-fuel-failure",over,"ESCAPE_LIMIT")
    turns=[("q","😀测试"*2400)]
    raw=json.dumps(document(turns),ensure_ascii=True,separators=(",",":")).encode()
    yield good("unicode-escapes-including-surrogate-pairs",turns,source=raw+b" "*(FILE_BYTES-len(raw)))
    raw,_=fitting_turn("a")
    yield bad("file-limit-plus-one",raw+b" ","FILE_LIMIT")
    yield bad("invalid-utf8",b"\xff")
    yield bad("comment-is-not-json",b"// comment\n"+sample)
    yield bad("wrong-root-fields",compact(dict(document(SAMPLE),endpoint="https://example.invalid")))
    yield bad("wrong-schema",compact(dict(document(SAMPLE),schema="zi-devtools-saved-conversation")))
    yield bad("wrong-version",compact(dict(document(SAMPLE),version=2)))
    yield bad("missing-reply",compact(dict(document(SAMPLE),messages=document(SAMPLE)["messages"][:-1])))
    yield bad("zero-turns",compact(document([])))
    yield bad("seventeen-turns",compact(document([("q","a")]*17)))
    yield bad("empty-question",compact(document([("\u3000","a")])))
    yield bad("empty-answer",compact(document([("q","\t\r\n")])))
    swapped=document(SAMPLE); swapped["messages"][0]["role"]="assistant"
    yield bad("unexpected-role-order",compact(swapped))
    system=document(SAMPLE); system["messages"][0]["role"]="system"
    yield bad("system-message",compact(system))
    tool=document(SAMPLE); tool["messages"][1]["role"]="tool"
    yield bad("tool-message",compact(tool))
    unknown=document(SAMPLE); unknown["messages"][0]["tool_calls"]=[]
    yield bad("tool-data-field",compact(unknown))
    binary=document(SAMPLE); binary["messages"][1]["content"]={"type":"file","path":"synthetic.md"}
    yield bad("object-content",compact(binary))
    yield bad("duplicate-root-fields",sample[:-1]+b',"version":1}')
    yield bad("resource-not-text",sample,"INVALID_RESOURCE",mime="image/png")
    yield dict(bad("unsupported-extra-setting",sample,"INVALID_REQUEST"),options={"model":"synthetic"})
    yield bad("wrong-resource-content-digest",sample,"INVALID_RESOURCE",host_error="Resource Worker content mismatch")


def wire(case):
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",commandId=None,
                 resourceOutputProfile=OUTPUT,input=case["options"])
    resource=dict(resourceId="8bb27a8a-139b-42a0-812b-383123234552",mimeType=case["mime"],
                  byteLength=len(case["raw"]),contentVersion="0"*64 if case.get("host_error") else digest(case["raw"]))
    return request,resource


def run_native(executable, case, directory):
    request,resource=wire(case)
    source=directory/"conversation.json"; output=directory/"result.md"
    source.write_bytes(case["raw"])
    envelope=dict(**request,resourceInputProfile=INPUT,resources=[dict(index=0,**resource)])
    done=subprocess.run([executable,source,output],input=compact(envelope),capture_output=True,check=True,
                        timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert source.read_bytes()==case["raw"]
    return json.loads(done.stdout),output.read_bytes() if output.exists() else None


def run_worker(executable, module, case):
    request,resource=wire(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),
                        resources=[resource],module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case["raw"]
    done=subprocess.run([executable,"--zicode-plugin-worker"],input=frame,capture_output=True,timeout=15,
                        creationflags=subprocess.CREATE_NO_WINDOW)
    if host_error := case.get("host_error"):
        assert done.returncode!=0 and json.loads(done.stdout)==dict(ok=False,result=None,error=host_error)
        return None
    assert done.returncode==0 and done.stdout.startswith(b"ZIWO\0\0\0\x01"),(done.stdout[:500],done.stderr[:350])
    end=12+struct.unpack_from("<I",done.stdout,8)[0]
    result=json.loads(done.stdout[12:end]); data=done.stdout[end:]
    if not result["resources"]:
        assert not data
        return result["result"],None
    item,=result["resources"]
    assert item["index"]==0 and item["mimeType"]==MARKDOWN and item["byteLength"]==len(data) and item["contentVersion"]==digest(data)
    return result["result"],data


def verify_markdown(markdown, turns):
    """Independently read native syntax, auditing that every source byte stays inside a longer text fence."""
    content=markdown.decode()
    assert content.startswith(HEADER)
    offset=len(HEADER)
    for index,(question,answer) in enumerate(turns,1):
        for role,expected in (("你",question),("模型",answer)):
            heading=f"\n## 第 {index} 轮 · {role}\n\n"
            assert content.startswith(heading,offset)
            offset+=len(heading)
            opening=re.match(r"(`{3,})text\n",content[offset:]); assert opening
            fence=opening[1]
            offset+=opening.end()
            assert not any(len(match[0])>=len(fence) for match in re.finditer(r"`+",expected))
            assert content.startswith(expected+"\n"+fence+"\n",offset)
            offset+=len(expected)+len(fence)+2
    assert offset==len(content)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for item in ("worker","native","module","evidence"):
        parser.add_argument("--"+item,required=True,type=Path)
    args=parser.parse_args(); module=args.module.read_bytes(); results=[]; failures=[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            directory=Path(temporary)/case["name"]; directory.mkdir()
            try:
                worker=run_worker(args.worker,module,case)
                native=run_native(args.native,case,directory)
                if worker is not None:
                    assert worker==native,str(worker[0])[:350]
                info,data=native
                if code:=case.get("error"):
                    assert info["ok"] is False and info["error"]["code"]==code and data is None,info
                    output=None
                else:
                    assert info["ok"] and 0 < len(data) <= 192*1024,info
                    verify_markdown(data,case["turns"])
                    assert info["data"]["turns"]==len(case["turns"]) and info["data"]["markdownBytes"]==len(data)
                    assert info["data"]["sourceBytes"]==len(case["raw"]) and info["data"]["sourceSha256"]==digest(case["raw"])
                    output=dict(mime=MARKDOWN,bytes=len(data),sha256=digest(data),data=info["data"])
                results.append(dict(case=case["name"],mime=case["mime"],inputBytes=len(case["raw"]),sourceSha256=digest(case["raw"]),
                    options=case["options"],error=info["error"],hostError=case.get("host_error"),output=output))
            except (AssertionError, KeyError, TypeError, ValueError, subprocess.SubprocessError) as error:
                failures.append(dict(case=case["name"],failure=str(error)[:500]))
    proof=dict(workerSha256=digest(args.worker.read_bytes()),wasmSha256=digest(module),wasmBytes=len(module),
               inputProfile=INPUT,outputProfile=OUTPUT,cases=results,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf8")
    print(json.dumps(dict(passed=len(results),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
