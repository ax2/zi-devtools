"""Freeze the verified SQLite browser package and its external receipt."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from package_disk_inspector_plugin import dump,integer,obj,string
from package_duplicate_files_plugin import candidate_arguments
from verify_sqlite_browser_plugin import CAPABILITY,INPUT,OCTET,OUTPUT,PLUGIN,SQLITE,TITLE,compact,digest,fixtures

ROOT=Path(__file__).resolve().parents[1]
SOURCE="sqlite"
VERSION="0.1.0"
SLUG="sqlite-browser"
SCOPE="逐次确认一份本机 SQLite 静态文件，查看目录和普通表数据页，可明确生成完整页 CSV。"


def manifest():
    yes_no={"type":"boolean"}
    nil={"type":"null"}
    page={"type":"integer","minimum":1,"maximum":16}
    payload=obj(kind={"type":"string","enum":["catalog","table"]},page=page,rowCount=integer(16),
                columnCount=integer(16),hasNext=yes_no,truncatedCells=integer(256),
                csvBytes={"oneOf":[nil,integer(24*1024)]},text=string(48*1024))
    format_version={"type":"string","const":"1.0.0-rc.1"}
    ok=obj(contractVersion=format_version,ok=dict(yes_no,const=True),data=payload,error=nil)
    error=obj(contractVersion=format_version,ok=dict(yes_no,const=False),data=nil,
              error=obj(code=string(40),message=string(256)))
    inp=obj(tableName=dict(string(512),title="表名（留空查看目录）",
                           description="填写目录中的普通表名，再次执行查看具体数据页。"),
            page=dict(page,title="页码（可选）",default=1,description="每页最多 16 项或 16 行。"),
            exportCsv=dict(yes_no,title="生成此页 CSV（可选）",default=False,
                           description="仅从指定普通表生成完整页 CSV。保存时另行确认。"))
    inp["required"]=[]
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Read-only SQLite Browser",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1",
                          "output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[OCTET,SQLITE],
            inputSchema=inp,outputSchema={"type":"object","oneOf":[ok,error]},maxResultBytes=48*1024,
            directory=dict(categoryId="files",aliases=["浏览 SQLite","SQLite 导出 CSV","SQLite Snapshot Browser"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次明确确认的 SQLite 静态快照。")],optional=[]))


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
    with TemporaryDirectory(dir=args.evidence.parent.resolve()) as temp:
        reference=list(fixtures(Path(temp)))
    expected=[dict(case=name,sourceSha256=digest(data),inputBytes=len(data),mime=mime,options=options,
                   code=state if isinstance(state,str) else None,
                   outputSha256=digest(csv) if csv is not None else None)
              for name,data,mime,options,state,_,csv in reference]
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["profile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["failures"]
            or proof["cases"]!=expected):
        parser.error("module or production Windows Worker proof does not match the fixed cases")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/sqlite-browser/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. SQLite 3.46.0 is in the public domain. Other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
        packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_sqlite_browser_plugin.py",cases=expected))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        sqlite="3.46.0",zig="0.15.2",moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
           for path in sorted(package.rglob("*")) if path.is_file()]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=="__main__":
    main()
