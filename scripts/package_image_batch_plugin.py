"""Freeze the verified multi-file PNG batch plugin and its external receipt."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_image_batch_plugin import CAPABILITY, INPUT, MIME, OUTPUT, PLUGIN, TITLE, compact, digest, fixtures


ROOT = Path(__file__).resolve().parents[1]
SLUG = "image-batch"
VERSION = "0.1.0"
SCOPE = "将逐次确认的一至两张本机小型静态 PNG 等比限宽，生成各自可确认保存的独立 PNG。"


def manifest():
    boolean = {"type":"boolean"}
    nil = {"type":"null"}
    dimensions = {"type":"integer","minimum":1,"maximum":64}
    item = obj(inputIndex=integer(1),outputIndex=integer(1),sourceWidth=dimensions,sourceHeight=dimensions,
               width=dimensions,height=dimensions,byteLength=integer(32768))
    result = obj(mimeType={"type":"string","const":MIME},items={"type":"array","minItems":1,"maxItems":2,"items":item},
                 text=string(1024))
    ok = obj(ok=dict(boolean,const=True),data=result,error=nil)
    reject = obj(ok=dict(boolean,const=False),data=nil,error=obj(code=string(40),message=string(256)))
    settings = obj(maxWidth={"type":"integer","minimum":1,"maximum":48,"title":"最大宽度（可选）",
                             "description":"1–48 像素，按原比例缩小。留空保留原尺寸。"})
    settings["required"] = []
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Small PNG Batch",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMax=2,resourceInputMimeTypes=[MIME],
            inputSchema=settings,outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["PNG 批量缩放","图标批量缩放","Small PNG Batch Resize"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次确认的一至两张本机静态 PNG。")],optional=[]))


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
    for row, (name, sources, options, expected) in zip(proof["cases"],fixtures(),strict=True):
        summary=[dict(mime=mime,sha256=digest(raw),bytes=len(raw)) for mime,raw in sources]
        if row["case"]!=name or row["sources"]!=summary or row["options"]!=options:
            parser.error("verification case does not match its fixture")
        if isinstance(expected,str):
            if row["error"]!=expected or row["outputs"]:
                parser.error("unverified rejection in receipt")
        elif row["error"] is not None or len(row["outputs"])!=len(expected) or any(
                output["mime"]!=MIME or not 0<output["bytes"]<=32768 or len(output["sha256"])!=64
                for output in row["outputs"]):
            parser.error("unverified output in receipt")
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["failures"]):
        parser.error("module or production Windows Worker proof does not match the candidate")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/image-batch/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their own licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
        packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId=SLUG,capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_image_batch_plugin.py",cases=proof["cases"]))
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
