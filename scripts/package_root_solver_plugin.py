"""Freeze the verified offline Studio bracketed root solver candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_root_solver_plugin import CAPABILITY, CONTRACT, MAX_RESULT, PLUGIN, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "advanced-calculator"
SLUG = "root-solver"
VERSION = "0.1.0"
TITLE = "非线性方程求根"
SCOPE = "用区间二分法估算 f(x)=0 的一个候选点，显示收敛状态、残差和迭代次数。"


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    iterations = {"type":"integer","minimum":1,"maximum":200,"default":100,
                  "title":"迭代上限","description":"允许 1–200 次，默认 100 次。"}
    endpoint = {"type":"number","minimum":-1_000_000_000_000,"maximum":1_000_000_000_000}
    tolerance = {"type":"number","minimum":1e-12,"maximum":1e-2,"default":1e-10,
                 "title":"区间宽度容差","description":"允许 1e-12 到 1e-2，默认 1e-10。"}
    angle = {"type":"string","enum":["radians","degrees"],"default":"radians","title":"角度单位",
             "description":"三角函数默认用弧度 radians。选 degrees 改用角度。"}
    formula = dict(string(2048),minLength=1,title="函数 f(x)",
                   description="输入例如 x^2-2。函数在所给区间内需连续，程序无法判断连续性。")
    constants = {"type":"object","maxProperties":64,
                 "additionalProperties":True,
                 "title":"常量（可选）",
                 "description":'JSON 对象如 {"scale":"3/2","shift":0.75}。字符串为精确数，JSON 数字为有限近似数，不能覆盖 x 或内置名称。'}
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),error=nil,
                  data=obj(converged=boolean,iterations={"type":"integer","minimum":0,"maximum":200},
                           root=string(64),residual=string(64),text=string(256)))
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))
    inputs = dict(obj(formula=formula,lower=dict(endpoint,title="左端点"),upper=dict(endpoint,title="右端点"),
                      tolerance=tolerance,maxIterations=iterations,angle=angle,constants=constants),
                  required=["formula","lower","upper"])
    capability = dict(id=CAPABILITY,title=TITLE,description=SCOPE,inputSchema=inputs,
                      outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
                      directory=dict(categoryId="data",aliases=["区间二分","非线性方程","区间求根","Bisection","Nonlinear root"],description=SCOPE))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Bisection Root Solver",displayName=TITLE,
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
    shutil.copyfile(ROOT/"plugins/root-solver/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=CAPABILITY,title=TITLE,status="candidate",scope=SCOPE)]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_root_solver_plugin.py",cases=[
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
