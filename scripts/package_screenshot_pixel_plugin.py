"""Freeze an unsigned PNG pixel reader plugin verified by the real Windows Worker."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_screenshot_pixel_plugin import CAPABILITY, PNG, PROFILE, PLUGIN, TITLE, compact, digest, fixtures


ROOT = Path(__file__).resolve().parents[1]
SLUG = "screenshot-pixel"
VERSION = "0.1.0"
SCOPE = "读取已确认的本机静态 PNG 的一个原始像素，可复制 RGB、Alpha 和十六进制色值。"


def manifest():
    boolean={"type":"boolean"}
    none={"type":"null"}
    coordinate=integer(511)
    dim={"type":"integer","minimum":1,"maximum":512}
    data=obj(x=coordinate,y=coordinate,width=dim,height=dim,
        rgba={"type":"array","minItems":4,"maxItems":4,"items":integer(255)},
        hexRgb=string(7),hexRgba=string(9),text=string(512))
    ok=obj(ok=dict(boolean,const=True),data=data,error=none)
    reject=obj(ok=dict(boolean,const=False),data=none,error=obj(code=string(40),message=string(256)))
    properties=obj(x=dict(coordinate,title="原图 X 坐标",description="从左侧 0 起算的列。"),
                   y=dict(coordinate,title="原图 Y 坐标",description="从顶部 0 起算的行。"))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools PNG Screenshot Pixel",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[PNG],
            inputSchema=properties,outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=4096,
            directory=dict(categoryId="files",aliases=["截图取色","原像素色值","PNG Pixel Color"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次明确确认的一张截图文件。")],optional=[]))


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
    expected=[dict(case=name,mime=mime,inputBytes=len(raw),sourceSha256=digest(raw),options=options,
                   code=assertion if isinstance(assertion,str) else None,
                   rgba=None if isinstance(assertion,str) else list(assertion[2]))
              for name,raw,mime,options,assertion in fixtures()]
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=PROFILE or proof["failures"] or proof["cases"]!=expected):
        parser.error("module or production Windows Worker proof does not match the candidate")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/screenshot-pixel/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
        packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="screenshot-workbench",capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=PROFILE,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_screenshot_pixel_plugin.py",cases=proof["cases"]))
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
