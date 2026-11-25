"""Freeze the verified offline Studio matrix calculator candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_matrix_calculator_plugin import BINARY, CONTRACT, MAX_RESULT, PLUGIN, PREFIX, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "advanced-calculator"
SLUG = "matrix-calculator"
VERSION = "0.1.0"
TITLE = "矩阵计算"
SCOPE = "离线计算实数矩阵加减、乘法、转置、行列式、逆矩阵和矩阵方程 AX=B。"
DESCRIPTORS = {
    "add": ("矩阵加法", "对同样尺寸的矩阵 A 和 B 逐项相加。", ["矩阵相加", "Matrix addition"]),
    "subtract": ("矩阵减法", "从矩阵 A 中逐项减去同样尺寸的矩阵 B。", ["矩阵相减", "Matrix subtraction"]),
    "multiply": ("矩阵乘法", "计算 A × B。A 的列数须与 B 的行数相同。", ["矩阵乘积", "Matrix multiplication"]),
    "transpose": ("矩阵转置", "将 A 的行与列互换。", ["转置矩阵", "Matrix transpose"]),
    "determinant": ("矩阵行列式", "计算方阵 A 的 det(A)。", ["行列式", "Matrix determinant"]),
    "inverse": ("逆矩阵", "计算可逆方阵 A 的逆。", ["矩阵求逆", "Matrix inverse"]),
    "solve": ("矩阵方程求解", "求解 AX=B。A 须为可逆方阵，B 可有多列右端项。", ["解线性方程组", "Solve AX=B", "Matrix equation solver"]),
}


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    count = {"type":"integer","minimum":1,"maximum":8}
    exact = obj(kind={"type":"string","const":"exact"}, numerator=string(40), denominator=string(39))
    approximate = obj(kind={"type":"string","const":"approx"}, value=string(64))
    cell = {"type":"object","oneOf":[exact,approximate]}
    row = {"type":"array","items":cell,"minItems":1,"maxItems":8}
    values = {"type":"array","items":row,"minItems":1,"maxItems":8}
    success = obj(contractVersion=contract,ok=dict(boolean,const=True),error=nil,
                  data=obj(rowCount=count,columnCount=count,approximate=boolean,values=values,text=string(MAX_RESULT)))
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(256)))

    def matrix(name):
        return dict(string(4096),minLength=1,title=f"矩阵 {name}",
                    description="粘贴 1–8 行、每行 1–8 列。列用 Tab 分隔，行用换行分隔，可从电子表格直接复制。每个矩阵最多 4096 UTF-8 字节。")

    precision = {"type":"string","enum":["exact","approximate"],"default":"exact",
                 "title":"计算精度","description":"exact 使用精确有理数，支持分数如 1/3。approximate 使用有限小数或科学计数法。"}
    capabilities = []
    for operation,(title,scope,aliases) in DESCRIPTORS.items():
        matrices = {"a":matrix("A")}
        if operation in BINARY:
            matrices["b"] = matrix("B")
        inputs = dict(obj(**matrices,precision=precision),required=list(matrices))
        capabilities.append(dict(id=PREFIX+operation,title=title,description=scope,
                                 inputSchema=inputs,outputSchema={"type":"object","oneOf":[success,failure]},
                                 maxResultBytes=MAX_RESULT,
                                 directory=dict(categoryId="data",aliases=aliases,description=scope)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Matrix Calculator",displayName=TITLE,
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
    shutil.copyfile(ROOT/"plugins/matrix-calculator/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=PREFIX+op,title=names[0],status="candidate",
                                                  scope=names[1]) for op,names in DESCRIPTORS.items()]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_matrix_calculator_plugin.py",cases=[
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
