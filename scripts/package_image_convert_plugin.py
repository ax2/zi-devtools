"""Freeze the Windows-verified original-size image conversion package."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump
from package_duplicate_files_plugin import candidate_arguments
from verify_image_convert_plugin import (
    CAPABILITY, INPUT, OUTPUT, PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "image-tools"
SLUG = "image-convert"
VERSION = "0.1.0"
TITLE = "保持原尺寸转换图片"
SCOPE = "明确选择一张静态 PNG、JPEG 或 WebP，保留原宽高并转换成所选格式，可确认另存。"


def manifest():
    # Conversion reuses the supported single-image resource budget and result metadata.
    value = json.loads((ROOT/"plugins/image-resize/plugin.json").read_text(encoding="utf-8"))
    value.update(id=PLUGIN,name="ZiDevTools Original-size Image Conversion",displayName=TITLE,
                 description=SCOPE,version=VERSION,activationEvents=["onCapability:"+CAPABILITY])
    cap, = value["contributes"]["capabilities"]
    cap.update(id=CAPABILITY,title=TITLE,description=SCOPE)
    inp=cap["inputSchema"]
    inp["properties"].pop("maxWidth")
    inp["properties"]["quality"]["default"]=85
    inp["properties"]["quality"]["description"] = "省略时为 85；JPEG 不保留透明度。"
    inp["required"]=["format"]
    for dimension in ("sourceWidth","sourceHeight","width","height"):
        cap["outputSchema"]["properties"][dimension]["maximum"]=12000
    cap["directory"].update(aliases=["图片格式转换","原图尺寸","PNG JPEG WebP 转换","Convert image without resizing"],
                            description=SCOPE)
    return value


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
    expected=[(name,digest(data),mime,options,error)
              for name,data,mime,options,error in vectors]
    actual=[(row["case"],row["inputSha256"],row["mime"],row["input"],row["error"])
            for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["workerProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT
            or proof["executionProfile"]!=PROFILE or proof["failures"]
            or proof["passed"]!=len(vectors) or actual!=expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/image-convert/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",
                                                  inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_image_convert_plugin.py",cases=[
        dict(id=name,inputSha256=digest(data),inputBytes=len(data),input=options,mime=mime,error=error)
        for name,data,mime,options,error in vectors
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
