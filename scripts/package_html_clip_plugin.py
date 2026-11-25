"""Freeze the verified read-only local HTML preview package."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_html_clip_plugin import (
    CAPABILITY, CONTRACT, MAX_BODY, MAX_FILE, MAX_RESULT, MIME, PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "browser-clipper"
SLUG = "local-html"
VERSION = "0.1.0"
TITLE = "本机 HTML 正文预览"
SCOPE = "确认一份本机 UTF-8 HTML，查看标题及最多 6 KiB 的纯文本摘录。单份文件最多 16 KiB。"


def manifest():
    boolean = {"type":"boolean"}
    nil = {"type":"null"}
    contract = {"type":"string","const":CONTRACT}
    data = obj(
        title=string(180),
        byteLength=integer(MAX_FILE),
        bodyByteLength=integer(256 * 1024),
        previewByteLength=integer(MAX_BODY),
        truncated=boolean,
        text=string(8000),
    )
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=data,error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools Local HTML Preview",displayName=TITLE,description=SCOPE,
        version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[MIME],
            inputSchema=obj(),
            outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="files",aliases=["HTML 文件","网页正文提取","HTML 转纯文本","Local HTML text preview"],
                           description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次确认的本机 HTML 文件快照。")],optional=[]),
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
    shutil.copyfile(ROOT/"plugins/local-html/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Scraper, html5ever, Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",
                                                  inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_html_clip_plugin.py",cases=[
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
