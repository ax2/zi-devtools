"""Freeze the verified offline civil dates plugin."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_date_calculator_plugin import CONTRACT, MAX_RESULT, PLUGIN, PREFIX, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "advanced-calculator"
SLUG = "date-calculator"
VERSION = "0.1.0"
TITLE = "日期与工作日"
SCOPE = "离线计算公历日期差、日期偏移、工作日计数与偏移。工作日按周一至周五计算。"
DESCRIPTORS = {
    "difference": ("日期差", "计算结束日期减去开始日期的带符号天数。", ["相差天数","Date difference"]),
    "offset": ("日期加减", "按天、月或年偏移公历日期。可显式选择截到目标月末。", ["月末日期","Date offset"]),
    "workday_count": ("工作日计数", "按周一至周五计算天数，结束日包含在内。可选择包含开始日。", ["工作日天数","Count weekdays"]),
    "workday_offset": ("工作日加减", "按周一至周五正向或反向偏移。输入 0 保持起始日。", ["工作日偏移","Shift weekdays"]),
}


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    days = {"type":"integer","minimum":-4000000,"maximum":4000000}
    day = obj(date=string(10),weekday=string(12),text=string(80))
    only = {"type":"boolean","const":True}
    output = {
        "difference": obj(days=days,text=string(80)),
        "offset": day,
        "workday_count": obj(days=days,text=string(80),weekdaysOnly=only),
        "workday_offset": obj(**day["properties"],weekdaysOnly=only),
    }
    start = dict(string(10),minLength=10,title="开始日期",description="格式 YYYY-MM-DD，公历 0001–9999 年。")
    end = dict(string(10),minLength=10,title="结束日期",description="格式 YYYY-MM-DD。")
    span = obj(start=start,end=end)
    amount = dict(days,title="偏移量",description="输入整数，负数表示向前，正数表示向后。")
    unit = {"type":"string","enum":["days","months","years"],"title":"单位","description":"days 天，months 月，years 年。"}
    options = {
        "difference":span,
        "offset":dict(obj(start=start,amount=amount,unit=unit,clamp=dict(boolean,title="截到月末")),
                       required=["start","amount","unit"]),
        "workday_count":dict(obj(start=start,end=end,includeStart=dict(boolean,title="包含开始日")),
                              required=["start","end"]),
        "workday_offset":obj(start=start,amount=amount),
    }
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    capabilities = []
    for operation,(title,scope,aliases) in DESCRIPTORS.items():
        success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=output[operation],error=nil)
        capabilities.append(dict(id=PREFIX+operation,title=title,description=scope,
                                 inputSchema=options[operation],
                                 outputSchema={"type":"object","oneOf":[success,failure]},
                                 maxResultBytes=MAX_RESULT,
                                 directory=dict(categoryId="data",aliases=aliases,description=scope)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Civil Date Calculator",displayName=TITLE,
                description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
                engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
                requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1",
                                  "output.schema.v1":"=1.0.0-rc.1",
                                  "tool.directory.v1":"=1.0.0-preview.1"},
                activationEvents=["onCapability:"+capability["id"] for capability in capabilities],
                runtime={"wasi":"runtime/tools.wasm"},
                contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=capabilities),
                permissions=dict(required=[],optional=[]))


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
    expected=[(name,PREFIX+op,digest(compact(inputs)),not isinstance(value,str))
              for name,op,inputs,value in vectors]
    actual=[(row["case"],row["capabilityId"],row["inputSha256"],row["ok"])
            for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"]!=digest(module)
            or proof["workerProfile"]!="compute.wasi.v1" or proof["passed"]!=len(vectors) or actual!=expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/date-calculator/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.\n",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=PREFIX+op,title=names[0],status="candidate",
                                                  scope=names[1]) for op,names in DESCRIPTORS.items()]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_date_calculator_plugin.py",cases=[
        dict(id=name,capabilityId=PREFIX+op,inputSha256=digest(compact(inputs)),ok=not isinstance(value,str))
        for name,op,inputs,value in vectors
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
