"""Freeze the verified portable shortcut-profile reviewer and its external receipt."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_shortcut_profile_plugin import CAPABILITY, EXECUTION, INPUT, JSON, OCTET, OUTPUT, PLAIN, PLUGIN, TITLE, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = "shortcut-profile"
VERSION = "0.1.0"
SCOPE = "确认共享两份独立版快捷配置，指定导入文件的位置，预览合并或替换及冲突，并确认保存新的配置 JSON。"


def manifest():
    mode = {"type":"string","enum":["merge","replace"]}
    result = obj(text=string(4096), mode=mode, baseCount=integer(32), incomingCount=integer(32),
                 resultCount=integer(64), changedCount=integer(64), conflictPairs=integer(2080), outputBytes=integer(49152))
    ok = obj(ok={"type":"boolean","const":True},data=result,error={"type":"null"})
    reject = obj(ok={"type":"boolean","const":False},data={"type":"null"},error=obj(code=string(48),message=string(256)))
    settings = obj(mode=dict(mode,title="预览方式",description="merge 保留未替换项；replace 仅保留导入项。留空合并。"),
        incomingPosition={"type":"integer","minimum":1,"maximum":2,"title":"导入文件在列表的位置","description":"填写 1 或 2，留空使用第 2 份。另一份是基准。"})
    settings["required"] = []
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Shortcut Profile Review",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1","resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1",EXECUTION:"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,executionProfile=EXECUTION,resourceInputMin=2,resourceInputMax=2,resourceInputMimeTypes=[OCTET,PLAIN],
            inputSchema=settings,outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["快捷配置预览","快捷键导入比较","Shortcut Profile"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次确认的两份便携配置 JSON。")],optional=[]))


def main():
    parser,args=candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package=ROOT/"release/plugin-candidates"/f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt=package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_text(encoding="utf8"))
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module) or proof["wasmBytes"]!=len(module)
            or proof["inputProfile"]!=INPUT or proof["outputProfile"]!=OUTPUT or proof["executionProfile"]!=EXECUTION or proof["genericJsonDenied"] is not True or proof["failures"]):
        parser.error("module or production Windows Worker proof does not match the candidate")
    for row,case in zip(proof["cases"],fixtures(),strict=True):
        source=[dict(mime=mime,bytes=len(raw),sha256=digest(raw)) for mime,raw in case["sources"]]
        if row["case"]!=case["name"] or row["sources"]!=source or row["options"]!=case["options"]:
            parser.error("verification case does not match its fixture")
        expected=case["expected"]
        if isinstance(expected,str):
            if (not row["error"] or row["error"]["code"]!=expected or row["output"] is not None
                    or row.get("hostError")!=case.get("host_error")):
                parser.error("unverified rejection in receipt")
        elif expected is None:
            if row["error"] is not None or row["output"] is not None or row["report"]["conflictPairs"]!=case["conflicts"]:
                parser.error("unverified no-output conflict in receipt")
        elif (row["error"] is not None or not row["output"] or row["output"]["mime"]!=JSON
              or not 0<row["output"]["bytes"]<=49152 or len(row["output"]["sha256"])!=64):
            parser.error("unverified shortcut JSON in receipt")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/shortcut-profile/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their own licenses.\n",encoding="utf8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
        packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="unified-shortcuts",capabilityId=CAPABILITY,title=TITLE,status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_shortcut_profile_plugin.py",cases=proof["cases"]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=item.relative_to(package).as_posix(),size=item.stat().st_size,sha256=digest(item.read_bytes()))
           for item in sorted(package.rglob("*")) if item.is_file()]
    index=digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=="__main__":
    main()
