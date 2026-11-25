"""Freeze the verified read-only batch rename preview plugin."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_rename_preview_plugin import (
    CAPABILITY, MAX_ENTRIES, MAX_RESULT, MIME, PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "file-rename"
SLUG = "rename-preview"
VERSION = "0.1.0"
TITLE = "批量改名预览"
SCOPE = "切到「文件夹」并确认共享一层名称，预览查找替换、前后缀及目标冲突。最多 64 个目录项。"


def manifest():
    ok = {"type":"boolean"}
    count = {"type":"integer","minimum":0,"maximum":MAX_ENTRIES}
    nullable_name = {"oneOf":[string(255),{"type":"null"}]}
    row = obj(source=string(255),target=nullable_name,
              status={"type":"string","enum":["proposed","unchanged","case-only","duplicate","occupied","invalid"]},
              issue={"oneOf":[string(128),{"type":"null"}]})
    success = obj(ok=dict(ok,const=True),previewOnly=dict(ok,const=True),entryCount=count,fileCount=count,
                  proposedCount=count,unchangedCount=count,blockedCount=count,
                  rows={"type":"array","maxItems":MAX_ENTRIES,"items":row})
    options = {"type":"object","additionalProperties":False,"properties":{
        "find":dict(string(128),title="查找"),
        "replacement":dict(string(128),title="替换为"),
        "prefix":dict(string(128),title="前缀"),
        "suffix":dict(string(128),title="后缀"),
        "caseSensitive":{"type":"boolean","title":"区分大小写"},
    }}
    failure = obj(ok=dict(ok,const=False), error=obj(code=string(32),message=string(256)))
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools Batch Rename Preview",displayName=TITLE,description=SCOPE,
        version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[MIME],
            inputSchema=options,
            outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="files",aliases=["批量重命名","改名预览","文件改名","Batch rename preview"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次确认的目录名称快照。"),dict(name="resource.directory.read",reason="经逐次确认读取一层相对名称和类型。")],optional=[]),
    )


def main():
    parser,args=candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package=ROOT/"release/plugin-candidates"/f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt=package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors=list(fixtures())
    expected=[(name,digest(data),inputs,mime,not isinstance(value,str))
              for name,data,inputs,mime,value in vectors]
    actual=[(row["case"],row["inputSha256"],row["input"],row["mime"],row["ok"])
            for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["workerProfile"]!=PROFILE or proof["passed"]!=len(vectors) or actual!=expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/rename-preview/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",
                                                  inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_rename_preview_plugin.py",cases=[
        dict(id=name,inputSha256=digest(data),inputBytes=len(data),input=inputs,mime=mime,ok=not isinstance(value,str))
        for name,data,inputs,mime,value in vectors
    ]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
                                         builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",
                                         wasmProfile="plugin",moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
           for path in sorted(package.rglob("*")) if path.is_file()]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
                       directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
                                                   objectKeyOrder=["path","size","sha256"],ensureAscii=False,
                                                   separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__ == "__main__":
    main()
