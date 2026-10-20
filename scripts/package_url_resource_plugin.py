#!/usr/bin/env python3
"""Freeze the standalone URL text-resource WASI provider candidate."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from package_studio_catalog_revision import ENCODING, verify_package
from tool_directory import validate_directory, directory_snapshot

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "release/plugin-candidates/url-text-resource-0.1.1-candidate-01"
BASELINE = ROOT / "release/plugin-candidates/url-text-resource-0.1.0-candidate-02"
WASM = BASELINE / "runtime/tools.wasm"
RUNTIME_PROOF = ROOT / "target/plugin-url-resource-011/verification.json"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def encoded(value: object) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def inside(path: Path, base: Path) -> Path:
    cursor = path.absolute()
    while cursor != base.absolute():
        if cursor.is_symlink() or cursor.is_junction():
            raise SystemExit(f"Linked output path rejected: {cursor}")
        if cursor == cursor.parent:
            raise SystemExit(f"Output escapes candidate directory: {path}")
        cursor = cursor.parent
    resolved = path.resolve(strict=False)
    if not resolved.is_relative_to(base.resolve(strict=True)):
        raise SystemExit(f"Output escapes candidate directory: {path}")
    cursor = resolved
    while cursor != base.resolve(strict=True):
        if cursor.exists() and (cursor.is_symlink() or cursor.is_junction()):
            raise SystemExit(f"Linked output path rejected: {cursor}")
        cursor = cursor.parent
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()
    output = inside(args.output.absolute(), ROOT / "release/plugin-candidates")
    if output.exists():
        raise SystemExit(f"Refusing to replace an existing candidate: {output}")
    receipt_path = output.parent / f"{output.name}-verification.json"
    if receipt_path.exists():
        raise SystemExit(f"Refusing to replace an existing receipt: {receipt_path}")
    if not WASM.is_file() or not RUNTIME_PROOF.is_file():
        raise SystemExit("Build the WASI module and run scripts/verify_url_resource_plugin.py first")
    wasm = WASM.read_bytes()
    runtime_proof = json.loads(RUNTIME_PROOF.read_text(encoding="utf-8"))
    if runtime_proof["wasmSha256"] != sha256(wasm):
        raise SystemExit("Runtime evidence does not match the current WASI module")

    manifest = json.loads((ROOT / "plugins/url-text-resource/plugin.json").read_text(encoding="utf-8"))
    catalog = json.loads((ROOT / "plugins/url-text-resource/catalog.json").read_text(encoding="utf-8"))
    if manifest["requiredProfiles"] != {"resource.input.types.v1": "=1.0.0-preview.1", "output.schema.v1": "=1.0.0-rc.1", "tool.directory.v1": "=1.0.0-preview.1"}:
        raise SystemExit("Resource profile pin mismatch")
    if manifest["runtime"].get("resourceInputProfile") != "resource.wasi.input.draft2":
        raise SystemExit("Resource input profile mismatch")
    capability = manifest["contributes"]["capabilities"][0]
    if capability["resourceInputMimeTypes"] != ["text/plain"]:
        raise SystemExit("Capability must accept exactly text/plain")
    if catalog["tools"][0]["toolId"] != "devtools.transforms.url.encode" or catalog["tools"][0]["version"] != "1.0.2":
        raise SystemExit("Candidate is not pinned to the current URL-encode tool")
    git = lambda *argv: subprocess.check_output(["git", *argv], cwd=ROOT).decode().strip()
    source = {"revision": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain"))}
    catalog["source"] = source

    payload: dict[str, bytes] = {}
    def add(name: str, data: bytes) -> None:
        if name in payload or Path(name).is_absolute() or ".." in Path(name).parts:
            raise SystemExit(f"Invalid or duplicate package path: {name}")
        payload[name] = data

    for name in ["README.md", "plugin.json", "catalog.json"]:
        path = ROOT / "plugins/url-text-resource" / name
        add(name, encoded(catalog) if name == "catalog.json" else path.read_bytes())
    add("plugin.json", (ROOT / "plugins/url-text-resource/plugin.json").read_bytes()) if "plugin.json" not in payload else None
    add("schemas/input.schema.json", (ROOT / "plugins/url-text-resource/schemas/input.schema.json").read_bytes())
    add("schemas/output.schema.json", (ROOT / "plugins/url-text-resource/schemas/output.schema.json").read_bytes())
    for name in ["success-vectors.json", "runtime-negative-vectors.json", "host-negative-cases.json"]:
        add(f"fixtures/{name}", (ROOT / "plugins/url-text-resource/fixtures" / name).read_bytes())
    add("runtime/tools.wasm", wasm)
    add("verification/runtime-conformance.json", encoded(runtime_proof))
    add("LICENSE", (ROOT / "LICENSE").read_bytes())


    baseline_receipt = BASELINE.with_name(BASELINE.name + "-verification.json")
    verified_baseline = verify_package(BASELINE.relative_to(ROOT).as_posix(), baseline_receipt.relative_to(ROOT).as_posix())
    original = json.loads((BASELINE / "plugin.json").read_bytes())
    for key in ("id", "runtime", "permissions", "activationEvents"):
        if manifest[key] != original[key]:
            raise SystemExit(f"Runtime reuse forbids changing {key}")
    original_capability = original["contributes"]["capabilities"][0]
    for key in ("id", "inputSchema", "resourceInputMimeTypes", "maxResultBytes"):
        if capability[key] != original_capability[key]:
            raise SystemExit(f"Runtime reuse forbids changing capability {key}")
    if capability["outputSchema"]["oneOf"][0] != original_capability["outputSchema"]:
        raise SystemExit("Existing success schema must remain identical")
    validate_directory(capability["directory"])
    if runtime_proof.get("outputSchemaSha256") != sha256((ROOT / "plugins/url-text-resource/schemas/output.schema.json").read_bytes()):
        raise SystemExit("Runtime proof does not cover current output schema")
    if len(runtime_proof.get("cases", [])) != 13 or not all(row.get("outputSchemaValid") for row in runtime_proof["cases"]):
        raise SystemExit("Expected all 13 resource/output cases")
    for row in json.loads(baseline_receipt.read_bytes())["files"]:
        if row["path"].startswith("licenses/") or row["path"] == "NOTICE":
            add(row["path"], (BASELINE / row["path"]).read_bytes())
    add("verification/baseline-receipt.json", baseline_receipt.read_bytes())
    add("verification/tool-directory-snapshot.json", encoded(directory_snapshot(manifest)))
    add("verification/verify_url_resource_plugin.py", (ROOT / "scripts/verify_url_resource_plugin.py").read_bytes())

    files = [
        {"path": name, "size": len(data), "sha256": sha256(data)}
        for name, data in sorted(payload.items())
    ]
    receipt = {
        "schemaVersion": 1,
        "packageId": manifest["id"],
        "packageVersion": manifest["version"],
        "source": source,
        "deliveryStatus": "unsigned_provider_candidate_not_host_accepted",
        "runtimeReusedFrom": verified_baseline,
        "files": files,
        "directoryIndexEncoding": ENCODING,
        "directoryIndexSha256": sha256(json.dumps(files, separators=(",", ":")).encode()),
        "wasmSha256": sha256(wasm),
        "unverified": [
            "Studio installation, discovery, UI resource transfer and user-visible result",
            "Studio Host cancellation, permission-revocation and stale-transfer-grant rejection",
            "Production Worker fuel/deadline enforcement and macOS/Linux Host behavior",
            "Signing and installer/update/rollback flows",
        ],
    }
    # All boundary and hash checks precede writing the new versioned directory.
    output.mkdir(parents=True, exist_ok=False)
    for name, data in payload.items():
        path = output / name
        inside(path, output)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    receipt_path.write_bytes(encoded(receipt))
    for row in files:
        if sha256((output / row["path"]).read_bytes()) != row["sha256"]:
            raise SystemExit(f"Post-write hash mismatch: {row['path']}")
    print(f"Unsigned URL resource candidate: {output}; {len(files)} files; {sum(row['size'] for row in files)} bytes")
    print(f"Receipt: {receipt_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
