"""Compare the native adapter and real Windows Worker for confirmed directory metadata."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory


PLUGIN="com.zicode.devtools.integration-entries"
CAPABILITY="devtools.integrations.directory.entries"
TITLE="本机工具目录盘点"
INPUT="resource.wasi.input.draft2"
OUTPUT="resource.wasi.output.draft1"
TEXT="text/plain"


def compact(value):
    return json.dumps(value,ensure_ascii=False,separators=(",",":")).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def snapshot(entries):
    return compact(dict(schemaVersion=1,profile="resource.local-directory.draft1",
                        entries=[dict(name=name,kind=kind) for name,kind in entries]))


def fixtures():
    sample = [
        ("JAVA.EXE","file"),("Javac.exe","file"),("mvn.CMD","file"),("node.exe","file"),("uv.EXE","file"),
        ("CODEX.ps1","file"),("Codex.EXE","file"),("docker.exe","directory"),("django-admin.cmd","directory"),
        ("python.exe.bak","file"),("tool-not-reported.txt","file"),("ＧＩＴ.exe","file"),
    ]
    expected = {"Codex CLI":["codex.exe","codex.ps1"],"Java":["java.exe"],"Java 编译器":["javac.exe"],
                "Maven":["mvn.cmd"],"Node.js":["node.exe"],"uv":["uv.exe"]}
    yield "fixture-file-names",snapshot(sample),TEXT,(len(sample),expected)
    yield "reversed-inspection-order",snapshot(sample[::-1]),TEXT,(len(sample),expected)
    yield "case-alias-no-duplicate",snapshot([("GIT.EXE","file"),("git.exe","file")]),TEXT,(2,{"Git":["git.exe"]})
    yield "known-directories-unmatched",snapshot([("java.exe","directory"),("git.exe","directory")]),TEXT,(2,{})
    yield "empty-directory",snapshot([]),TEXT,(0,{})
    aliases={"Ollama":["ollama.exe"],"Codex CLI":["codex.exe","codex.cmd","codex.ps1"],"Claude CLI":["claude.exe","claude.cmd"],
        "Python":["python.exe"],"Java":["java.exe"],"Java 编译器":["javac.exe"],"Maven":["mvn.cmd","mvn.exe"],
        "Gradle":["gradle.bat","gradle.exe"],"Django CLI":["django-admin.exe","django-admin.cmd"],"Node.js":["node.exe"],
        "uv":["uv.exe"],"Docker":["docker.exe"],"Git":["git.exe"]}
    entries=[(value.upper(),"file") for group in aliases.values() for value in group]
    yield "all-native-aliases",snapshot(entries),TEXT,(len(entries),aliases)
    dense_candidates=[(f"{index:03X}.EXE","file") for index in range(2047)]+[("java.exe","file")]
    suite=snapshot(dense_candidates)
    assert len(suite)<=128*1024
    yield "maximum-executable-like-entries",suite,TEXT,(len(dense_candidates),{"Java":["java.exe"]})
    wide_names=[(f"结果-{index:04d}-"+"abc"*8,"file") for index in range(2047)]+[("javac.exe","file")]
    extended=snapshot(wide_names)
    assert len(extended)<=128*1024
    extended+=b" "*(128*1024-len(extended))
    yield "maximum-entries-and-bytes",extended,TEXT,(len(wide_names),{"Java 编译器":["javac.exe"]})
    precise=snapshot(sample)
    precise+=b" "*(128*1024-len(precise))
    yield "exact-128-kib",precise,TEXT,(len(sample),expected)
    for name,raw,mime,code in [
        ("metadata-limit-plus-one",precise+b" ",TEXT,"INPUT_LIMIT"),
        ("entry-limit-plus-one",snapshot(dense_candidates+[("node.exe","file")]),TEXT,"ENTRY_LIMIT"),
        ("initial-4096-short-metadata",snapshot([(f"{i:x}","file") for i in range(4096)]),TEXT,"ENTRY_LIMIT"),
        ("initial-3601-exe-like-metadata",snapshot([(f"{i:03X}.EXE","file") for i in range(3600)]+[("java.exe","file")]),TEXT,"ENTRY_LIMIT"),
        ("unsupported-mime",snapshot(sample),"application/json","INVALID_RESOURCE"),
        ("ordinary-text",b"java.exe\nnode.exe",TEXT,"INVALID_SNAPSHOT"),
        ("invalid-utf8",b"\xff",TEXT,"INVALID_SNAPSHOT"),
        ("wrong-profile",snapshot(sample).replace(b"local-directory.draft1",b"local-directory.draft2"),TEXT,"INVALID_SNAPSHOT"),
        ("empty-entry-name",snapshot([("","file")]),TEXT,"INVALID_SNAPSHOT"),
        ("path-fragment",snapshot([("arbitrary/java.exe","file")]),TEXT,"INVALID_SNAPSHOT"),
        ("windows-path-fragment",snapshot([("sample\\java.exe","file")]),TEXT,"INVALID_SNAPSHOT"),
        ("drive-prefix",snapshot([("C:java.exe","file")]),TEXT,"INVALID_SNAPSHOT"),
        ("special-parent",snapshot([("..","directory")]),TEXT,"INVALID_SNAPSHOT"),
        ("control-name",snapshot([("java\0.exe","file")]),TEXT,"INVALID_SNAPSHOT"),
        ("non-file-kind",snapshot([("java.exe","symlink")]),TEXT,"INVALID_SNAPSHOT"),
    ]:
        yield name,raw,mime,code


def verify(case,worker,native,module,scratch):
    name,source,mime,expected=case
    report,blob=run_worker(worker,module,case)
    assert (report,blob)==run_native(native,case,scratch),report
    if isinstance(expected,str):
        assert not report["ok"] and report["error"]["code"]==expected and blob is None,report
        error=expected
    else:
        count,presences=expected
        assert report["ok"] and blob,report
        data=report["data"]
        assert data["entryCount"]==count and data["found"]==len(presences)
        assert len(data["apps"])==13 and len({item["name"] for item in data["apps"]})==13
        assert {item["name"]:item["names"] for item in data["apps"] if item["names"]}==presences
        assert blob.decode("utf-8")==data["text"]
        assert all(private not in data["text"] for private in ('tool-not-reported','python.exe.bak','ＧＩＴ.exe'))
        error=None
    return dict(case=name,mime=mime,inputBytes=len(source),sourceSha256=digest(source),code=error,
                outputBytes=len(blob) if blob else 0,outputSha256=digest(blob) if blob else None)


def adapter_header(case):
    _,source,mime,_=case
    resource=dict(resourceId="4fad13fe-807b-431b-8bbd-f032b26ee77e",mimeType=mime,byteLength=len(source),contentVersion=digest(source))
    request=dict(pluginId=PLUGIN,capabilityId=CAPABILITY,sceneId="programming",input={},resourceOutputProfile=OUTPUT)
    return request,resource


def run_native(native,case,scratch):
    request,resource=adapter_header(case)
    source,target=scratch/"source.json",scratch/"target.txt"
    source.write_bytes(case[1])
    envelope=dict(**request,commandId=None,resourceInputProfile=INPUT,resources=[dict(index=0,**resource)])
    process=subprocess.run([native,source,target],input=compact(envelope),capture_output=True,check=True,
                           timeout=12,creationflags=subprocess.CREATE_NO_WINDOW)
    assert source.read_bytes()==case[1]
    return json.loads(process.stdout),target.read_bytes() if target.exists() else None


def run_worker(worker,module,case):
    request,resource=adapter_header(case)
    header=compact(dict(profile=INPUT,request=dict(workerKind="wasi",workerPath="",**request),resources=[resource],
                        module=dict(byteLength=len(module),sha256=digest(module))))
    frame=b"ZIRW\0\0\0\x02"+struct.pack("<I",len(header))+header+module+case[1]
    process=subprocess.run([worker,"--zicode-plugin-worker"],input=frame,capture_output=True,
                           timeout=15,creationflags=subprocess.CREATE_NO_WINDOW)
    assert process.returncode==0 and process.stdout.startswith(b"ZIWO\0\0\0\x01"),process.stdout[:350]
    end=12+struct.unpack_from("<I",process.stdout,8)[0]
    wire=json.loads(process.stdout[12:end])
    blob=process.stdout[end:]
    if wire["resources"]:
        resource,=wire["resources"]
        assert resource["mimeType"]==TEXT and resource["byteLength"]==len(blob) and resource["contentVersion"]==digest(blob)
    else:
        assert not blob
    return wire["result"],blob if blob else None


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("worker","native","module","evidence"):
        parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args()
    module=args.module.read_bytes()
    cases,failures=[],[]
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temporary:
        for case in fixtures():
            scratch=Path(temporary)/case[0]
            scratch.mkdir()
            try:
                cases.append(verify(case,args.worker,args.native,module,scratch))
            except (AssertionError,subprocess.SubprocessError,ValueError,TypeError) as error:
                failures.append(dict(case=case[0],message=str(error)[:500]))
    proof=dict(wasmSha256=digest(module),wasmBytes=len(module),workerSha256=digest(args.worker.read_bytes()),
               inputProfile=INPUT,outputProfile=OUTPUT,cases=cases,failures=failures)
    args.evidence.write_text(json.dumps(proof,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(dict(passed=len(cases),failures=failures),ensure_ascii=False))
    return bool(failures)


if __name__=="__main__":
    raise SystemExit(main())
