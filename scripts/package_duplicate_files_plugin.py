"""Freeze a receipt-verified independent duplicate-file plugin candidate."""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import digest, dump, integer, nullable, obj, string
from verify_duplicate_files_plugin import (
    CAPABILITY, MAX_REPORT, PERMISSION, PLUGIN, PROFILE, cases, payload,
)
from verify_disk_inspector_plugin import compact

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.1.0"
TITLE = "重复文件检查"
SCOPE = "按已确认的相对路径、逻辑长度和 SHA-256 分组，显示理论重复大小并标记范围限制。"


def report_schema():
    boolean = {"type": "boolean"}
    count = integer(4096)
    group = obj(logicalBytes=string(20), sha256=string(64), redundantBytes=string(39),
                fileCount=integer(256), paths={"type": "array", "maxItems": 12, "items": string(2048)})
    limits = {"type": "string", "enum": ["entries", "time", "candidate_files", "hash_bytes", "snapshot_bytes"]}
    source = obj(
        visitedEntries=count, candidateFiles=count, hashedFiles=integer(256), hashedBytes=string(20),
        skippedLinks=count, skippedDepth=count, skippedNames=count, unreadable=count, changedFiles=count,
        skippedBudget=count, limitReason=nullable(limits), partial=boolean,
    )
    success = obj(
        ok={"type": "boolean", "const": True},
        schemaVersion={"type": "integer", "const": 1},
        material={"type": "string", "const": "duplicate-files-report"},
        partial=boolean, groupCount=integer(128), duplicateFileCount=integer(256), redundantBytes=string(39),
        groups={"type": "array", "maxItems": 20, "items": group},
        projectionTruncated=boolean, source=source,
    )
    error = obj(ok={"type": "boolean", "const": False}, error=obj(code=string(40), message=string(256)))
    return {"type": "object", "oneOf": [success, error]}


def manifest():
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Duplicate Files", displayName=TITLE,
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
            maxResultBytes=MAX_REPORT,
            directory=dict(categoryId="files", aliases=["查找重复文件", "文件去重检查", "duplicate files"],
                           description=SCOPE),
        )]),
        permissions=dict(required=[
            dict(name="resource.read", reason="读取本次确认的临时目录摘要快照。"),
            dict(name=PERMISSION, reason="明确确认后由 Studio 筛选同大小候选并计算 SHA-256。"),
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
    dest = ROOT / "release/plugin-candidates" / f"duplicate-files-{VERSION}-candidate-{args.candidate}"
    receipt = dest.with_name(dest.name + "-verification.json")
    if dest.exists() or receipt.exists():
        parser.error("candidate or external receipt already exists")

    wasm = args.module.read_bytes()
    evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(cases())
    expected_inputs = [(case, digest(payload(value)), expected["ok"])
                       for case, value, expected in vectors]
    actual_inputs = [(row["case"], row["inputSha256"], row["ok"]) for row in evidence["cases"]]
    if (not 0 < len(wasm) <= 2 * 1024 * 1024 or evidence["wasmSha256"] != digest(wasm)
            or evidence["passed"] != len(vectors) or actual_inputs != expected_inputs):
        parser.error("verification evidence does not match module or fixed vectors")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    tracked = subprocess.check_output(
        ["git", "status", "--porcelain", "--untracked-files=no"], cwd=ROOT, text=True,
    )
    if tracked.strip():
        parser.error("commit package source before freezing it")

    (dest / "runtime").mkdir(parents=True)
    (dest / "runtime/tools.wasm").write_bytes(wasm)
    shutil.copyfile(ROOT / "LICENSE", dest / "LICENSE")
    shutil.copyfile(ROOT / "plugins/duplicate-files/README.md", dest / "README.md")
    (dest / "NOTICE").write_text(
        "ZiDevTools and the WASI resource reader are MIT. Serde and serde_json retain their upstream licenses.\n",
        encoding="utf-8",
    )
    dump(dest / "plugin.json", manifest())
    dump(dest / "catalog.json", dict(
        contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN, packageVersion=VERSION,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId="duplicate-files", capabilityId=CAPABILITY, title=TITLE,
                    status="candidate", inputProfile=PROFILE, scope=SCOPE)],
    ))
    dump(dest / "provenance.json", dict(
        sourceRevision=revision, dirty=False, signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(), wasmTarget="wasm32-wasip1",
        wasmProfile="plugin", moduleSha256=digest(wasm), workerVerification=evidence,
    ))
    dump(dest / "fixtures.json", dict(
        generator="scripts/verify_duplicate_files_plugin.py",
        cases=[
            dict(id=name, snapshot=value if len(payload(value)) <= 4096 else None,
                 inputSha256=digest(payload(value)), expected=expected)
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
        directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                    objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                    separators=[",", ":"], trailingNewline=False),
    ))
    print(json.dumps(dict(candidate=str(dest), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
