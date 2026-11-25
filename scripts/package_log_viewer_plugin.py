"""Freeze a production-Worker-verified, unsigned log viewer package."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, integer, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_log_viewer_plugin import (
    CAPABILITY, MAX_FILE, MAX_MATCHES, MAX_PREVIEW, MAX_QUERY, MAX_RESULT,
    PLUGIN, PROFILE, compact, digest, fixtures,
)

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "log-viewer"
VERSION = "0.1.0"
TITLE = "本机日志筛选"
SCOPE = "查看一份已确认的 UTF-8 日志。按级别和区分大小写的字面搜索词筛选多行事件。文件限 16 KiB，显示前 12 条匹配。"
MIME_TYPES = ["text/plain", "application/octet-stream"]
LEVEL = {"type": "string", "enum": ["fatal", "error", "warning", "info", "debug", "trace", "unknown"]}


def manifest():
    fields = obj(
        level=dict(LEVEL, title="日志级别 / Log level"),
        query=dict(string(MAX_QUERY), title="搜索词 / Search phrase"),
    )
    fields["required"] = []
    boolean = {"type": "boolean"}
    entry = obj(eventNumber=integer(MAX_FILE), level=LEVEL, lineCount=integer(MAX_FILE),
                text=string(MAX_PREVIEW), previewTruncated=boolean)
    success = obj(ok=dict(boolean, const=True), sourceByteCount=integer(MAX_FILE),
                  eventCount=integer(MAX_FILE), matchCount=integer(MAX_FILE), truncated=boolean,
                  matches={"type": "array", "items": entry, "maxItems": MAX_MATCHES})
    failure = obj(ok=dict(boolean, const=False), error=obj(code=string(32), message=string(256)))
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Local Log Filter", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"resource.input.types.v1": "=1.0.0-preview.1",
                          "output.schema.v1": "=1.0.0-rc.1",
                          "tool.directory.v1": "=1.0.0-preview.1"},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime={"wasi": "runtime/tools.wasm", "resourceInputProfile": PROFILE},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=SCOPE,
            resourceInputMimeTypes=MIME_TYPES, inputSchema=fields,
            outputSchema={"type": "object", "oneOf": [success, failure]}, maxResultBytes=MAX_RESULT,
            directory=dict(categoryId="files", aliases=["日志工作台", "日志查看", "log viewer", "filter log"],
                           description=SCOPE),
        )]),
        permissions=dict(required=[
            dict(name="resource.read", reason="只读取本次明确确认的临时日志文件快照。"),
        ], optional=[]),
    )


def main():
    parser, args = candidate_arguments(__doc__)
    if not (len(args.candidate) == 2 and args.candidate.isascii() and args.candidate.isdecimal()):
        parser.error("use a two-digit candidate number")
    package = ROOT / "release/plugin-candidates" / f"{SOURCE}-{VERSION}-candidate-{args.candidate}"
    receipt = package.with_name(package.name + "-verification.json")
    if package.exists() or receipt.exists():
        parser.error("package and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(fixtures())
    expected = [(name, digest(data), inputs, mime, not isinstance(result, str))
                for name, data, inputs, mime, result in vectors]
    actual = [(row["case"], row["inputSha256"], row["input"], row["mime"], row["ok"])
              for row in proof["cases"]]
    if (not 0 < len(module) <= 2 * 1024 * 1024 or proof["wasmSha256"] != digest(module)
            or proof["workerProfile"] != PROFILE or proof["passed"] != len(vectors) or actual != expected):
        parser.error("module or fixed conformance vectors differ")

    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()

    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")
    (package / "runtime").mkdir(parents=True)
    (package / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
    shutil.copyfile(ROOT / "plugins/log-viewer/README.md", package / "README.md")
    (package / "NOTICE").write_text("ZiDevTools MIT. Chrono, Serde and serde_json retain their upstream licenses.\n",
                                    encoding="utf-8")
    dump(package / "plugin.json", manifest())
    dump(package / "catalog.json", dict(contract="zicode.devtools-plugin/1.0.0-rc.1",
                                        packageId=PLUGIN, packageVersion=VERSION,
                                        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                        tools=[dict(sourceToolId=SOURCE, capabilityId=CAPABILITY, title=TITLE,
                                                    status="candidate", inputProfile=PROFILE, scope=SCOPE)]))
    dump(package / "fixtures.json", dict(generator="scripts/verify_log_viewer_plugin.py", cases=[
        dict(id=name, inputSha256=digest(data), inputBytes=len(data), input=inputs,
             mime=mime, ok=not isinstance(result, str)) for name, data, inputs, mime, result in vectors
    ]))
    dump(package / "provenance.json", dict(sourceRevision=git("rev-parse", "HEAD"), dirty=False,
                                          signature="unsigned", builtAt=datetime.now(timezone.utc).isoformat(),
                                          wasmTarget="wasm32-wasip1", wasmProfile="plugin",
                                          moduleSha256=digest(module), workerVerification=proof))
    files = [dict(path=path.relative_to(package).as_posix(), size=path.stat().st_size,
                  sha256=digest(path.read_bytes())) for path in sorted(package.rglob("*")) if path.is_file()]
    origin = digest(compact(files))
    dump(receipt, dict(schemaVersion=1, pluginId=PLUGIN, packageVersion=VERSION, signatureVerified=False,
                       directoryIndexSha256=origin, files=files, fileCount=len(files),
                       totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                                   objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                                   separators=[",", ":"], trailingNewline=False)))
    print(json.dumps(dict(candidate=str(package), directoryIndexSha256=origin, files=len(files))))


if __name__ == "__main__":
    main()
