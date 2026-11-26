"""Package the exact offline time-zone candidate and its verification receipt."""
import argparse
from datetime import datetime, timezone
import json
import re
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from verify_time_zone_plugin import (
    CAPABILITY, CONTRACT, IANA, MAX_RESULT, PLUGIN, check, compact, digest, fixtures, request,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "time-zone"
SLUG = "time-zone"
VERSION = "0.2.0"
TITLE = "时区工作台"
ACTIVITY = "devtools.time-zone"
VIEW = ACTIVITY + ".main"
OPERATION = ACTIVITY + ".workbench"
VIEW_PATH = "views/main.html"
SCOPE = "用相同 IANA 时区规则离线将一个本地时间换算到最多 8 个时区，明确列出重复或缺失时间。"


def manifest():
    nil = {"type":"null"}
    choice = {"type":"string","enum":["earlier","later"]}
    contract = {"type":"string","const":CONTRACT}

    def nullable(schema):
        return {"oneOf":[schema,nil]}

    wall = string(32)
    offset = string(6)
    instant = obj(utc=string(40), offset=offset)
    row = obj(zone=string(64), local=wall, offset=offset, abbreviation=string(32))
    report = obj(
        status={"type":"string","enum":["unique","ambiguous","nonexistent"]},
        sourceZone=string(64), sourceLocal=wall, sourceOffset=nullable(offset),
        sourceAbbreviation=nullable(string(32)), utc=nullable(string(40)),
        earlier=nullable(instant), later=nullable(instant), selectedChoice=nullable(choice),
        previousValid=nullable(wall), nextValid=nullable(wall),
        rows={"type":"array","maxItems":8,"items":row},
        tzdbVersion={"type":"string","const":IANA}, text=string(2048),
    )
    success = obj(contractVersion=contract,ok={"type":"boolean","const":True},data=report,error=nil)
    failure = obj(contractVersion=contract,ok={"type":"boolean","const":False},data=nil,
                  error=obj(code=string(32),message=string(256)))
    inputs = dict(obj(
        localTime=dict(string(32),minLength=1,title="本地时间",
                       description="格式 YYYY-MM-DD HH:MM[:SS]，例如 2026-11-01 01:30。"),
        sourceZone=dict(string(64),minLength=1,title="来源时区",
                        description="填 IANA 名称，例如 America/New_York。"),
        targetZones={"type":"array","minItems":1,"maxItems":8,"items":dict(string(64),minLength=1),
                     "title":"目的时区",
                     "description":'按需要的顺序填写 JSON 数组，例如 ["Asia/Shanghai","Europe/London"]。输入 1 至 8 个不同名称。'},
        ambiguousChoice=dict(choice,title="重复时间选择（可选）",
                             description="遇到重复时间先查看两个候选，再选 earlier（较早）或 later（较晚）。"),
    ),required=["localTime","sourceZone","targetZones"])
    capability=dict(id=CAPABILITY,title="IANA 时间换算",description=SCOPE,inputSchema=inputs,
                    outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
                    directory=dict(categoryId="data",aliases=["时区转换","IANA 时区","夏令时","会议时间换算","Time Zone","Timezone Converter"],description=SCOPE))
    operation = dict(id=OPERATION,title=TITLE,viewId=VIEW,sourceToolId=SOURCE,
                     directory=dict(categoryId="data",aliases=["时区工作台","时区转换","timezone workbench"],
                                    description="搜索来源与目的时区，换算并逐行复制。"))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Time Zone",displayName=TITLE,
                description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
                engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
                requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1","output.schema.v1":"=1.0.0-rc.1",
                                  "tool.directory.v1":"=1.0.0-preview.1",
                                  "tool.open.v1":"=1.0.0-preview.1",
                                  "view.operation.directory.v1":"=1.0.0-preview.1"},
                activationEvents=["onCapability:"+CAPABILITY,"onView:"+VIEW],runtime={"wasi":"runtime/tools.wasm"},
                contributes=dict(activities=[dict(id=ACTIVITY,title=TITLE,order=872)],
                                 views=[dict(id=VIEW,activity=ACTIVITY,location="main",title=TITLE,entry=VIEW_PATH)],
                                 viewOperations=[operation],commands=[],piTools=[],capabilities=[capability]),
                permissions=dict(required=[],optional=[]))


def view_html():
    source = ROOT/"plugins/time-zone"/VIEW_PATH
    template = source.read_text(encoding="utf-8")
    version, *names = source.with_name("zones.txt").read_text(encoding="utf-8").splitlines()
    marker = "__ZICODE_IANA_ZONES__"
    if (version != f"# IANA {IANA}" or template.count(marker) != 1
            or not names or names != sorted(set(names))
            or not all(re.fullmatch(r"[A-Za-z0-9_+/-]{1,64}", name) for name in names)):
        raise ValueError("invalid View time zone catalogue")
    return template.replace(marker, json.dumps(names, separators=(",", ":")))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--module", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--candidate", default="01")
    parser.add_argument("--module-source", help="Existing verified revision used to build identical WASM")
    args = parser.parse_args()
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package=ROOT/"release/plugin-candidates"/f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt=package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    html = view_html()
    module=args.module.read_bytes()
    proof=json.loads(args.evidence.read_text(encoding="utf-8"))
    fixed=list(fixtures())
    if (not 0<len(module)<=2*1024*1024 or proof["wasmSha256"]!=digest(module) or proof["moduleBytes"]!=len(module)
            or proof["ianaVersion"]!=IANA or proof["workerProfile"]!="compute.wasi.v1" or proof["failures"]
            or proof["passed"]!=len(fixed)):
        parser.error("module or Windows Worker proof does not match the candidate")
    for row,expected in zip(proof["cases"],fixed,strict=True):
        data=compact(row["result"])
        check(row["result"],expected["expected"])
        if (row["case"]!=expected["name"] or row["requestSha256"]!=digest(compact(request(expected)))
                or row["resultBytes"]!=len(data) or row["resultSha256"]!=digest(data) or len(data)>MAX_RESULT):
            parser.error("fixture proof differs from its result or identity")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()
    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    source = git("rev-parse", "HEAD")
    module_source = source
    if args.module_source:
        module_source = git("rev-parse", "--verify", f"{args.module_source}^{{commit}}")
        if git("diff", "--name-only", module_source, source, "--",
               "crates/zi-time-zone-core/src", "crates/zi-time-zone-core/Cargo.toml", "crates/zi-time-zone-wasi"):
            parser.error("the time zone runtime sources differ from the reused module revision")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    (package/VIEW_PATH).parent.mkdir()
    (package/VIEW_PATH).write_text(html, encoding="utf-8", newline="\n")
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/time-zone/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Dependencies, including chrono-tz, retain their own licenses. IANA rule data 2025b.\n",encoding="utf8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title="IANA 时间换算",status="candidate",
                    scope=SCOPE,tzdbVersion=IANA),
               dict(sourceToolId=SOURCE,operationId=OPERATION,title=TITLE,kind="view-operation",
                    status="candidate",scope="搜索时区、按顺序换算并逐行复制。",tzdbVersion=IANA)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_time_zone_plugin.py",cases=proof["cases"]))
    dump(package/"provenance.json",dict(sourceRevision=source,wasmSourceRevision=module_source,dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        moduleSha256=digest(module),ianaVersion=IANA,workerVerification=proof))
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
