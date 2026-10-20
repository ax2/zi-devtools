#!/usr/bin/env python3
"""Build a hash-indexed unsigned Studio image-resize provider candidate directory."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
RELEASE = ROOT / "release"
DEFAULT_WASM = ROOT / "target/wasm32-wasip1/plugin/zi-image-wasi.wasm"
DEFAULT_RESULTS = ROOT / "target/image-resource-benchmark/results.json"
DEFAULT_WORKER = Path(r"E:\zicode-studio-desktop\target\debug\zicode-studio-desktop.exe")
PROFILES = {
    "resource.wasi.image.resize.v1": "=1.0.0-preview.1",
    "output.schema.v1": "=1.0.0-rc.1",
    "resource.output.types.v1": "=1.0.0-preview.1",
}
PAYLOAD = {
    "LICENSE": ROOT / "LICENSE",
    "plugin.json": ROOT / "plugins/image-resize/plugin.json",
    "catalog.json": ROOT / "plugins/image-resize/catalog.json",
    "README.md": ROOT / "plugins/image-resize/README.md",
    "schemas/resize-input.schema.json": ROOT / "plugins/image-resize/schemas/resize-input.schema.json",
    "schemas/resize-output.schema.json": ROOT / "plugins/image-resize/schemas/resize-output.schema.json",
}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def safe_path(path: Path, *, must_exist: bool = True) -> Path:
    resolved = path.resolve(strict=must_exist)
    root = ROOT.resolve(strict=True)
    if not resolved.is_relative_to(root):
        raise RuntimeError(f"Path escapes project root: {path}")
    cursor = resolved
    while cursor != root:
        if cursor.exists() and cursor.is_symlink():
            raise RuntimeError(f"Linked path rejected: {cursor}")
        cursor = cursor.parent
    return resolved


def sha_file(path: Path) -> str:
    return sha256(path.read_bytes())


def load_results(path: Path, module_sha: str, worker_sha: str) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("wasm", {}).get("sha256") != module_sha:
        raise RuntimeError("Benchmark module hash does not match candidate WASM")
    if data.get("studioWorker", {}).get("sha256") != worker_sha:
        raise RuntimeError("Benchmark Studio Worker hash does not match selected Worker")
    profile = data.get("studioResourceProfile", {})
    expected = {
        "profile": "resource.wasi.image.resize.v1@1.0.0-preview.1",
        "hostMemoryBytes": 256 * 1024 * 1024,
        "fuel": 30_000_000_000,
        "executionDeadlineMs": 10_000,
        "maxInputResources": 1,
        "maxOutputResources": 1,
        "globalWorkerSlots": 2,
        "perPluginWorkerSlots": 1,
    }
    if any(profile.get(key) != value for key, value in expected.items()):
        raise RuntimeError("Results do not record the negotiated image-resize profile")
    fixtures = {row["name"]: row for row in data.get("fixtures", [])}
    for name in ("4mp", "16mp", "2mp-transparent"):
        row = fixtures.get(name, {})
        if row.get("studioWorker", {}).get("exitCode") != 0:
            raise RuntimeError(f"Studio Worker fixture failed: {name}")
        result = row["studioWorker"].get("result", {})
        if result.get("ok") is not True or len(row["studioWorker"].get("outputResources", [])) != 1:
            raise RuntimeError(f"Studio Worker did not publish one successful image output: {name}")
        direct = row.get("nativeWasiConformance", {}).get("wasm", {})
        if row["studioWorker"]["outputResources"][0].get("sha256") != direct.get("sha256"):
            raise RuntimeError(f"Studio Worker bytes differ from direct WASI output: {name}")
    matrix = data.get("formatMatrix", [])
    if len(matrix) != 9 or any(
        row.get("studioWorker", {}).get("exitCode") != 0
        or row.get("studioWorker", {}).get("result", {}).get("ok") is not True
        or len(row.get("studioWorker", {}).get("outputResources", [])) != 1
        for row in matrix
    ):
        raise RuntimeError("All nine Studio Worker format combinations must pass")
    for row in matrix:
        direct = row.get("nativeWasiConformance", {}).get("wasm", {})
        if row["studioWorker"]["outputResources"][0].get("sha256") != direct.get("sha256"):
            raise RuntimeError(f"Studio Worker bytes differ from direct WASI output: {row['case']}")
    for name, code in (("profile-over-edge", "IMAGE_DIMENSION_LIMIT"), ("damaged", "IMAGE_OPERATION_FAILED")):
        row = fixtures.get(name, {})
        result = row.get("studioWorker", {}).get("result", {})
        if result.get("ok") is not False or result.get("error", {}).get("code") != code:
            raise RuntimeError(f"Studio Worker rejection fixture changed unexpectedly: {name}")
        if row.get("studioWorker", {}).get("outputResources"):
            raise RuntimeError(f"Rejected input published an output: {name}")
    if data.get("probes", {}).get("outputLimit", {}).get("result", {}).get("ok") is not True:
        raise RuntimeError("The 32 MiB output-boundary probe did not pass")
    for name, code in (("invalidParameters", "INVALID_PARAMETERS"), ("mimeMismatch", "IMAGE_OPERATION_FAILED")):
        probe = data.get("probes", {}).get(name, {})
        result = probe.get("result", {})
        if probe.get("exitCode") != 0 or result.get("ok") is not False:
            raise RuntimeError(f"Studio Worker did not reject invalid request: {name}")
        if result.get("error", {}).get("code") != code or probe.get("outputResourceCount") != 0:
            raise RuntimeError(f"Invalid request rejection contract changed: {name}")
    return data


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wasm", type=Path, default=DEFAULT_WASM)
    parser.add_argument("--worker", type=Path, default=DEFAULT_WORKER)
    parser.add_argument("--results", type=Path, default=DEFAULT_RESULTS)
    parser.add_argument(
        "--out",
        type=Path,
        default=RELEASE / "plugin-candidates/image-resize-0.1.1-candidate-02",
    )
    parser.add_argument("--refresh-proof", action="store_true", help="refresh only the external receipt for an existing frozen candidate directory")
    args = parser.parse_args()

    wasm = safe_path(args.wasm)
    worker = args.worker.resolve(strict=True)
    results = safe_path(args.results)
    if "zicode-studio-desktop" not in str(worker).lower() or not worker.is_file():
        raise RuntimeError("Selected Host Worker is not a Studio desktop executable")
    module = wasm.read_bytes()
    module_sha = sha256(module)
    worker_sha = sha_file(worker)
    evidence = load_results(results, module_sha, worker_sha)
    if len(module) == 0 or len(module) > 2 * 1024 * 1024:
        raise RuntimeError("WASM module must be non-empty and no larger than 2 MiB")
    if evidence.get("wasm", {}).get("moduleMemoryCeilingBytes") != 256 * 1024 * 1024:
        raise RuntimeError("WASM module ceiling is not 256 MiB")

    manifest = json.loads(PAYLOAD["plugin.json"].read_text(encoding="utf-8"))
    catalog = json.loads(PAYLOAD["catalog.json"].read_text(encoding="utf-8"))
    if manifest.get("schemaVersion") != 2 or manifest.get("id") != "com.zicode.devtools.image":
        raise RuntimeError("Unexpected image plugin identity or manifest schema")
    if catalog.get("packageId") != manifest.get("id") or catalog.get("packageVersion") != manifest.get("version"):
        raise RuntimeError("Plugin manifest and catalog package versions differ")
    if manifest.get("requiredProfiles") != PROFILES or catalog.get("requiredProfiles") != PROFILES:
        raise RuntimeError("Manifest and catalog must require the exact negotiated profiles")
    if manifest.get("runtime", {}).get("wasi") != "runtime/tools.wasm":
        raise RuntimeError("Manifest runtime path is invalid")
    caps = manifest.get("contributes", {}).get("capabilities", [])
    if len(caps) != 1 or caps[0].get("id") != "devtools.image.resize":
        raise RuntimeError("Candidate must contain only the resize capability")
    if caps[0].get("executionProfile") != "resource.wasi.image.resize.v1":
        raise RuntimeError("Resize execution profile is missing")
    if caps[0].get("resourceInputMax") is not None:
        raise RuntimeError("Single-image profile must not widen resource input count")
    if "resource.read" not in {
        entry.get("name") for entry in manifest.get("permissions", {}).get("required", [])
    }:
        raise RuntimeError("The resource.read permission must be declared")
    if len(evidence.get("formatMatrix", [])) != 9:
        raise RuntimeError("The source fixture matrix is incomplete")

    destination = args.out.resolve(strict=False)
    allowed_parent = (RELEASE / "plugin-candidates").resolve(strict=False)
    if not destination.is_relative_to(allowed_parent) or destination == allowed_parent:
        raise RuntimeError("Candidate output must be a new child of release/plugin-candidates")
    if destination.is_symlink() or (destination.exists() and not args.refresh_proof):
        raise RuntimeError(f"Refusing to replace existing candidate: {destination}")
    if args.refresh_proof and not destination.is_dir():
        raise RuntimeError("Proof refresh requires an existing candidate directory")
    proof_path = destination.parent / f"{destination.name}-verification.json"
    if proof_path.is_symlink() or (proof_path.exists() and not args.refresh_proof):
        raise RuntimeError(f"Refusing to replace existing external verification: {proof_path}")
    for source in PAYLOAD.values():
        safe_path(source)
    if destination.parent.exists() and destination.parent.is_symlink():
        raise RuntimeError("Linked candidate output parent rejected")
    destination.parent.mkdir(parents=True, exist_ok=True)

    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.strip()
    dirty = bool(subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip())
    source_catalog = json.loads((ROOT / "docs/tools.json").read_text(encoding="utf-8"))
    source_tool = next((tool for tool in source_catalog["tools"] if tool["id"] == "image-workflow"), None)
    if source_tool is None:
        raise RuntimeError("image-workflow source tool is missing from docs/tools.json")

    files: list[dict] = []
    try:
        if not args.refresh_proof:
            for relative, source in PAYLOAD.items():
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(safe_path(source), target)
                if relative == "catalog.json":
                    packaged_catalog = json.loads(target.read_text(encoding="utf-8"))
                    packaged_catalog["source"] = {"revision": revision, "dirty": dirty}
                    target.write_text(json.dumps(packaged_catalog, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="")
            runtime_path = destination / "runtime/tools.wasm"
            runtime_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(wasm, runtime_path)
        expected_paths = {*PAYLOAD, "runtime/tools.wasm"}
        actual_paths = {path.relative_to(destination).as_posix() for path in destination.rglob("*") if path.is_file()}
        if actual_paths != expected_paths:
            raise RuntimeError("Existing candidate contents differ from declared payload")
        for path in destination.rglob("*"):
            if path.is_symlink():
                raise RuntimeError(f"Linked package path rejected: {path.relative_to(destination)}")
        for relative in sorted(expected_paths):
            target = destination / relative
            if relative == "runtime/tools.wasm" and target.read_bytes() != module:
                raise RuntimeError("Frozen candidate WASM differs from selected build module")
            if relative != "catalog.json" and relative != "runtime/tools.wasm" and target.read_bytes() != safe_path(PAYLOAD[relative]).read_bytes():
                raise RuntimeError(f"Frozen candidate source drifted: {relative}")
            files.append({"path": relative, "size": target.stat().st_size, "sha256": sha_file(target)})
        packaged_catalog = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
        source_entry = packaged_catalog.get("source", {})
        if source_entry != {"revision": revision, "dirty": dirty}:
            raise RuntimeError("Frozen candidate catalog source revision/dirty state does not match current source")
        original_catalog = json.loads(PAYLOAD["catalog.json"].read_text(encoding="utf-8"))
        packaged_catalog.pop("source", None)
        original_catalog.pop("source", None)
        if packaged_catalog != original_catalog:
            raise RuntimeError("Frozen candidate catalog differs from current package source")
        files.sort(key=lambda item: item["path"])
        index_bytes = json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode("utf-8")

        proof = {
            "schemaVersion": 1,
            "kind": "unsigned-provider-candidate",
            "packageId": manifest["id"],
            "packageVersion": manifest["version"],
            "candidateDirectory": destination.relative_to(ROOT).as_posix(),
            "sourceRevision": revision,
            "sourceWorkingTreeDirty": dirty,
            "platform": {"os": platform.system(), "release": platform.release(), "architecture": platform.machine()},
            "contract": {"version": "1.0.0-preview.1", "profile": "resource.wasi.image.resize.v1", "manifestSchemaVersion": 2},
            "profiles": PROFILES,
            "permissions": ["resource.read"],
            "sourceTool": {"id": source_tool["id"], "version": source_tool["tool_version"], "status": source_tool["status"], "catalogPath": "docs/tools.json", "catalogSha256": sha_file(ROOT / "docs/tools.json"), "scope": source_tool["scope"]},
            "scope": {"included": ["one user-authorized PNG/JPEG/WebP image resource", "resize to a bounded maximum width", "encode one PNG/JPEG/WebP derived resource"], "excluded": ["batch operations", "crop or annotate", "EXIF cleanup", "OCR", "persistent library", "filesystem paths", "network", "automatic save/overwrite"]},
            "signature": {"signed": False, "publisherKeyAvailable": False, "format": "unsigned directory; no .zicode-plugin container"},
            "acceptance": {"providerCandidate": True, "studioReceiptVerified": False, "nativeStudioEndToEnd": "pending"},
            "wasm": {"path": "runtime/tools.wasm", "size": len(module), "sha256": module_sha, "memoryCeilingBytes": evidence["wasm"]["moduleMemoryCeilingBytes"]},
            "studioWorker": {"path": str(worker), "sha256": worker_sha},
            "functionalEvidence": {"path": results.relative_to(ROOT).as_posix(), "sha256": sha_file(results), "profile": evidence["studioResourceProfile"]["profile"], "fixtureNames": [row["name"] for row in evidence["fixtures"]], "formatMatrixCases": len(evidence["formatMatrix"]), "tests": [{"name": "Studio Worker resize fixtures (4 MP, near 16 MP, transparent PNG)", "status": "passed"}, {"name": "Studio Worker PNG/JPEG/WebP input-output matrix", "status": "passed"}, {"name": "Studio Worker rejects over-profile image dimensions", "status": "passed"}, {"name": "Studio Worker rejects damaged image without output", "status": "passed"}, {"name": "Studio Worker rejects invalid width parameters without output", "status": "passed"}, {"name": "Studio Worker rejects MIME/file-format mismatch without output", "status": "passed"}, {"name": "Studio Worker rejects output above 32 MiB", "status": "passed"}]},
            "files": files,
            "directoryIndexSha256": sha256(index_bytes),
            "candidateTreeSha256": sha256(index_bytes),
            "limitations": ["No signing key was used; this directory is not an installable signed .zicode-plugin package.", "Studio worker-mode execution verifies module/framing behavior, not package installation, user authorization, resource registration, preview, transfer, save, cancellation, leases or concurrency acceptance.", "Current Host image-output registration requires exactly one output resource before validating the result; zero-resource adapter failures therefore map to invalid_output and do not preserve adapter error codes. Host-level negative-result handling remains an acceptance blocker.", "WASI/native resizing has a bounded one-channel delta in two WebP-input fixtures; see full semantic comparisons in the benchmark receipt."],
        }
        proof_path.write_text(json.dumps(proof, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="")
    except Exception:
        if not args.refresh_proof:
            shutil.rmtree(destination, ignore_errors=True)
        raise

    print(f"Unsigned candidate directory: {destination}")
    print(f"Payload: {len(files)} files, {sum(item['size'] for item in files):,} bytes")
    print(f"Candidate tree SHA-256: {proof['candidateTreeSha256']}")
    print(f"WASM SHA-256: {module_sha}")
    print(f"Studio Worker SHA-256: {worker_sha}")
    print(f"External verification: {proof_path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"Image resize candidate not created: {error}", file=sys.stderr)
        raise SystemExit(1)
