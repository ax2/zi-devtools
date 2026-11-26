"""Freeze the verified Windows clock backup inspector and its external receipt."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_clock_backup_plugin import CAPABILITY, CONTRACT, MAX_FILE, MAX_WORK, MAX_RESULT, MIME, PLUGIN, PROFILE, check, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = "clock-backup"
VERSION = "0.1.0"
TITLE = "时钟备份检查"
SCOPE = "确认共享本机时钟 JSON v1–v3，核验格式并报告保存时间、计时器、闹钟等摘要。"


def manifest():
    nil = {"type":"null"}
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    fields = obj(schemaVersion={"type":"integer","minimum":1,"maximum":3},byteLength=integer(MAX_FILE),
                 savedAtUtc=string(64),timerCount=integer(32),alarmCount=integer(64),lapCount=integer(1024),
                 noticeCount=integer(97),zoneCount=integer(12),windowSizeCount=integer(48),audioSettingsPresent=boolean,
                 storedTimerPolicy={"type":"string","enum":["pause","continue"]},legacyV1=boolean,text=string(1024))
    ok = obj(contractVersion=contract,ok=dict(boolean,const=True),data=fields,error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Clock Backup Inspector",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+CAPABILITY],runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":PROFILE},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[MIME,"application/json"],
            inputSchema=obj(),outputSchema={"type":"object","oneOf":[ok,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="files",aliases=["时钟会话检查","闹钟备份","计时器备份","Clock backup validation"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="读取本次确认的单份时钟备份。")],optional=[]))


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
    fixed=list(fixtures())
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["moduleBytes"]!=len(module) or proof["workerProfile"]!=PROFILE
            or proof["fileLimit"]!=MAX_FILE or proof["workLimit"]!=MAX_WORK
            or proof["failures"] or proof["passed"]!=len(fixed)):
        parser.error("module or Windows Worker proof does not match the candidate")
    for row,expected in zip(proof["cases"],fixed,strict=True):
        result=compact(row["result"])
        check(row["result"],expected)
        if (row["case"]!=expected["name"] or row["inputSha256"]!=digest(expected["raw"])
                or row["inputBytes"]!=len(expected["raw"]) or row["mime"]!=expected["mime"]
                or row["hostRejected"]!=expected["wrongDigest"] or row["resultBytes"]!=len(result)
                or len(result)>MAX_RESULT or row["resultSha256"]!=digest(result)):
            parser.error("fixed conformance fixture mismatch")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()
    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/clock-backup/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their own licenses. IANA rule data 2025b.\n",encoding="utf8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="clock-workbench",capabilityId=CAPABILITY,title=TITLE,status="candidate",scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_clock_backup_plugin.py",cases=proof["cases"]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        moduleSha256=digest(module),workerVerification=proof))
    files=[dict(path=item.relative_to(package).as_posix(),size=item.stat().st_size,sha256=digest(item.read_bytes()))
           for item in sorted(package.rglob("*")) if item.is_file()]
    index=digest(json.dumps(files,ensure_ascii=False,separators=(",", ":")).encode("utf8"))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=="__main__":
    main()
