"""Freeze a new unsigned image-crop plugin with external verification."""
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess

from package_disk_inspector_plugin import dump, obj
from package_duplicate_files_plugin import candidate_arguments
from verify_image_crop_plugin import CAPABILITY, FORMATS, INPUT, OUTPUT, PLUGIN, PROFILE, WIRE_PROFILE, compact, digest, vectors

ROOT = Path(__file__).resolve().parents[1]
SLUG = "image-crop"
VERSION = "0.1.0"
SOURCE_TOOL = "image-crop-annotate"
TITLE = "图片裁剪 / Crop image"
DESCRIPTION = "按像素裁剪一张已确认的图片，生成无损 PNG。Crop one approved image by pixel coordinates and produce a lossless PNG."


def integer(label, least, greatest):
    return dict(type="integer", title=label, minimum=least, maximum=greatest)


def sections():
    preview = "=1.0.0-preview.1"
    inputs = obj(x=dict(integer("左侧 / Left (px)", 0, 4095), default=0),
                 y=dict(integer("顶部 / Top (px)", 0, 4095), default=0),
                 width=integer("宽度 / Width (px)", 1, 4096),
                 height=integer("高度 / Height (px)", 1, 4096))
    outputs = obj(ok={"type": "boolean", "const": True},
                  width=integer("Width", 1, 4096), height=integer("Height", 1, 4096),
                  sourceWidth=integer("Source width", 1, 4096), sourceHeight=integer("Source height", 1, 4096),
                  mimeType={"type": "string", "const": "image/png"},
                  byteLength=integer("PNG bytes", 1, 32 * 1024 * 1024))
    manifest = dict(
        schemaVersion=2, id=PLUGIN, name="ZiDevTools Image Crop", displayName=TITLE, description=DESCRIPTION,
        version=VERSION, publisher="zicode", license="MIT",
        engines={"zicode": ">=0.7.2 <1.0.0", "pluginApi": "^1.0.0"},
        requiredProfiles={PROFILE: preview, "output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": preview,
                          "resource.input.types.v1": preview, "resource.output.types.v1": preview},
        activationEvents=["onCapability:" + CAPABILITY],
        runtime=dict(wasi="runtime/tools.wasm", resourceInputProfile=INPUT, resourceOutputProfile=OUTPUT),
        contributes=dict(activities=[], views=[], commands=[], piTools=[], capabilities=[dict(
            id=CAPABILITY, title=TITLE, description=DESCRIPTION, inputSchema=inputs, outputSchema=outputs,
            maxResultBytes=1024, executionProfile=PROFILE, resourceInputMimeTypes=list(FORMATS.values()),
            directory=dict(categoryId="media", aliases=["裁剪图片", "PNG crop", "image clipping"], description=DESCRIPTION),
        )]),
        permissions=dict(required=[dict(name="resource.read", reason="Read only the image selected and approved for this run.")], optional=[]),
    )
    catalog = dict(contract="zicode.devtools-plugin/1.0.0-rc.1", packageId=PLUGIN, packageVersion=VERSION,
                   deliveryStatus="unsigned_provider_candidate_not_host_accepted",
                   tools=[dict(sourceToolId=SOURCE_TOOL, capabilityId=CAPABILITY, title=TITLE,
                               status="candidate", scope=DESCRIPTION)])
    return manifest, catalog, inputs, outputs


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
    expected = [(name, digest(source), rect, mime, error) for name, source, mime, rect, error in vectors()]
    actual = [(row["case"], row["inputSha256"], row["rectangle"], row["inputMimeType"], row.get("error")) for row in proof["cases"]]
    if not 0 < len(module) <= 2 * 1024 * 1024 or proof["wasmSha256"] != digest(module) or actual != expected or proof["workerProfile"] != WIRE_PROFILE:
        parser.error("WASM digest or fixed conformance vectors differ")
    manifest, catalog, inputs, outputs = sections()
    git = lambda *args: subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()
    if git("status", "--porcelain", "--untracked-files=no"):
        parser.error("commit source before freezing")

    (target / "runtime").mkdir(parents=True)
    (target / "schemas").mkdir()
    (target / "runtime/tools.wasm").write_bytes(module)
    shutil.copyfile(ROOT / "LICENSE", target / "LICENSE")
    shutil.copyfile(ROOT / "plugins/image-crop/README.md", target / "README.md")
    dump(target / "plugin.json", manifest)
    dump(target / "catalog.json", catalog)
    dump(target / "schemas/crop-input.schema.json", inputs)
    dump(target / "schemas/crop-output.schema.json", outputs)
    dump(target / "provenance.json", dict(sourceRevision=git("rev-parse", "HEAD"), dirty=False, signature="unsigned",
                                         builtAt=datetime.now(timezone.utc).isoformat(), wasmTarget="wasm32-wasip1",
                                         wasmProfile="plugin with zi-image-wasi/crop", moduleSha256=digest(module),
                                         workerVerification=proof))
    files = [dict(path=path.relative_to(target).as_posix(), size=path.stat().st_size,
                  sha256=digest(path.read_bytes())) for path in sorted(target.rglob("*")) if path.is_file()]
    index = digest(compact(files))
    dump(receipt, dict(schemaVersion=1, pluginId=PLUGIN, packageVersion=VERSION, signatureVerified=False,
                       directoryIndexSha256=index, files=files, fileCount=len(files), totalBytes=sum(row["size"] for row in files),
                       directoryIndexEncoding=dict(hash="SHA-256", encoding="UTF-8", content="files array in listed order",
                                                   objectKeyOrder=["path", "size", "sha256"], ensureAscii=False,
                                                   separators=[",", ":"], trailingNewline=False)))
    print(json.dumps(dict(candidate=str(target), directoryIndexSha256=index, files=len(files))))


if __name__ == "__main__":
    main()
