"""Freeze the Windows Worker-verified static keyword-search plugin."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_keyword_index_plugin import CAPABILITY, INPUT, OCTET, OUTPUT, PLUGIN, SQLITE, TITLE, compact, fixtures, sha

ROOT=Path(__file__).resolve().parents[1]
SLUG="keyword-index"
VERSION="0.1.0"
SCOPE="查询一次明确确认的 ZiDevTools v1 静态知识索引副本，显示可核对的来源、匹配片段与引用；用户确认后保存文本。"


def manifest():
    flag={"type":"boolean"}
    nil={"type":"null"}
    query=dict(string(120),minLength=1,title="关键词",description="最多 120 字符；字面检索 12 个词，FTS5 4 个词项。")
    mode={"type":"string","enum":["auto","phrase","all","any"],"default":"auto",
          "title":"匹配方式（可选）","description":"auto 自动；phrase 字面短语；all 全部词；any 任一词。"}
    source=dict(string(64),title="来源 ID（可选）",description="省略则搜索授权副本中的所有来源。")
    limit=dict(integer(10),minimum=1,default=5,title="最多结果（可选）")
    inputs=dict(obj(query=query,mode=mode,sourceId=source,limit=limit),required=["query"])
    hit=obj(sourceId=string(64),relative=string(512),location=string(160),ordinal=integer(100000),
            fileSha256=string(64),chunkSha256=string(64),excerpt=string(262))
    data=obj(literalMatch=flag,hits={"type":"array","maxItems":10,"items":hit},text=string(24576))
    ok=obj(ok=dict(flag,const=True),data=data,error=nil)
    failure=obj(ok=dict(flag,const=False),data=nil,error=obj(code=string(40),message=string(256)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Static Knowledge Index Keyword Search",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1","resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[SQLITE,OCTET],
            inputSchema=inputs,outputSchema={"type":"object","oneOf":[ok,failure]},maxResultBytes=49152,
            directory=dict(categoryId="files",aliases=["知识索引查询","FTS5 检索","Offline knowledge keyword search"],description=SCOPE)
        )]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次明确确认的静态 SQLite 知识索引副本。")],optional=[]))


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
    scratch=ROOT/".cache"
    scratch.mkdir(exist_ok=True)
    with TemporaryDirectory(dir=scratch) as directory:
        expected=[(name,mime,len(source),sha(source),inputs,code if isinstance(code,str) else None)
                  for name,source,mime,inputs,code in fixtures(Path(directory))]
    actual=[(item["case"],item["mime"],item["inputBytes"],item["inputSha256"],item["options"],item["code"]) for item in proof["cases"]]
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=sha(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["failures"]
            or not proof["threeConcurrentIndependentWorkers"] or expected!=actual
            or not all(item["outputBytes"]==0 and item["outputSha256"] is None if item["code"] else
                       0<item["outputBytes"]<=24576 and len(item["outputSha256"] or "")==64 for item in proof["cases"])):
        parser.error("module or production Windows Worker proof does not match")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    for origin,dest in [(ROOT/"LICENSE","LICENSE"),(ROOT/"plugins/keyword-index/README.md","README.md")]:
        shutil.copyfile(origin,package/dest)
    (package/"NOTICE").write_text("ZiDevTools is MIT. SQLite is public domain. Other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="rag-keyword-search",capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_keyword_index_plugin.py",cases=proof["cases"]))
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
