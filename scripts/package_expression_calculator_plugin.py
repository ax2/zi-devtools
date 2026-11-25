"""Freeze the verified offline Studio expression calculator candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_expression_calculator_plugin import CAPABILITY, CONTRACT, MAX_RESULT, PLUGIN, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "advanced-calculator"
SLUG = "expression-calculator"
VERSION = "0.1.0"
TITLE = "公式计算器"
SCOPE = "离线计算单个公式，支持精确分数、科学和复数运算、位运算、单位换算及排列组合。"


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    angle = {"type":"string","enum":["radians","degrees"],"default":"radians","title":"角度单位",
             "description":"三角函数默认用弧度 radians。选 degrees 改用角度。"}
    formula = dict(string(2048),minLength=1,title="公式",
                   description="输入一个公式，例如 1/3+1/6、comb(52,5) 或 5 km -> m。")
    constants = {"type":"object","maxProperties":64,"additionalProperties":True,
                 "title":"常量（可选）",
                 "description":'JSON 对象如 {"scale":"3/2","offset":0.75}。字符串为精确数，JSON 数字为有限近似数。'}
    exact = obj(kind={"type":"string","const":"exact"}, numerator=string(40), denominator=string(39))
    approximate = obj(kind={"type":"string","const":"approx"}, value=string(64))
    complex_value = obj(kind={"type":"string","const":"complex"}, real=string(64), imaginary=string(64))
    value = {"type":"object","oneOf":[exact,approximate,complex_value]}
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),
                  data=obj(value=value,approximate=boolean,text=string(256)),error=nil)
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(MAX_RESULT)))
    inputs = dict(obj(formula=formula,angle=angle,constants=constants),required=["formula"])
    capability = dict(id=CAPABILITY,title=TITLE,description=SCOPE,inputSchema=inputs,
                      outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
                      directory=dict(categoryId="data",aliases=["表达式计算","科学计算","排列组合","阶乘","位运算",
                                                                  "单位换算","Expression calculator","Combinations"],
                                     description=SCOPE))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Expression Calculator",displayName=TITLE,
                description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
                engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
                requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1",
                                  "output.schema.v1":"=1.0.0-rc.1",
                                  "tool.directory.v1":"=1.0.0-preview.1"},
                activationEvents=["onCapability:"+CAPABILITY],runtime={"wasi":"runtime/tools.wasm"},
                contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[capability]),
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
    expected = [(name,CAPABILITY,digest(compact(inputs)),not isinstance(value,str))
                for name,inputs,value in fixtures()]
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
    shutil.copyfile(ROOT/"plugins/expression-calculator/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_expression_calculator_plugin.py",cases=[
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
