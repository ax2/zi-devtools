"""Freeze the verified static table pipeline and its external receipt."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_table_pipeline_plugin import CAPABILITY, CSV, INPUT, OCTET, OUTPUT, PLAIN, PLUGIN, TITLE, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = "table-pipeline"
VERSION = "0.1.0"
SCOPE = "选择 JSON 管线配方和 CSV、TSV 或 JSON 表格，按文件列表的顺序指定配方位置。确认共享并运行，再确认保存带公式保护的新 CSV。"


def manifest():
    error = obj(code=string(40), message=string(256), step={"type":"integer","minimum":1,"maximum":32})
    error["required"] = ["code", "message"]
    result = obj(text=string(512), steps={"type":"integer","minimum":1,"maximum":32},rowsBefore=integer(96),
                 rowsAfter=integer(96), columns={"type":"integer","minimum":1,"maximum":16},csvBytes=integer(32768),
                 formulaProtection={"type":"boolean","const":True})
    ok = obj(ok={"type":"boolean","const":True},data=result,error={"type":"null"})
    reject = obj(ok={"type":"boolean","const":False},data={"type":"null"},error=error)
    settings = obj(format={"type":"string","enum":["csv","tsv","json"],"title":"数据文件的表格格式",
                           "description":"选择 CSV、TSV 或 JSON。留空使用 CSV。"},
                   recipePosition={"type":"integer","minimum":1,"maximum":2,"title":"配方在文件列表的位置",
                                   "description":"填写 1 或 2。留空使用列表第一份。"})
    settings["required"] = []
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Confirmed Table Pipeline",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1", "resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMax=2,resourceInputMimeTypes=[OCTET,PLAIN,CSV],
            inputSchema=settings,outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["可组合数据管线","表格流水线","Table Pipeline"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取本次明确确认的两份文件，按填写的位置区分配方和表格。")],optional=[]))


def main():
    parser, args = candidate_arguments(__doc__)
    if len(args.candidate) != 2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package = ROOT / "release/plugin-candidates" / f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt = package.with_name(package.name + "-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf8"))
    if (not 0 < len(module) <= 2 * 1024 * 1024 or proof["wasmSha256"] != digest(module) or proof["wasmBytes"] != len(module)
            or proof["inputProfile"] != INPUT or proof["outputProfile"] != OUTPUT or proof["failures"]):
        parser.error("module or production Windows Worker proof does not match the candidate")
    for row, case in zip(proof["cases"], fixtures(), strict=True):
        source = [dict(mime=mime, bytes=len(raw), sha256=digest(raw)) for mime, raw in case["sources"]]
        if row["case"] != case["name"] or row["sources"] != source or row["options"] != case["options"]:
            parser.error("verification case does not match its fixture")
        if isinstance(case["expected"], str):
            if (not row["error"] or row["error"]["code"] != case["expected"] or row["output"] is not None
                    or row.get("hostError") != case.get("host_error")):
                parser.error("unverified rejection in receipt")
        elif (row["error"] is not None or not row["output"] or row["output"]["mime"] != CSV
              or not 0 < row["output"]["bytes"] <= 32768 or len(row["output"]["sha256"]) != 64):
            parser.error("unverified protected CSV in receipt")

    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()

    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")
    (package / "runtime").mkdir(parents=True)
    (package / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
    shutil.copyfile(ROOT / "plugins/table-pipeline/README.md", package / "README.md")
    (package / "NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their own licenses.\n", encoding="utf8")
    dump(package / "plugin.json", manifest())
    dump(package / "catalog.json", dict(contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN,
        packageVersion=VERSION, deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="pipeline", capabilityId=CAPABILITY, title=TITLE, status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package / "fixtures.json", dict(generator="scripts/verify_table_pipeline_plugin.py", cases=proof["cases"]))
    dump(package / "provenance.json", dict(sourceRevision=git("rev-parse", "HEAD"), dirty=False, signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        moduleSha256=digest(module),workerVerification=proof))
    files = [dict(path=item.relative_to(package).as_posix(), size=item.stat().st_size, sha256=digest(item.read_bytes()))
             for item in sorted(package.rglob("*")) if item.is_file()]
    index = digest(compact(files))
    dump(receipt, dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
