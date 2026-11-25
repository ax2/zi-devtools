"""Freeze a verified unsigned local MCP config preview plugin."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_mcp_client_export_plugin import CONTRACT, JSON, MAX_PATH, MAX_RESULT, PLUGIN, TOML, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "mcp-export"
SLUG = "mcp-client-export"
VERSION = "0.1.0"
TITLE = "MCP 客户端配置预览"
TOML_TITLE = "MCP 配置 · Codex TOML"
JSON_TITLE = "MCP 配置 · 通用 JSON"
TOML_SCOPE = "预览 Codex TOML，仅启用两个知识工具并保留写入审批。可确认保存为 Studio 产物。"
JSON_SCOPE = "预览通用 mcpServers JSON 的 stdio 字段。可确认保存为 Studio 产物。"


def manifest():
    flag = {"type": "boolean"}
    contract = {"type": "string", "const": CONTRACT}
    nil = {"type": "null"}
    path = dict(string(MAX_PATH), minLength=1, title="MCP 服务程序绝对路径",
                description=r"例如 C:\ZiDevTools\ZiDevToolsMcp.exe。请输入已知且信任的本机路径。")
    failure = obj(contractVersion=contract, ok=dict(flag, const=False), data=nil,
                  error=obj(code=string(40), message=string(256)))

    def capability(identity, title, description, kind, aliases):
        result = obj(format={"type":"string", "const":kind}, text=string(4096))
        success = obj(contractVersion=contract, ok=dict(flag, const=True), data=result, error=nil)
        return dict(
            id=identity, title=title, description=description, inputSchema=obj(executablePath=path),
            outputSchema={"type":"object", "oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="text",aliases=aliases,description=description),
        )

    codex = capability(TOML, TOML_TITLE, TOML_SCOPE, "toml", ["MCP Codex 配置", "Codex MCP config"])
    generic = capability(JSON, JSON_TITLE, JSON_SCOPE, "json", ["MCP 通用配置", "Generic MCP configuration"])
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools MCP Client Config Preview",displayName=TITLE,
        description="手动输入已知 MCP 程序路径，预览与复制客户端配置。",version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+TOML,"onCapability:"+JSON],
        runtime={"wasi":"runtime/tools.wasm"},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[codex,generic]),
        permissions=dict(required=[],optional=[]),
    )



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
    expected = [(name,capability,digest(compact(inputs)),value in ("toml","json"))
                for name,capability,inputs,value in fixtures()]
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
    shutil.copyfile(ROOT/"plugins/mcp-client-export/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=identity,title=title,status="candidate",scope=scope)
                                             for identity,title,scope in [
                                                 (TOML,TOML_TITLE,TOML_SCOPE),
                                                 (JSON,JSON_TITLE,JSON_SCOPE),
                                             ]]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_mcp_client_export_plugin.py",cases=[
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
