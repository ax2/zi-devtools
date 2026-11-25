"""Freeze the verified read-only tool collection preview package."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_tool_collections_plugin import (
    CAPABILITY, CONTRACT, MAX_RESULT, MIME, PAGE_SIZE, PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "command-collections"
SLUG = "tool-collections"
VERSION = "0.1.0"
TITLE = "工具集合导入预览"
SCOPE = "检查一份最多 128 KiB 的 ZiDevTools 工具集合导出。可比对手动填写的集合名称、预览改名和按原顺序查看工具 ID。"


def manifest():
    boolean = {"type":"boolean"}
    nil = {"type":"null"}
    contract = {"type":"string","const":CONTRACT}
    natural = lambda maximum: {"type":"integer", "minimum":1, "maximum":maximum}
    collection = obj(sourceName=string(40),resolvedName=string(40),renamed=boolean,toolCount=integer(512))
    members = obj(collection=natural(128),toolCount=integer(512),startMember=natural(512),endMember=integer(512),
                  nextMember={"oneOf":[natural(512),nil]},toolIds={"type":"array","maxItems":16,"items":string(512)},
                  truncated=boolean)
    data = obj(collectionCount=natural(128),toolCount=integer(65536),comparisonNameCount=integer(128),
               renamedCount=integer(128),startCollection=natural(128),endCollection=natural(128),
               collections={"type":"array","maxItems":PAGE_SIZE,"items":collection},memberPreview=members,text=string(12000))
    input_schema = dict(obj(
        compareNames={"type":"array","maxItems":128,"items":string(40),"title":"比较名称（可选）",
                      "description":'填写要模拟占用的名称 JSON 数组，例如 ["工作台","日常"]。'},
        startCollection=dict(natural(128),title="起始集合序号",
                             description="默认 1。按导出顺序列出从该处起的最多 16 个集合。"),
        startMember=dict(natural(512),title="工具起始序号",
                         description="默认 1。按导出顺序查看首个所列集合中的工具 ID。"),
    ),required=[])
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=data,error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools Tool Collection Import Preview",displayName=TITLE,description=SCOPE,
        version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[MIME,"application/json","text/plain"],
            inputSchema=input_schema,
            outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="files",aliases=["工具集合","工具组导入","预览集合改名","Preview tool collections"],
                           description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次确认的本机工具集合导出快照。")],optional=[]),
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
    expected=[(name,digest(data),mime,not isinstance(value,str))
              for name,data,inputs,mime,value in vectors]
    actual=[(row["case"],row["inputSha256"],row["mime"],row["ok"])
            for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["workerProfile"]!=PROFILE or proof["failures"] or proof["passed"]!=len(vectors) or actual!=expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/tool-collections/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",
                                                  inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_tool_collections_plugin.py",cases=[
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
