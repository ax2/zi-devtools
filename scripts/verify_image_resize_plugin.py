#!/usr/bin/env python3
"""Verify the unsigned image-resize candidate inventory and its external evidence."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_PACKAGE = ROOT / "release/plugin-candidates/image-resize-0.1.1-candidate-02"
PROFILES = {
    "resource.wasi.image.resize.v1": "=1.0.0-preview.1",
    "output.schema.v1": "=1.0.0-rc.1",
    "resource.output.types.v1": "=1.0.0-preview.1",
}
SOURCE_FILES = {
    "LICENSE": "LICENSE",
    "plugin.json": "plugins/image-resize/plugin.json",
    "catalog.json": "plugins/image-resize/catalog.json",
    "README.md": "plugins/image-resize/README.md",
    "schemas/resize-input.schema.json": "plugins/image-resize/schemas/resize-input.schema.json",
    "schemas/resize-output.schema.json": "plugins/image-resize/schemas/resize-output.schema.json",
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def inside(path: Path, root: Path) -> Path:
    resolved = path.resolve(strict=True)
    require(resolved.is_relative_to(root.resolve(strict=True)), f"Path escapes allowed root: {path}")
    cursor = resolved
    while cursor != root.resolve(strict=True):
        require(not cursor.is_symlink(), f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", type=Path, default=DEFAULT_PACKAGE)
    args = parser.parse_args()
    package = inside(args.package, ROOT / "release/plugin-candidates")
    proof_path = package.parent / f"{package.name}-verification.json"
    require(proof_path.is_file() and not proof_path.is_symlink(), "External verification receipt missing or linked")
    require(not proof_path.resolve().is_relative_to(package), "Verification receipt must remain outside payload")

    actual = sorted(path.relative_to(package).as_posix() for path in package.rglob("*") if path.is_file())
    for path in package.rglob("*"):
        require(not path.is_symlink(), f"Linked package entry rejected: {path.relative_to(package)}")
    expected = sorted([*SOURCE_FILES, "runtime/tools.wasm"])
    require(actual == expected, "Candidate package contains missing or undeclared files")

    manifest = json.loads((package / "plugin.json").read_text(encoding="utf-8"))
    catalog = json.loads((package / "catalog.json").read_text(encoding="utf-8"))
    require(manifest.get("schemaVersion") == 2, "Manifest schema version mismatch")
    require(manifest.get("id") == "com.zicode.devtools.image" and manifest.get("version") == "0.1.1", "Package identity mismatch")
    require(catalog.get("packageId") == manifest.get("id") and catalog.get("packageVersion") == manifest.get("version"), "Catalog package version mismatch")
    require(manifest.get("requiredProfiles") == PROFILES and catalog.get("requiredProfiles") == PROFILES, "Required profile pins mismatch")
    runtime = manifest.get("runtime", {})
    require(runtime.get("wasi") == "runtime/tools.wasm", "WASI module path mismatch")
    require(runtime.get("resourceInputProfile") == "resource.wasi.input.draft2", "Resource input profile mismatch")
    require(runtime.get("resourceOutputProfile") == "resource.wasi.output.draft1", "Resource output profile mismatch")
    caps = manifest.get("contributes", {}).get("capabilities", [])
    require(len(caps) == 1, "Candidate must expose one capability")
    capability = caps[0]
    require(capability.get("id") == "devtools.image.resize", "Capability ID mismatch")
    require(capability.get("executionProfile") == "resource.wasi.image.resize.v1", "Execution profile missing")
    require(capability.get("resourceInputMax") is None, "Candidate must remain single-input")
    permissions = {item.get("name") for item in manifest.get("permissions", {}).get("required", [])}
    require(permissions == {"resource.read"}, "Permission set is not minimal")
    require(manifest.get("permissions", {}).get("optional", []) == [], "Unexpected optional permissions")
    require(all(manifest.get("contributes", {}).get(key, []) == [] for key in ("activities", "views", "commands", "piTools")), "Unexpected extra executable contribution")
    input_schema = capability.get("inputSchema")
    require(input_schema == json.loads((package / "schemas/resize-input.schema.json").read_text(encoding="utf-8")), "Input schema sidecar differs from manifest")
    require("仅JPEG生效" in input_schema.get("properties", {}).get("quality", {}).get("title", ""), "JPEG quality scope must be explicit in the tool form")
    output_schema = capability.get("outputSchema")
    require(output_schema == json.loads((package / "schemas/resize-output.schema.json").read_text(encoding="utf-8")), "Output schema sidecar differs from manifest")
    require(capability.get("maxResultBytes") == 1024, "Result JSON bound mismatch")

    wasm = package / "runtime/tools.wasm"
    require(0 < wasm.stat().st_size <= 2 * 1024 * 1024, "WASM module size outside Studio package limit")
    proof = json.loads(proof_path.read_text(encoding="utf-8"))
    require(proof.get("schemaVersion") == 1 and proof.get("kind") == "unsigned-provider-candidate", "Receipt schema/kind mismatch")
    require(proof.get("packageId") == manifest["id"] and proof.get("packageVersion") == manifest["version"], "Receipt identity mismatch")
    require(proof.get("candidateDirectory") == package.relative_to(ROOT).as_posix(), "Receipt package path mismatch")
    require(proof.get("signature", {}).get("signed") is False, "Candidate must be clearly unsigned")
    require(proof.get("signature", {}).get("publisherKeyAvailable") is False, "Publisher key status missing")
    require(proof.get("acceptance", {}).get("studioReceiptVerified") is False, "Must not claim Studio acceptance")
    limitations = proof.get("limitations", [])
    require(any("zero-resource adapter failures" in item and "invalid_output" in item for item in limitations), "Host negative-result limitation is missing from the external receipt")
    require(proof.get("profiles") == PROFILES and proof.get("permissions") == ["resource.read"], "Receipt contract mismatch")
    source_tool = proof.get("sourceTool", {})
    source_catalog_path = ROOT / source_tool.get("catalogPath", "")
    require(source_catalog_path.is_file() and sha256(source_catalog_path.read_bytes()) == source_tool.get("catalogSha256"), "Source tool catalog evidence is stale")
    source_catalog = json.loads(source_catalog_path.read_text(encoding="utf-8"))
    registered_tool = next((tool for tool in source_catalog.get("tools", []) if tool.get("id") == source_tool.get("id")), None)
    require(registered_tool is not None, "Source tool ID missing from canonical catalog")
    require(registered_tool.get("tool_version") == source_tool.get("version") and registered_tool.get("status") == source_tool.get("status") and registered_tool.get("scope") == source_tool.get("scope"), "Candidate/source tool scope mapping mismatch")
    packaged_catalog = json.loads((package / "catalog.json").read_text(encoding="utf-8"))
    source_catalog_candidate = json.loads((ROOT / SOURCE_FILES["catalog.json"]).read_text(encoding="utf-8"))
    packaged_source = packaged_catalog.pop("source", None)
    source_catalog_candidate.pop("source", None)
    require(packaged_source == {"revision": proof.get("sourceRevision"), "dirty": proof.get("sourceWorkingTreeDirty")}, "Packaged source revision/dirty declaration mismatch")
    require(packaged_catalog == source_catalog_candidate, "Packaged catalog content differs from provider source")
    entries = proof.get("files", [])
    entry_paths = [entry.get("path") for entry in entries]
    require(sorted(entry_paths) == actual and len(entry_paths) == len(set(entry_paths)), "Receipt file inventory mismatch")
    for entry in entries:
        payload = package / entry["path"]
        require(payload.stat().st_size == entry.get("size"), f"Payload size mismatch: {entry['path']}")
        require(sha256(payload.read_bytes()) == entry.get("sha256"), f"Payload hash mismatch: {entry['path']}")
        if entry["path"] in SOURCE_FILES and entry["path"] != "catalog.json":
            source = inside(ROOT / SOURCE_FILES[entry["path"]], ROOT)
            require(payload.read_bytes() == source.read_bytes(), f"Frozen package source drifted: {entry['path']}")
    index_bytes = json.dumps(entries, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    require(sha256(index_bytes) == proof.get("directoryIndexSha256") == proof.get("candidateTreeSha256"), "Candidate tree hash mismatch")
    require(sha256(wasm.read_bytes()) == proof.get("wasm", {}).get("sha256"), "WASM proof hash mismatch")

    evidence = proof.get("functionalEvidence", {})
    results_path = inside(ROOT / evidence.get("path", ""), ROOT / "target")
    require(sha256(results_path.read_bytes()) == evidence.get("sha256"), "Benchmark receipt hash mismatch")
    results = json.loads(results_path.read_text(encoding="utf-8"))
    require(results.get("wasm", {}).get("sha256") == proof.get("wasm", {}).get("sha256"), "Benchmark did not execute packaged module")
    require(results.get("studioResourceProfile", {}).get("profile") == "resource.wasi.image.resize.v1@1.0.0-preview.1", "Benchmark did not use image-resize profile")
    host = proof.get("studioWorker", {})
    host_path = Path(host.get("path", ""))
    require(host_path.is_file() and sha256(host_path.read_bytes()) == host.get("sha256"), "Studio Worker evidence is stale")
    require(results.get("studioWorker", {}).get("sha256") == host.get("sha256"), "Benchmark Host Worker hash mismatch")
    for name in ("4mp", "16mp", "2mp-transparent"):
        row = next((item for item in results.get("fixtures", []) if item.get("name") == name), None)
        require(row is not None and row.get("studioWorker", {}).get("exitCode") == 0, f"Host Worker fixture did not pass: {name}")
        require(row["studioWorker"].get("result", {}).get("ok") is True, f"Host Worker fixture result invalid: {name}")
        require(len(row["studioWorker"].get("outputResources", [])) == 1, f"Host Worker output missing: {name}")
    matrix = results.get("formatMatrix", [])
    require(len(matrix) == 9, "Host Worker format matrix incomplete")
    require(all(row.get("studioWorker", {}).get("exitCode") == 0 and row.get("studioWorker", {}).get("result", {}).get("ok") is True and len(row.get("studioWorker", {}).get("outputResources", [])) == 1 for row in matrix), "Host Worker format matrix has failures")
    for name, code in (("profile-over-edge", "IMAGE_DIMENSION_LIMIT"), ("damaged", "IMAGE_OPERATION_FAILED")):
        row = next((item for item in results.get("fixtures", []) if item.get("name") == name), None)
        require(row is not None and row.get("studioWorker", {}).get("result", {}).get("ok") is False, f"Invalid input was not rejected: {name}")
        require(row["studioWorker"]["result"].get("error", {}).get("code") == code, f"Stable error code mismatch: {name}")
        require(not row["studioWorker"].get("outputResources"), f"Rejected input emitted output: {name}")
    require(results.get("probes", {}).get("outputLimit", {}).get("result", {}).get("ok") is True, "32 MiB output-boundary probe failed")
    for name, code in (("invalidParameters", "INVALID_PARAMETERS"), ("mimeMismatch", "IMAGE_OPERATION_FAILED")):
        probe = results.get("probes", {}).get(name, {})
        require(probe.get("exitCode") == 0, f"Worker rejection probe failed: {name}")
        require(probe.get("result", {}).get("ok") is False, f"Invalid request was not rejected: {name}")
        require(probe["result"].get("error", {}).get("code") == code, f"Invalid request error code changed: {name}")
        require(probe.get("outputResourceCount") == 0, f"Invalid request emitted an output: {name}")
    diffs = [row["nativeWasiConformance"]["decodedPixelComparison"] for row in matrix]
    require(sum(int(item.get("differentPixels", 0) == 0) for item in diffs) == 7, "Unexpected native/WASI semantic comparison results")
    require(all(item.get("maxChannelDelta", 0) <= 1 for item in diffs), "Native/WASI semantic delta exceeded one channel value")
    require(all(item.get("differentPixels", 0) <= 1 for item in diffs), "Native/WASI changed more than one pixel in a format vector")

    print(f"Verified unsigned provider candidate: {package}")
    print(f"Files: {len(entries)}; payload bytes: {sum(entry['size'] for entry in entries):,}")
    print(f"Candidate tree SHA-256: {proof['candidateTreeSha256']}")
    print(f"WASM SHA-256: {proof['wasm']['sha256']}")
    print(f"Studio Worker SHA-256: {host['sha256']}")
    print("Studio Worker accepted all 12 success fixtures; 7/9 format matrix outputs are pixel-identical and the other two differ by one channel value at one pixel each")
    print("Unsigned candidate only; package installation and Host end-to-end acceptance remain pending")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
