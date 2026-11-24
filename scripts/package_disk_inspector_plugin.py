"""Freeze a receipt-verified, unsigned directory usage plugin candidate."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

from verify_disk_inspector_plugin import CAPABILITY, MAX_INPUT, PLUGIN, PROFILE, cases, compact

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.1.0"
TITLE = "目录空间分析"
SCOPE = "对明确共享的目录快照统计逻辑字节、直属子目录和最大文件。两张排行各显示前 20 条。部分结果会显示覆盖限制。"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def dump(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n",
                    encoding="utf-8", newline="\n")


def obj(**fields):
    return {"type": "object", "properties": fields, "required": list(fields),
            "additionalProperties": False}


def string(max_length):
    return {"type": "string", "maxLength": max_length}


def integer(maximum):
    return {"type": "integer", "minimum": 0, "maximum": maximum}


def nullable(value):
    return {"oneOf": [{"type": "null"}, value]}


def report_schema():
    boolean = {"type": "boolean"}
    count = integer(4096)
    budget = integer(2 * 1024 * 1024)
    usage = obj(path=string(512), logicalBytes=string(40), pathTruncated=boolean)
    rows = {"type": "array", "maxItems": 20, "items": usage}
    limit = nullable(string(32))
    success = obj(
        ok={"type": "boolean", "const": True},
        schemaVersion={"type": "integer", "const": 1},
        material={"type": "string", "const": "directory-usage-report"},
        partial=boolean, logicalBytes=string(40), rootFilesLogicalBytes=string(40),
        fileCount=count, directoryCount=count, groupCount=count, groups=rows, largestFiles=rows,
        source=nullable(obj(visitedEntries=count, skippedLinks=count, skippedDepth=count,
                            skippedNames=count, unreadable=count, limitReason=limit,
                            partial=boolean)),
        analysis=obj(examinedEntries=count, inputBytes=budget, readBytes=integer(MAX_INPUT),
                     limitReason=limit),
        limits=obj(hostMaxEntries=count, hostMaxDepth=count, hostScanSeconds=count,
                   hostMaxSnapshotBytes=budget, maxAnalysisBytes=budget,
                   displayedPathBytes=count, displayedRowsPerList=count),
    )
    error = obj(ok={"type": "boolean", "const": False},
                error=obj(code=string(40), message=string(256)))
    return {"type": "object", "oneOf": [success, error]}


def manifest():
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Directory Usage", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"resource.input.types.v1": "=1.0.0-preview.1",
                          "output.schema.v1": "=1.0.0-rc.1",
                          "tool.directory.v1": "=1.0.0-preview.1"},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime={"wasi": "runtime/tools.wasm", "resourceInputProfile": "resource.wasi.input.draft2"},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=SCOPE,
            resourceInputMimeTypes=["text/plain"], inputSchema=obj(), outputSchema=report_schema(),
            maxResultBytes=48 * 1024,
            directory=dict(categoryId="files",
                           aliases=["磁盘占用", "目录大小", "查看大文件", "disk usage"],
                           description=SCOPE),
        )]),
        permissions=dict(required=[
            dict(name="resource.read", reason="读取本次确认的临时目录元数据快照。"),
            dict(name="resource.directory.usage.read", reason="经明确确认扫描所选目录的相对名称、类型和逻辑长度。"),
        ], optional=[]),
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--module", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--candidate", default="01")
    args = parser.parse_args()
    if not (len(args.candidate) == 2 and args.candidate.isascii() and args.candidate.isdigit()):
        parser.error("candidate number must contain two ASCII digits")
    dest = ROOT / "release/plugin-candidates" / f"disk-inspector-{VERSION}-candidate-{args.candidate}"
    receipt = dest.with_name(dest.name + "-verification.json")
    if dest.exists() or receipt.exists():
        parser.error("candidate or external receipt already exists")
    wasm = args.module.read_bytes()
    evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(cases())
    expected_inputs = [(name, digest(compact(snapshot)), expected["ok"])
                       for name, snapshot, expected in vectors]
    actual_inputs = [(row["case"], row["inputSha256"], row["ok"]) for row in evidence["cases"]]
    if (not 0 < len(wasm) <= 2 * 1024 * 1024
            or evidence["wasmSha256"] != digest(wasm)
            or evidence["passed"] != len(vectors)
            or actual_inputs != expected_inputs):
        parser.error("verification evidence does not match module or fixed vectors")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    tracked = subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True,
    )
    if tracked.strip():
        parser.error("commit the package source before freezing it")
    (dest / "runtime").mkdir(parents=True)
    (dest / "runtime/tools.wasm").write_bytes(wasm)
    shutil.copyfile(ROOT / "LICENSE", dest / "LICENSE")
    shutil.copyfile(ROOT / "plugins/disk-inspector/README.md", dest / "README.md")
    (dest / "NOTICE").write_text(
        "ZiDevTools and the WASI resource reader are MIT. Serde and serde_json retain their upstream licenses.\n",
        encoding="utf-8",
    )
    dump(dest / "plugin.json", manifest())
    dump(dest / "catalog.json", dict(
        contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN, packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="disk-inspector", capabilityId=CAPABILITY, title=TITLE,
                    status="candidate", inputProfile=PROFILE, scope=SCOPE)],
    ))
    dump(dest / "provenance.json", dict(
        sourceRevision=revision, dirty=False, signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(), wasmTarget="wasm32-wasip1",
        wasmProfile="plugin", moduleSha256=digest(wasm),
        workerVerification=evidence,
    ))
    dump(dest / "fixtures.json", dict(
        generator="scripts/verify_disk_inspector_plugin.py",
        cases=[
            dict(id=name, snapshot=value if len(compact(value)) <= 4096 else None,
                 inputSha256=digest(compact(value)), expected=expected)
            for name, value, expected in vectors
        ],
    ))
    files = []
    for path in sorted(dest.rglob("*")):
        if path.is_file():
            data = path.read_bytes()
            files.append(dict(path=path.relative_to(dest).as_posix(), size=len(data), sha256=digest(data)))
    index = digest(compact(files))
    dump(receipt, dict(
        schemaVersion=1, pluginId=PLUGIN, packageVersion=VERSION, signatureVerified=False,
        directoryIndexSha256=index, files=files, fileCount=len(files),
        totalBytes=sum(item["size"] for item in files),
        directoryIndexEncoding=dict(
            hash="SHA-256", encoding="UTF-8", content="files array in listed order",
            objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
            separators=[",", ":"], trailingNewline=False,
        ),
    ))
    print(json.dumps(dict(candidate=str(dest), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
