"""Freeze the verified offline conversation Markdown exporter and its external receipt."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_conversation_markdown_plugin import CAPABILITY, FILE_BYTES, INPUT, MARKDOWN, OCTET, OUTPUT, PLUGIN, TEXT, TITLE, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = "conversation-markdown"
VERSION = "0.1.0"
SCOPE = "确认共享一份 ZiDevTools 会话 JSON，将完整问答按顺序转成使用纯文本围栏的 Markdown，再确认保存新文件。"


def manifest():
    result = obj(text=string(512), turns={"type":"integer","minimum":1,"maximum":16},
                 sourceBytes=integer(FILE_BYTES), sourceSha256={"type":"string","minLength":64,"maxLength":64},
                 markdownBytes=integer(192*1024))
    ok = obj(ok={"type":"boolean","const":True},data=result,error={"type":"null"})
    reject = obj(ok={"type":"boolean","const":False},data={"type":"null"},error=obj(code=string(40),message=string(256)))
    return dict(schemaVersion=2,id=PLUGIN,name="ZiDevTools Offline Conversation Markdown",displayName=TITLE,
        description=SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"resource.input.types.v1":"=1.0.0-preview.1", "resource.output.types.v1":"=1.0.0-preview.1",
                          "tool.directory.v1":"=1.0.0-preview.1","output.schema.v1":"=1.0.0-rc.1"},
        activationEvents=["onCapability:"+CAPABILITY],
        runtime={"wasi":"runtime/tools.wasm","resourceInputProfile":INPUT,"resourceOutputProfile":OUTPUT},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[dict(
            id=CAPABILITY,title=TITLE,description=SCOPE,resourceInputMimeTypes=[OCTET,TEXT],
            inputSchema=obj(),outputSchema={"type":"object","oneOf":[ok,reject]},maxResultBytes=8192,
            directory=dict(categoryId="files",aliases=["离线会话导出","对话转 Markdown","Conversation to Markdown"],description=SCOPE))]),
        permissions=dict(required=[dict(name="resource.read",reason="只读取为当前转换明确共享的一份会话文件。")],optional=[]))


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
        if (row["case"] != case["name"] or row["inputBytes"] != len(case["raw"]) or row["sourceSha256"] != digest(case["raw"])
                or row["mime"] != case["mime"] or row["options"] != case["options"]):
            parser.error("verification case does not match its fixture")
        if code := case.get("error"):
            if (not row["error"] or row["error"]["code"] != code or row["output"] is not None
                    or row.get("hostError") != case.get("host_error")):
                parser.error("unverified rejection in receipt")
        elif (row["error"] is not None or not row["output"] or row["output"]["mime"] != MARKDOWN
              or not 0 < row["output"]["bytes"] <= 192*1024 or len(row["output"]["sha256"]) != 64):
            parser.error("unverified Markdown in receipt")

    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()

    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")
    (package / "runtime").mkdir(parents=True)
    (package / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
    shutil.copyfile(ROOT / "plugins/conversation-markdown/README.md", package / "README.md")
    (package / "NOTICE").write_text("ZiDevTools is MIT. Dependencies retain their own licenses.\n", encoding="utf8")
    dump(package / "plugin.json", manifest())
    dump(package / "catalog.json", dict(contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN,
        packageVersion=VERSION, deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="model-chat", capabilityId=CAPABILITY, title=TITLE, status="candidate",inputProfile=INPUT,scope=SCOPE)]))
    dump(package / "fixtures.json", dict(generator="scripts/verify_conversation_markdown_plugin.py", cases=proof["cases"]))
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
