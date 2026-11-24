"""Freeze a production Worker-verified, unsigned independent merge package."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_table_merge_plugin import FIELD_BYTES, FORMATS, IPC_BYTES, MODES, PLUGIN, PREFIX, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.1.0"
CONTRACT = "1.0.0-rc.1"
TITLE = "两表合并（独立格式）"
SCOPE = "左右分别使用 CSV、TSV 或 JSON。每张表最多 8 KiB，关联按值和类型匹配。"
TITLES = dict(left="左连接", inner="内连接", append="按同名列追加", columns="按行号拼列")


def manifest():
    version = {"type": "string", "const": CONTRACT}
    absent = {"type": "null"}
    output = {"type": "object", "oneOf": [
        obj(contractVersion=version, ok={"type": "boolean", "const": True}, data=obj(text=string(IPC_BYTES)), error=absent),
        obj(contractVersion=version, ok={"type": "boolean", "const": False}, data=absent, error=obj(code=string(32), message=string(IPC_BYTES))),
    ]}
    capabilities = []
    for mode in MODES:
        inputs = obj(
            left=dict(string(FIELD_BYTES), minLength=1, title="Left table"),
            leftFormat={"type":"string","enum":list(FORMATS),"title":"Left table format"},
            right=dict(string(FIELD_BYTES), minLength=1, title="Right table"),
            rightFormat={"type":"string","enum":list(FORMATS),"title":"Right table format"},
        )
        if mode in ("left", "inner"):
            inputs["properties"].update(leftKey=dict(string(FIELD_BYTES), title="Left join column"),
                                        rightKey=dict(string(FIELD_BYTES), title="Right join column"))
        capabilities.append(dict(
            id=PREFIX+mode, title=TITLES[mode]+"（独立格式）", description=SCOPE,
            inputSchema=inputs, outputSchema=output, maxResultBytes=IPC_BYTES,
            directory=dict(categoryId="data", aliases=["两表合并","混合格式",mode+" mixed table merge"], description=SCOPE),
        ))
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Independent Table Merge", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"output.schema.v1":"=1.0.0-rc.1", "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+cap["id"] for cap in capabilities],
        runtime={"wasi":"runtime/tools.wasm"},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=capabilities),
        permissions=dict(required=[], optional=[]),
    )


def main():
    parser, args = candidate_arguments(__doc__)
    if len(args.candidate)!=2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package = ROOT / "release/plugin-candidates" / f"table-merge-{VERSION}-candidate-{args.candidate}"
    receipt = package.with_name(package.name+"-verification.json")
    if package.exists() or receipt.exists():
        parser.error("candidate directory and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(fixtures())
    expected = [(name, PREFIX+mode, digest(compact(data))) for name,mode,data,_ in vectors]
    actual = [(row["case"],row["operation"],row["inputSha256"]) for row in proof["cases"]]
    if len(module)>2*1024*1024 or digest(module)!=proof["wasmSha256"] or actual!=expected:
        parser.error("verification module and vectors differ")
    revision = subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip()
    if subprocess.check_output(["git","status","--porcelain","--untracked-files=no"],cwd=ROOT).strip():
        parser.error("commit source before freezing")
    (package/"runtime").mkdir(parents=True)
    (package/"runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT/"LICENSE", package/"LICENSE")
    shutil.copyfile(ROOT/"plugins/table-merge/README.md", package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. serde, serde_json, csv and other Rust dependencies retain their upstream licenses.\n", encoding="utf-8")
    plugin = manifest()
    dump(package/"plugin.json",plugin)
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/"+CONTRACT,packageId=PLUGIN,packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="csv-merge",capabilityId=cap["id"],title=cap["title"],status="candidate",scope=SCOPE)
               for cap in plugin["contributes"]["capabilities"]]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_table_merge_plugin.py",cases=[
        dict(id=name,operation=PREFIX+mode,inputSha256=digest(compact(inputs)),input=inputs if len(compact(inputs))<4096 else None,expectedSuccess=isinstance(result,dict))
        for name,mode,inputs,result in vectors]))
    dump(package/"provenance.json",dict(sourceRevision=revision,dirty=False,wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        builtAt=datetime.now(timezone.utc).isoformat(),moduleSha256=digest(module),workerVerification=proof))
    files = [dict(path=path.relative_to(package).as_posix(),size=path.stat().st_size,sha256=digest(path.read_bytes()))
             for path in sorted(package.rglob("*")) if path.is_file()]
    index = digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
            objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package),directoryIndexSha256=index,files=len(files))))


if __name__=="__main__":
    main()
