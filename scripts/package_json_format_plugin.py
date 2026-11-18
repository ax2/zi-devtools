"""Create a frozen, unsigned strict JSON-file transform candidate."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[1]
PLUGIN = "com.zicode.devtools.json-file"
PROFILE = "resource.wasi.json-file-transform.v1"
PREVIEW = "=1.0.0-preview.1"
VERSION = "0.1.0"
LIMIT = 1024 * 1024
OPERATIONS = [
    ("format", "json", "JSON 文件格式化", ["JSON 格式化", "JSON file format"]),
    ("minify", "json", "JSON 文件压缩", ["JSON 压缩", "JSON file minify"]),
    ("validate", "json", "JSON 文件校验", ["JSON 校验", "JSON file validate"]),
]

SCOPE = "每项输入最多 1 MiB 的 UTF-8 JSON，64层，拒绝重复键、不安全整数和不能精确往返的十进制数值；不把策略拒绝误报为语法错误。仅在确认文件并执行后读取，输出保留为可另存的 JSON，最多 8 MiB。"
LICENSES = "ZiDevTools adapter and shared zi-text-core are MIT. Base64, anyhow, serde and serde_json retain their upstream licenses."


def object_schema(properties):
    return {"type": "object", "properties": properties, "required": list(properties), "additionalProperties": False}


def capability(suffix, title, aliases):
    count = 1
    success = object_schema({
        "ok": {"type": "boolean", "const": True},
        "schemaVersion": {"type": "integer", "const": 1},
        "material": {"type": "string", "const": "json-file-result"},
        "sourceByteCounts": {"type": "array", "minItems": count, "maxItems": count,
                             "items": {"type": "integer", "minimum": 0, "maximum": LIMIT}},
        "outputByteCount": {"type": "integer", "minimum": 1, "maximum": 8 * LIMIT},
    })
    failure = object_schema({
        "ok": {"type": "boolean", "const": False},
        "error": object_schema({"code": {"type": "string", "maxLength": 40}, "message": {"type": "string", "maxLength": 512}}),
    })
    definition = dict(
        id="devtools.json-file." + suffix, title=title, executionProfile=PROFILE, maxResultBytes=2048,
        inputSchema=object_schema({}),
        outputSchema={"type": "object", "oneOf": [success, failure]},
        resourceInputMimeTypes=["application/octet-stream", "application/json", "text/plain"],
        directory=dict(categoryId="data", aliases=aliases, description=SCOPE),
    )
    return definition


def dump(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def sha(value):
    return hashlib.sha256(value).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", required=True, help="Two-digit candidate number")
    parser.add_argument("--module", type=Path, default=ROOT / "target/wasm32-wasip1/plugin/zi-json-format-wasi.wasm")
    args = parser.parse_args()
    if len(args.candidate) != 2 or not args.candidate.isascii() or not args.candidate.isdigit():
        parser.error("candidate number must be two ASCII digits")
    dest = ROOT / "release/plugin-candidates" / f"json-file-{VERSION}-candidate-{args.candidate}"
    if dest.exists() or not 0 < args.module.stat().st_size <= 2 * LIMIT:
        parser.error("candidate already exists or module exceeds the supported size")
    capabilities = [capability(suffix, title, aliases) for suffix, _, title, aliases in OPERATIONS]
    manifest = dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools JSON File", displayName="ZiDevTools JSON 文件",
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.6.4 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"resource.input.types.v1": PREVIEW, "resource.output.types.v1": PREVIEW,
                          "output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": PREVIEW, PROFILE: PREVIEW},
        activationEvents=["onCapability:" + item["id"] for item in capabilities],
        runtime={"wasi": "runtime/tools.wasm", "resourceInputProfile": "resource.wasi.input.draft2",
                 "resourceOutputProfile": "resource.wasi.output.draft1"},
        contributes=dict(activities=[], views=[], capabilities=capabilities, commands=[], piTools=[]),
        permissions={"required": [{"name": "resource.read", "reason": "读取本次明确选择和确认的 JSON 文件快照。"}],
                     "optional": []},
    )
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip())
    (dest / "runtime").mkdir(parents=True)
    shutil.copyfile(args.module, dest / "runtime/tools.wasm")
    shutil.copyfile(ROOT / "LICENSE", dest / "LICENSE")
    (dest / "NOTICE").write_text(LICENSES + "\n", encoding="utf-8")
    (dest / "README.md").write_text("# JSON 文件格式化、压缩和校验\n\n" + SCOPE +
                                   "\n\n共用 zi-text-core 严格解析器及格式化核心，拒绝重复键、超过64层嵌套和不安全整数。数值允许规范化词形，但只有精确十进制数值可无损往返时才执行，否则明确拒绝。校验输出独立 JSON 报告；不改写原文件。\n", encoding="utf-8")
    dump(dest / "plugin.json", manifest)
    dump(dest / "catalog.json", dict(
        contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN, packageVersion=VERSION,
        tools=[dict(sourceToolId=source, capabilityId=item["id"], title=item["title"], scope=SCOPE)
               for (_, source, _, _), item in zip(OPERATIONS, capabilities)]))
    dump(dest / "provenance.json", dict(sourceRevision=revision, dirty=dirty, signature="unsigned",
                                      builtAt=datetime.now(timezone.utc).isoformat(),
                                      wasmTarget="wasm32-wasip1", wasmProfile="plugin",
                                      moduleSha256=sha(args.module.read_bytes())))
    files = []
    for path in sorted(dest.rglob("*")):
        if path.is_file():
            content = path.read_bytes()
            files.append(dict(path=path.relative_to(dest).as_posix(), size=len(content), sha256=sha(content)))
    index = sha(json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode())
    dump(dest.with_name(dest.name + "-verification.json"), dict(
        schemaVersion=1, pluginId=PLUGIN, packageVersion=VERSION, signatureVerified=False,
        directoryIndexSha256=index, files=files, fileCount=len(files), totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                    objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                    separators=[",", ":"], trailingNewline=False)))
    print(json.dumps(dict(candidate=str(dest), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
