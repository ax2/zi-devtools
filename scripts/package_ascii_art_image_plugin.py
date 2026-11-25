"""Freeze an unsigned, locally verified image-to-ASCII provider candidate."""

from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

from benchmark_studio_image_resource import MEMORY_LIMIT, sha256, wasm_memory_ceiling
from package_disk_inspector_plugin import dump
from package_duplicate_files_plugin import candidate_arguments
from verify_ascii_art_image_plugin import CAPABILITY, INPUT, OUTPUT, PLUGIN, PROFILE, TEST_EXECUTION, compact, fixture_matrix

ROOT = Path(__file__).resolve().parents[1]
SLUG = "ascii-art-image"
SOURCE_TOOL = "ascii-art"
PLUGIN_SOURCE = ROOT / "plugins" / SLUG


def main():
    parser, args = candidate_arguments(__doc__)
    if not (len(args.candidate) == 2 and args.candidate.isascii() and args.candidate.isdecimal()):
        parser.error("use two ASCII digits for candidate")
    manifest = json.loads((PLUGIN_SOURCE / "plugin.json").read_text(encoding="utf-8"))
    capability, = manifest["contributes"]["capabilities"]
    if (manifest["id"] != PLUGIN or capability["id"] != CAPABILITY
            or manifest["requiredProfiles"][PROFILE] != "=1.0.0-preview.1"
            or capability["executionProfile"] != PROFILE
            or manifest["runtime"] != {"wasi": "runtime/tools.wasm", "resourceInputProfile": INPUT,
                                       "resourceOutputProfile": OUTPUT}
            or [entry["name"] for entry in manifest["permissions"]["required"]] != ["resource.read"]
            or manifest["permissions"]["optional"]):
        parser.error("provider manifest does not match the ASCII resource contract")

    version = manifest["version"]
    target = ROOT / "release" / "plugin-candidates" / f"{SLUG}-{version}-candidate-{args.candidate}"
    receipt = target.with_name(target.name + "-verification.json")
    if target.exists() or receipt.exists():
        parser.error("candidate and receipt must be new")
    module = args.module.read_bytes()
    proof = json.loads(args.evidence.read_text(encoding="utf-8"))
    with TemporaryDirectory() as directory:
        vectors = [(key, sha256(path.read_bytes()), width, mime, error)
                   for key, path, mime, width, error in fixture_matrix(Path(directory), quick=False)]
    actual = [(row["case"], row["inputSha256"], row["requestedWidth"], row["mime"], row["expectedError"])
              for row in proof["cases"]]
    if (not 0 < len(module) <= 2 * 1024 * 1024 or proof["sha256"]["wasm"] != sha256(module)
            or wasm_memory_ceiling(module) != MEMORY_LIMIT or proof["moduleMemoryBytes"] != MEMORY_LIMIT
            or proof["targetProfile"] != PROFILE or proof["workerAdapterProfile"] not in (TEST_EXECUTION, PROFILE)
            or proof["passed"] != len(vectors) or actual != vectors
            or not all(row["passed"] and row["matchesBaselineNative"] is not False for row in proof["cases"])):
        parser.error("module or fixed verification cases differ")

    def git(*arguments):
        return subprocess.check_output(["git", *arguments], cwd=ROOT, text=True).strip()

    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit candidate source before freezing")
    (target / "runtime").mkdir(parents=True)
    (target / "runtime/tools.wasm").write_bytes(module)
    for source, destination in [(ROOT / "LICENSE", "LICENSE"), (PLUGIN_SOURCE / "README.md", "README.md"),
                                (PLUGIN_SOURCE / "plugin.json", "plugin.json")]:
        shutil.copyfile(source, target / destination)
    (target / "NOTICE").write_text(
        "ZiDevTools is MIT. Image, serde and other dependencies retain their upstream licenses.\n",
        encoding="utf-8",
    )
    dump(target / "catalog.json", dict(
        contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN, packageVersion=version,
        deliveryStatus="unsigned_provider_candidate_not_host_accepted",
        tools=[dict(sourceToolId=SOURCE_TOOL, capabilityId=CAPABILITY, title=manifest["displayName"],
                    status="candidate", inputProfile=INPUT, scope=manifest["description"])],
    ))
    dump(target / "fixtures.json", dict(
        generator="scripts/verify_ascii_art_image_plugin.py",
        cases=[dict(id=key, inputSha256=hash_value, input=width, mime=mime, error=error)
               for key, hash_value, width, mime, error in vectors],
    ))
    dump(target / "provenance.json", dict(
        sourceRevision=git("rev-parse", "HEAD"), dirty=False, signature="unsigned",
        builtAt=datetime.now(timezone.utc).isoformat(), wasmTarget="wasm32-wasip1",
        wasmProfile="plugin, 256 MiB maximum memory, 2 MiB stack", moduleSha256=sha256(module),
        workerVerification=proof,
    ))
    files = [dict(path=path.relative_to(target).as_posix(), size=path.stat().st_size,
                  sha256=sha256(path.read_bytes())) for path in sorted(target.rglob("*")) if path.is_file()]
    index = sha256(compact(files))
    dump(receipt, dict(
        schemaVersion=1, pluginId=PLUGIN, packageVersion=version, signatureVerified=False,
        directoryIndexSha256=index, files=files, fileCount=len(files),
        totalBytes=sum(row["size"] for row in files),
        directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                    objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                    separators=[",", ":"], trailingNewline=False),
    ))
    print(json.dumps(dict(candidate=str(target), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
