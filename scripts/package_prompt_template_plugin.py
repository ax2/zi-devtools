"""Freeze the verified offline Studio prompt tools candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_prompt_template_plugin import (
    CONTRACT, EXPAND, INSPECT, MAX_EXPANSION, MAX_PREVIEW, MAX_RESULT, MAX_TEMPLATE, PLUGIN, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "prompt-library"
SLUG = "prompt-expansion"
VERSION = "0.1.0"
TITLE = "提示词模板"
INSPECT_TITLE = "提示词变量检查"
EXPAND_TITLE = "提示词模板展开"
INSPECT_SCOPE = "检查本机模板正文中最多 12 个不同变量，并识别四花括号字面。模板最多 16 KiB。"
EXPAND_SCOPE = "填写文本变量并离线展开一次。完整展开最多 128 KiB，工具中心显示最多 16 KiB 的有界预览。"


def manifest():
    boolean = {"type":"boolean"}
    contract = {"type":"string","const":CONTRACT}
    nil = {"type":"null"}
    template = dict(string(MAX_TEMPLATE), minLength=1, title="模板正文",
                    description="使用 {{project}}。变量名可用英文字母、数字和下划线。{{{{ 与 }}}} 为字面双花括号。")
    bindings = {"type":"object","maxProperties":12,"additionalProperties":True,
                "title":"变量值（可选）",
                "description":'填写仅含字符串值的 JSON 对象，例如 {"project":"桌面工具"}。使用到的变量需要填写。'}
    names = {"type":"array","maxItems":12,"items":string(32)}
    failure = obj(contractVersion=contract,ok=dict(boolean,const=False),data=nil,
                  error=obj(code=string(32),message=string(240)))

    def capability(identity, title, summary, inputs, result, aliases):
        success = obj(contractVersion=contract,ok=dict(boolean,const=True),data=result,error=nil)
        return dict(
            id=identity,title=title,description=summary,inputSchema=inputs,
            outputSchema={"type":"object","oneOf":[success,failure]},maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="text",aliases=aliases,description=summary),
        )

    inspect = capability(
        INSPECT, INSPECT_TITLE, INSPECT_SCOPE, obj(template=template),
        obj(variables=names,templateByteLength=integer(MAX_TEMPLATE),text=string(512)),
        ["检查提示词","列出提示词变量","Prompt variables"],
    )
    expand = capability(
        EXPAND, EXPAND_TITLE, EXPAND_SCOPE, dict(obj(template=template,values=bindings),required=["template"]),
        obj(variables=names,expandedByteLength=integer(MAX_EXPANSION),
            previewByteLength=integer(MAX_PREVIEW),truncated=boolean,text=string(MAX_PREVIEW+128)),
        ["展开提示词","提示词预览","提示词占位符","Expand prompt template"],
    )
    return dict(
        schemaVersion=2,id=PLUGIN,name="ZiDevTools Prompt Template Tools",displayName=TITLE,
        description=EXPAND_SCOPE,version=VERSION,publisher="zicode",license="MIT",
        engines={"zicode":">=0.7.2 <1.0.0","pluginApi":"^1.0.0"},
        requiredProfiles={"compute.wasi.v1":"=1.0.0-rc.1",
                          "output.schema.v1":"=1.0.0-rc.1",
                          "tool.directory.v1":"=1.0.0-preview.1"},
        activationEvents=["onCapability:"+INSPECT,"onCapability:"+EXPAND],
        runtime={"wasi":"runtime/tools.wasm"},
        contributes=dict(activities=[],views=[],commands=[],piTools=[],capabilities=[inspect,expand]),
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
    expected = [(name,capability,digest(compact(inputs)),not isinstance(value,str))
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
    shutil.copyfile(ROOT/"plugins/prompt-expansion/README.md",package/"README.md")
    (package/"NOTICE").write_text("ZiDevTools is MIT. Serde and other dependencies retain their licenses.",encoding="utf-8")
    dump(package/"plugin.json",manifest())
    dump(package/"catalog.json",dict(contract="zicode.devtools-plugin/1.0.0-rc.1",packageId=PLUGIN,
                                      packageVersion=VERSION,deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SOURCE,capabilityId=identity,title=title,status="candidate",scope=scope)
                                             for identity,title,scope in [
                                                 (INSPECT,INSPECT_TITLE,INSPECT_SCOPE),
                                                 (EXPAND,EXPAND_TITLE,EXPAND_SCOPE),
                                             ]]))
    dump(package/"fixtures.json",dict(generator="scripts/verify_prompt_template_plugin.py",cases=[
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
