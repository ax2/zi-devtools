"""Freeze a new unsigned JSON Schema candidate and an external receipt."""
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import shutil

from package_disk_inspector_plugin import dump, obj, string
from package_duplicate_files_plugin import candidate_arguments
from verify_json_schema_plugin import CAPABILITY, FIELD_LIMIT, IPC_LIMIT, PLUGIN, compact, digest, fixtures

ROOT = Path(__file__).resolve().parents[1]
SLUG = "json-schema"
VERSION = "0.1.0"
CONTRACT = "1.0.0-rc.1"
TITLE = "JSON Schema 校验"
SCOPE = "离线粘贴 JSON 和 Schema。支持 Draft 4、6、7（默认 Draft 7）及文档内引用，每项最多 8 KiB。"


def manifest():
    version = {"type": "string", "const": CONTRACT}
    absent = {"type": "null"}
    success = obj(contractVersion=version, ok={"type": "boolean", "const": True},
                  data=obj(text=string(IPC_LIMIT)), error=absent)
    failure = obj(contractVersion=version, ok={"type": "boolean", "const": False},
                  data=absent, error=obj(code=string(32), message=string(IPC_LIMIT)))
    return dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools JSON Schema", displayName=TITLE,
        description=SCOPE, version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={"output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": "=1.0.0-preview.1"},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime={"wasi": "runtime/tools.wasm"},
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=SCOPE,
            inputSchema=obj(instance=dict(string(FIELD_LIMIT), title="JSON 实例 / JSON instance", minLength=1),
                            schema=dict(string(FIELD_LIMIT), title="JSON Schema", minLength=1)),
            outputSchema={"type": "object", "oneOf": [success, failure]},
            maxResultBytes=IPC_LIMIT,
            directory=dict(categoryId="data", aliases=["JSON 文档验证", "json schema validator"], description=SCOPE),
        )]),
        permissions=dict(required=[], optional=[]),
    )


def main():
    parser, args = candidate_arguments(__doc__)
    if not (len(args.candidate) == 2 and args.candidate.isascii() and args.candidate.isdecimal()):
        parser.error("use two ASCII digits for candidate")
    target = ROOT / "release/plugin-candidates" / f"{SLUG}-{VERSION}-candidate-{args.candidate}"
    receipt = target.with_name(target.name + "-verification.json")
    if target.exists() or receipt.exists():
        parser.error("candidate and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    vectors = list(fixtures())
    expected = [(name, digest(compact(data))) for name, data, _, _ in vectors]
    actual = [(row["case"], row["inputSha256"]) for row in proof["cases"]]
    if not 0 < len(module) <= 2 * 1024 * 1024 or proof["wasmSha256"] != digest(module) or actual != expected:
        parser.error("WASM digest or fixed conformance vectors differ")
    git = lambda *args: subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()
    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")

    (target / "runtime").mkdir(parents=True)
    (target / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", target / "LICENSE")
    shutil.copyfile(ROOT / "plugins/json-schema/README.md", target / "README.md")
    (target / "NOTICE").write_text("ZiDevTools and jsonschema are MIT. Other Rust dependencies retain their upstream licenses.\n", encoding="utf-8")
    dump(target / "plugin.json", manifest())
    dump(target / "catalog.json", dict(contract="zicode.devtools-plugin/" + CONTRACT, packageId=PLUGIN,
                                      packageVersion=VERSION, deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                                      tools=[dict(sourceToolId=SLUG, capabilityId=CAPABILITY, title=TITLE,
                                                  status="candidate", scope=SCOPE)]))
    dump(target / "fixtures.json", dict(generator="scripts/verify_json_schema_plugin.py",
                                      cases=[dict(id=name, inputSha256=digest(compact(data)),
                                                  input=data if len(compact(data)) <= 4096 else None,
                                                  expected=prefix, ok=ok) for name, data, prefix, ok in vectors]))
    dump(target / "provenance.json", dict(sourceRevision=git("rev-parse", "HEAD"), dirty=False, signature="unsigned",
                                         builtAt=datetime.now(timezone.utc).isoformat(), wasmTarget="wasm32-wasip1",
                                         wasmProfile="plugin; opt-level=z; --compress-relocations",
                                         moduleSha256=digest(module), workerVerification=proof))
    files = [dict(path=path.relative_to(target).as_posix(), size=path.stat().st_size,
                  sha256=digest(path.read_bytes())) for path in sorted(target.rglob("*")) if path.is_file()]
    index = digest(compact(files))
    dump(receipt, dict(schemaVersion=1, pluginId=PLUGIN, packageVersion=VERSION, signatureVerified=False,
                       directoryIndexSha256=index, files=files, fileCount=len(files),
                       totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                                   objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                                   separators=[",", ":"], trailingNewline=False)))
    print(json.dumps(dict(candidate=str(target), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
