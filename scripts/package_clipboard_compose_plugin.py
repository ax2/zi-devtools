"""Freeze an unsigned clipboard composition plugin verified by the real Windows Worker."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_clipboard_compose_plugin import CAPABILITY, INPUT, JSON, OCTET, OUTPUT, PLUGIN, TEXT, TITLE, compact, digest, fixtures


ROOT = Path(__file__).resolve().parents[1]
SLUG = "clipboard-compose"
VERSION = "0.1.0"
SCOPE = "从确认的本机 JSON 组合有序正文，生成段落、编号、单列 CSV 或 Markdown 清单。"


def manifest():
    boolean={"type":"boolean"}
    none={"type":"null"}
    data=obj(count=integer(64),mimeType={"type":"string","enum":[TEXT,"text/csv","text/markdown"]},
             byteLength=integer(49152),text=string(1024))
    ok=obj(ok=dict(boolean,const=True),data=data,error=none)
    reject=obj(ok=dict(boolean,const=False),data=none,error=obj(code=string(40),message=string(256)))
    settings=obj(format={"type":"string","enum":["paragraphs","numbered","csv","markdown"],
                         "title":"结果格式","description":"paragraphs 为段落，numbered 为编号，csv 为单列 CSV，markdown 为清单。"})
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Clipboard Text Composition",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1","resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[JSON,TEXT,OCTET],
            inputSchema=settings,outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["文本片段组合","剪贴板 JSON 转列表","Clipboard Text Composition"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次明确确认的一份 JSON 正文集合。")],optional=[]))


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
    expected=[]
    for name,raw,mime,options,verification in fixtures():
        rejected=isinstance(verification,str)
        expected.append(dict(case=name,mime=mime,inputBytes=len(raw),sourceSha256=digest(raw),options=options,
                             code=verification if rejected else None,
                             mimeOutput=None if rejected else verification[1],
                             outputBytes=0 if rejected else len(verification[2]),
                             outputSha256=None if rejected else digest(verification[2])))
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["failures"] or proof["cases"]!=expected):
        parser.error("module or production Windows Worker proof does not match the candidate")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/clipboard-compose/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
        packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="super-clipboard",capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_clipboard_compose_plugin.py",cases=proof["cases"]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        moduleSha256=digest(module),workerVerification=proof))
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
