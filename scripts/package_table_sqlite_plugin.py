"""Freeze the verified file-backed SQLite export tool as an unsigned directory."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_table_sqlite_plugin import CAPABILITY, INPUT, MAX_SOURCE, OCTET, OUTPUT, PLUGIN, TITLE, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "data-sqlite-export"
VERSION = "0.1.0"
SLUG = "table-sqlite"
SCOPE = "读取一份已确认的本机 UTF-8 CSV、TSV 或 JSON 表格，将全部行生成一个新的 SQLite 单表文件。"


def manifest():
    boolean = {"type": "boolean"}
    nil = {"type": "null"}
    contract = {"type": "string", "const": "1.0.0-rc.1"}
    data = obj(tableName=string(512), rows=integer(128), columns=integer(16),
               byteLength=integer(128 * 1024), text=string(256))
    inputs = obj(format={"type": "string", "enum": ["csv", "tsv", "json"], "title": "表格格式"},
                 tableName=dict(string(512), title="表名（可选）", default="data",
                                description="留空时为 data，最多 512 个 UTF-8 字节。"))
    inputs["required"] = ["format"]
    match_ok = obj(contractVersion=contract, ok=dict(boolean, const=True), data=data, error=nil)
    match_failure = obj(contractVersion=contract, ok=dict(boolean, const=False), data=nil,
                        error=obj(code=string(40), message=string(256)))
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Table SQLite Export", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={name: "=1.0.0-preview.1" for name in
                          ("resource.input.types.v1", "resource.output.types.v1", "tool.directory.v1")} |
                         {"output.schema.v1": "=1.0.0-rc.1"},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=SCOPE,
            resourceInputMimeTypes=[OCTET,"text/csv","application/json","text/plain"], inputSchema=inputs,
            outputSchema={"type": "object", "oneOf": [match_ok, match_failure]}, maxResultBytes=8192,
            directory=dict(categoryId="files", aliases=["CSV 转 SQLite","TSV 转 SQLite","JSON 转 SQLite","Table SQLite Export"],
                           description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read", reason="读取本次确认的一份表格快照。")], optional=[]),
    )


def main():
    parser, args = candidate_arguments(__doc__)
    if len(args.candidate) != 2 or not args.candidate.isascii() or not args.candidate.isdecimal():
        parser.error("use a two-digit candidate number")
    package = ROOT / "release/plugin-candidates" / f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt = package.with_name(package.name + "-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(fixtures())
    expected = [(name,digest(payload),mime,options,headers if isinstance(headers,str) else None)
                for name,payload,mime,options,headers,rows in vectors]
    actual = [(row["case"],row["sourceSha256"],row["mime"],row["options"],row["code"]) for row in proof["cases"]]
    if (not 0 < len(module) <= 2*1024*1024 or proof["wasmSha256"] != digest(module)
            or proof["profile"] != INPUT or proof["outputProfile"] != OUTPUT or proof["failures"] or actual != expected):
        parser.error("module and fixed Windows Worker results must match")

    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()

    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")
    (package / "runtime").mkdir(parents=True)
    (package / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
    shutil.copyfile(ROOT / "plugins/table-sqlite/README.md", package / "README.md")
    (package / "NOTICE").write_text("ZiDevTools is MIT. SQLite 3.46.0 is in the public domain. Other dependencies retain their licenses.\n",
                                   encoding="utf-8")
    dump(package / "plugin.json", manifest())
    dump(package / "catalog.json", dict(contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN,
        packageVersion=VERSION, deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId=SOURCE, capabilityId=CAPABILITY, title=TITLE, status="candidate", inputProfile=INPUT, scope=SCOPE)]))
    dump(package / "fixtures.json", dict(generator="scripts/verify_table_sqlite_plugin.py", cases=[
        dict(id=name, inputSha256=digest(payload), inputBytes=len(payload), input=options, mime=mime,
             error=headers if isinstance(headers,str) else None)
        for name,payload,mime,options,headers,rows in vectors]))
    dump(package / "provenance.json", dict(sourceRevision=git("rev-parse","HEAD"),dirty=False,signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(),wasmTarget="wasm32-wasip1",wasmProfile="plugin",
        sqlite="3.46.0",zig="0.15.2",moduleSha256=digest(module),workerVerification=proof))
    files = [dict(path=item.relative_to(package).as_posix(),size=item.stat().st_size,sha256=digest(item.read_bytes()))
             for item in sorted(package.rglob("*")) if item.is_file()]
    index = digest(compact(files))
    dump(receipt,dict(schemaVersion=1,pluginId=PLUGIN,packageVersion=VERSION,signatureVerified=False,
        directoryIndexSha256=index,files=files,fileCount=len(files),totalBytes=sum(row["size"] for row in files),
        directoryIndexEncoding=dict(hash="SHA-256",encoding="UTF-8",content="files array in listed order",
             objectKeyOrder=["path","size","sha256"],ensureAscii=False,separators=[",",":"],trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
