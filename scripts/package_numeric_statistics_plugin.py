"""Freeze the verified offline Studio numeric statistics candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_numeric_statistics_plugin import CONTRACT, MAX_RESULT, PLUGIN, PREFIX, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "advanced-calculator"
SLUG = "numeric-statistics"
VERSION = "0.1.0"
TITLE = "数值统计"
SCOPE = "离线计算总和、均值、中位数、总体和样本的方差及标准差。"
DESCRIPTORS = {
    "sum": ("数值总和", "把输入的实数相加。", ["求和", "数列总和", "Numeric sum"]),
    "mean": ("算术均值", "计算输入实数的算术平均值。", ["平均数", "算术平均值", "Arithmetic mean"]),
    "median": ("中位数", "排序后取中间值。数据有偶数个时取中间两项的均值。", ["数值中位数", "Numeric median"]),
    "population_variance": ("总体方差", "将离均差平方和除以总体数据个数 n，最少一个数。", ["总体变异", "Population variance"]),
    "sample_variance": ("样本方差", "将离均差平方和除以 n−1，最少两个数。", ["样本变异", "Sample variance"]),
    "population_standard_deviation": ("总体标准差", "总体方差的近似平方根，最少一个数。", ["总体标准偏差", "Population standard deviation"]),
    "sample_standard_deviation": ("样本标准差", "样本方差的近似平方根，最少两个数。", ["样本标准偏差", "Sample standard deviation"]),
}


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    count = {"type":"integer","minimum":1,"maximum":64}
    exact = obj(kind={"type":"string","const":"exact"}, numerator=string(40), denominator=string(39))
    approximate = obj(kind={"type":"string","const":"approx"}, value=string(64))
    cell = {"type":"object","oneOf":[exact,approximate]}
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),error=nil,
                  data=obj(count=count,approximate=boolean,value=cell,text=string(96)))
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    data = dict(string(4096),minLength=1,title="数值",
                description="粘贴 1–64 个实数，用逗号、Tab 或换行分隔。最多 4096 UTF-8 字节，不跳过空值。")
    precision = {"type":"string","enum":["exact","approximate"],"default":"exact",
                 "title":"计算精度","description":"exact 使用精确有理数，支持分数如 1/3。approximate 使用有限小数或科学计数法。标准差始终标记为近似。"}
    inputs = dict(obj(values=data,precision=precision),required=["values"])
    capabilities = []
    for operation,(title,scope,aliases) in DESCRIPTORS.items():
        capabilities.append(dict(id=PREFIX+operation,title=title,description=scope,
                                 inputSchema=inputs,outputSchema={"type":"object","oneOf":[success,failure]},
                                 maxResultBytes=MAX_RESULT,
                                 directory=dict(categoryId="data",aliases=aliases,description=scope)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Numeric Statistics",displayName=TITLE,
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
    parser,args = candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package = ROOT/"release/plugin-candidates"/f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt = package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    expected = [(name,PREFIX+op,digest(compact(inputs)),not isinstance(value,str))
                for name,op,inputs,value in fixtures()]
    actual = [(row["case"],row["capabilityId"],row["inputSha256"],row["ok"]) for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"] != digest(module)
            or proof["workerProfile"] != "compute.wasi.v1" or proof["failures"]
            or proof["passed"] != len(expected) or actual != expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git",*args],cwd=ROOT,text=True).strip()

    if git("status","--porcelain","--untracked-files=no"):
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE",package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/numeric-statistics/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=PREFIX+op,title=names[0],status="candidate",
                                                  scope=names[1]) for op,names in DESCRIPTORS.items()]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_numeric_statistics_plugin.py",cases=[
        dict(id=name,capabilityId=capability,inputSha256=sha,ok=ok) for name,capability,sha,ok in expected
    ]))
    dump(package/"provenance.json",dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
                                         builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",
                                         wasmProfile="plugin",moduleSha256=digest(module),workerVerification=proof))
    files = [dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
             for path in sorted(package.rglob("*")) if path.is_file()]
    index = digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
                       directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
                                                   objectKeyOrder=["path","size","sha256"],ensureAscii=False,
                                                   separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__ == "__main__":
    main()
