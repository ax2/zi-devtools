"""Freeze a verified single-file ZiDevTools knowledge index builder."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_knowledge_index_build_plugin import (
    CAPABILITY, INPUT, OCTET, OUTPUT, PLUGIN, SOURCE_ID, TEXT, TITLE, compact, fixtures, sha,
)

ROOT=Path(__file__).resolve().parents[1]
SLUG="knowledge-index-build"
VERSION="0.1.0"
SCOPE="从已确认的一份 UTF-8 TXT 创建可离线检索的新 SQLite 知识索引，文件需另行确认保存。"


def manifest():
    boolean={"type":"boolean"}
    nil={"type":"null"}
    label=dict(string(128), minLength=5, title="文本引用文件名",
               description="填写单个 TXT 名称，例如「会议纪要.txt」。用于显示引用。")
    data=obj(fileLabel=string(128),sourceId={"type":"string","const":SOURCE_ID},sourceSha256=string(64),
             sourceBytes=integer(4096),chunks=integer(32),textBytes=integer(4096),
             sqliteBytes=integer(131072),text=string(2048))
    ok=obj(ok=dict(boolean,const=True),data=data,error=nil)
    failure=obj(ok=dict(boolean,const=False),data=nil,error=obj(code=string(40),message=string(256)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Static Knowledge Index Builder",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1","resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[TEXT,OCTET],
            inputSchema=obj(fileLabel=label),outputSchema={"type":"object","oneOf":[ok,failure]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["构建 SQLite 知识索引","静态知识索引生成","Offline TXT knowledge index"],description=SCOPE)
        )]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次明确确认的一份 TXT。")],optional=[]))


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
    expected=[(name,mime,len(source),sha(source),inputs,code) for name,source,mime,inputs,code in fixtures()]
    actual=[(item["case"],item["mime"],item["inputBytes"],item["sourceSha256"],item["options"],item["code"])
            for item in proof["cases"]]
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=sha(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["failures"]
            or expected!=actual or len(proof["followup"])!=6
            or not all(value["passed"] for value in proof["followup"])
            or not all(item["outputBytes"]==0 and item["outputSha256"] is None if item["code"] else
                       0<item["outputBytes"]<=131072 and len(item["outputSha256"] or "")==64 for item in proof["cases"])):
        parser.error("module, validated source vectors or keyword reader proof does not match")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    for origin,dest in [(ROOT/"LICENSE","LICENSE"),(ROOT/"plugins/knowledge-index-build/README.md","README.md")]:
        shutil.copyfile(origin,package/dest)
    (package/"NOTICE").write_text("ZiDevTools is MIT. SQLite is public domain. Other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="rag-index",capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_knowledge_index_build_plugin.py",cases=proof["cases"],followup=proof["followup"]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        moduleSha256=sha(module),workerVerification=proof))
    files=[dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=sha(path.read_bytes()))
           for path in sorted(package.rglob("*")) if path.is_file()]
    index=sha(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=="__main__":
    main()
